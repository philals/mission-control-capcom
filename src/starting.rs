//! "Starting" marks: a session that was just asked for, before it has recorded itself on its task, so a
//! second click in that gap opens or reports the first instead of starting another.
use crate::host::Host;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::time::Duration;

pub const KEPT: Duration = Duration::from_secs(300);

pub struct Starting {
    path: PathBuf,
}

impl Starting {
    pub fn new(path: PathBuf) -> Starting {
        Starting { path }
    }

    /// `starting.json` in capcom's cache folder.
    pub fn default() -> Option<Starting> {
        crate::cache::Cache::default_dir().map(|d| Starting::new(d.join("starting.json")))
    }

    fn read(&self) -> Map<String, Value> {
        std::fs::read(&self.path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
    }

    /// Holds `<file>.lock` while a read-modify-write runs, so the TUI and capcom web (and a TUI's threads)
    /// never both take the same mark or drop each other's. Without a lock file it goes ahead unlocked.
    fn locked<T>(&self, f: impl FnOnce() -> T) -> T {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(self.path.with_extension("json.lock"));
        let held = lock.as_ref().ok().filter(|f| f.lock().is_ok());
        let out = f();
        if let Some(f) = held {
            let _ = f.unlock();
        }
        out
    }

    fn write(&self, marks: &Map<String, Value>) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = self.path.with_extension(format!("json.{}.{n}.tmp", std::process::id()));
        if std::fs::write(&tmp, Value::Object(marks.clone()).to_string()).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }

    fn id(key: &str, task: &str) -> String {
        format!("{key}/{task}")
    }

    /// Mark `key`/`task` as starting in `host`; false (and nothing written) when a live mark exists.
    pub fn mark(&self, key: &str, task: &str, host: Host, now: u64) -> bool {
        self.locked(|| {
            if self.live(key, task, now).is_some() {
                return false;
            }
            let mut marks = self.read();
            marks.retain(|_, m| m["at"].as_u64().is_some_and(|at| now.saturating_sub(at) <= KEPT.as_secs()));
            marks.insert(Self::id(key, task), json!({ "host": host.word(), "at": now }));
            self.write(&marks);
            true
        })
    }

    pub fn clear(&self, key: &str, task: &str) {
        self.locked(|| {
            let mut marks = self.read();
            if marks.remove(&Self::id(key, task)).is_some() {
                self.write(&marks);
            }
        })
    }

    pub fn live(&self, key: &str, task: &str, now: u64) -> Option<Host> {
        let marks = self.read();
        let m = marks.get(&Self::id(key, task))?;
        let at = m["at"].as_u64()?;
        (now.saturating_sub(at) <= KEPT.as_secs()).then(|| Host::parse(m["host"].as_str()?)).flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks() -> (tempfile::TempDir, Starting) {
        let dir = tempfile::tempdir().unwrap();
        let s = Starting::new(dir.path().join("starting.json"));
        (dir, s)
    }

    #[test]
    fn a_second_mark_within_five_minutes_is_refused() {
        let (_d, s) = marks();
        assert!(s.mark("K-1", "T2", Host::Orca, 1000));
        assert!(!s.mark("K-1", "T2", Host::Desktop, 1000 + 299));
        assert_eq!(s.live("K-1", "T2", 1000 + 299), Some(Host::Orca), "the refused mark wrote nothing");
    }

    #[test]
    fn a_mark_older_than_five_minutes_is_gone() {
        let (_d, s) = marks();
        assert!(s.mark("K-1", "T2", Host::Orca, 1000));
        assert_eq!(s.live("K-1", "T2", 1000 + 301), None);
        assert!(s.mark("K-1", "T2", Host::Herdr, 1000 + 301));
    }

    #[test]
    fn clear_removes_only_that_task() {
        let (_d, s) = marks();
        assert!(s.mark("K-1", "T2", Host::Orca, 1000));
        assert!(s.mark("K-1", "T3", Host::Orca, 1000));
        s.clear("K-1", "T2");
        assert_eq!(s.live("K-1", "T2", 1001), None);
        assert_eq!(s.live("K-1", "T3", 1001), Some(Host::Orca));
    }

    #[test]
    fn a_missing_or_broken_file_is_no_marks() {
        let (d, s) = marks();
        assert_eq!(s.live("K-1", "T2", 1000), None);
        std::fs::write(d.path().join("starting.json"), "not json").unwrap();
        assert_eq!(s.live("K-1", "T2", 1000), None);
        assert!(s.mark("K-1", "T2", Host::Desktop, 1000));
        assert_eq!(s.live("K-1", "T2", 1000), Some(Host::Desktop));
    }

    #[test]
    fn marks_made_at_the_same_moment_let_exactly_one_through() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("starting.json");
        let threads: Vec<_> = (0..16)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let s = Starting::new(path);
                    (s.mark("K-1", "T2", Host::Orca, 1000), s.mark("K-1", &format!("T{}", 10 + i), Host::Orca, 1000))
                })
            })
            .collect();
        let results: Vec<(bool, bool)> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|(same, _)| *same).count(), 1, "{results:?}");
        assert!(results.iter().all(|(_, own)| *own), "each thread's own task is marked");
        let s = Starting::new(path);
        assert!((0..16).all(|i| s.live("K-1", &format!("T{}", 10 + i), 1000).is_some()), "no mark was lost");
    }

    #[test]
    fn a_missing_folder_is_created() {
        let d = tempfile::tempdir().unwrap();
        let s = Starting::new(d.path().join("cache/starting.json"));
        assert!(s.mark("K-1", "T2", Host::Orca, 1000));
        assert_eq!(s.live("K-1", "T2", 1000), Some(Host::Orca));
    }
}
