//! A small on-disk cache shared by every `capcom-tui` that is open, so a second copy (or the
//! Herdr plugin plus a terminal) reuses the first one's GitHub results instead of polling again.
//! No daemon: whichever copy finds the data stale takes a lock, fetches and writes it.
use anyhow::{Context, Result};
use chrono::{DateTime, Duration as Age, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A forced refresh that finds data newer than this reuses it (two quick clicks make one call).
const FORCE_REUSE_SECONDS: i64 = 3;
/// How long to wait for another copy that is fetching right now.
const WAIT_FOR_OTHER: Duration = Duration::from_secs(75);
const PRUNE_AFTER_HOURS: i64 = 24;

#[derive(Serialize, Deserialize)]
struct Stored<T> {
    fetched_at: DateTime<Utc>,
    value: T,
}

#[derive(Clone, Debug)]
pub struct Cache {
    dir: PathBuf,
}

/// Stable across runs and builds, unlike the standard hasher.
pub fn key_of(parts: &[&str]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for part in parts {
        for byte in part.bytes().chain(std::iter::once(0)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    format!("{hash:016x}")
}

pub fn dir_from(override_dir: Option<&str>, xdg: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let non_empty = |v: Option<&str>| v.filter(|s| !s.is_empty()).map(str::to_string);
    if let Some(dir) = non_empty(override_dir) {
        return Some(PathBuf::from(dir));
    }
    if let Some(xdg) = non_empty(xdg) {
        return Some(Path::new(&xdg).join("capcom"));
    }
    non_empty(home).map(|home| Path::new(&home).join(".cache/capcom"))
}

fn create_private(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

impl Cache {
    pub fn default_dir() -> Option<PathBuf> {
        let var = |name: &str| std::env::var(name).ok();
        dir_from(var("CAPCOM_CACHE_DIR").as_deref(), var("XDG_CACHE_HOME").as_deref(), var("HOME").as_deref())
    }

    /// Open (and create) the cache folder; old files are tidied away. None when it cannot be used.
    pub fn open(dir: PathBuf) -> Option<Cache> {
        std::fs::create_dir_all(&dir).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        let cache = Cache { dir };
        cache.prune();
        Some(cache)
    }

    fn prune(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(PRUNE_AFTER_HOURS as u64 * 3600));
            if old {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    fn data_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }

    fn read<T: DeserializeOwned>(&self, name: &str) -> Option<Stored<T>> {
        let text = std::fs::read_to_string(self.data_path(name)).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn write<T: Serialize>(&self, name: &str, stored: &Stored<T>) {
        let path = self.data_path(name);
        let tmp = self.dir.join(format!("{name}.{}.tmp", std::process::id()));
        let result = serde_json::to_vec(stored).map_err(anyhow::Error::from).and_then(|bytes| {
            let mut file = create_private(&tmp).context("creating cache file")?;
            file.write_all(&bytes)?;
            std::fs::rename(&tmp, &path).context("replacing cache file")
        });
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    fn lock_file(&self, name: &str) -> Option<File> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        options.open(self.dir.join(format!("{name}.lock"))).ok()
    }

    /// The value for `name`, fetching only when the cached one is stale. `max_age` says how old
    /// is too old for a value (it can depend on the value: busy data goes stale sooner). `force`
    /// is a manual refresh. Errors are returned, never cached.
    pub fn shared<T: Serialize + DeserializeOwned>(
        &self,
        name: &str,
        now: impl Fn() -> DateTime<Utc>,
        max_age: impl Fn(&T) -> Duration,
        force: bool,
        fetch: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let usable = |stored: &Stored<T>, now: DateTime<Utc>| {
            let age = now - stored.fetched_at;
            let limit = if force { Age::seconds(FORCE_REUSE_SECONDS) } else { Age::from_std(max_age(&stored.value)).unwrap_or(Age::MAX) };
            age >= Age::zero() && age < limit
        };
        if let Some(stored) = self.read::<T>(name) {
            if usable(&stored, now()) {
                return Ok(stored.value);
            }
        }
        let Some(lock) = self.lock_file(name) else {
            return fetch();
        };
        let started = Instant::now();
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if started.elapsed() < WAIT_FOR_OTHER => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(TryLockError::WouldBlock) => return fetch(),
                Err(TryLockError::Error(_)) => return fetch(),
            }
        }
        if let Some(stored) = self.read::<T>(name) {
            let waited = started.elapsed() > Duration::from_millis(150);
            if usable(&stored, now()) || (waited && now() - stored.fetched_at < Age::seconds(FORCE_REUSE_SECONDS)) {
                return Ok(stored.value);
            }
        }
        let value = fetch()?;
        let stored = Stored { fetched_at: now(), value };
        self.write(name, &stored);
        Ok(stored.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    fn cache(dir: &TempDir) -> Cache {
        Cache::open(dir.path().to_path_buf()).unwrap()
    }

    fn at(clock: &Arc<Mutex<DateTime<Utc>>>) -> impl Fn() -> DateTime<Utc> + '_ {
        move || *clock.lock().unwrap()
    }

    fn clock() -> Arc<Mutex<DateTime<Utc>>> {
        Arc::new(Mutex::new("2026-10-09T01:00:00Z".parse().unwrap()))
    }

    fn secs(n: i64) -> Age {
        Age::seconds(n)
    }

    #[test]
    fn fresh_data_is_reused_and_stale_data_is_fetched_again() {
        let dir = TempDir::new().unwrap();
        let cache = cache(&dir);
        let clock = clock();
        let calls = AtomicUsize::new(0);
        let get = |force: bool| {
            cache
                .shared("x", at(&clock), |_: &u32| Duration::from_secs(30), force, || {
                    Ok(calls.fetch_add(1, Ordering::SeqCst) as u32 + 100)
                })
                .unwrap()
        };
        assert_eq!(get(false), 100);
        *clock.lock().unwrap() += secs(29);
        assert_eq!(get(false), 100, "29 seconds old is still fresh");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        *clock.lock().unwrap() += secs(2);
        assert_eq!(get(false), 101, "31 seconds old is stale");
    }

    #[test]
    fn how_old_is_too_old_can_depend_on_the_value() {
        let dir = TempDir::new().unwrap();
        let cache = cache(&dir);
        let clock = clock();
        let calls = AtomicUsize::new(0);
        let get = || {
            cache
                .shared("x", at(&clock), |busy: &bool| Duration::from_secs(if *busy { 5 } else { 30 }), false, || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(true)
                })
                .unwrap()
        };
        get();
        *clock.lock().unwrap() += secs(6);
        get();
        assert_eq!(calls.load(Ordering::SeqCst), 2, "busy data goes stale after 5 seconds");
    }

    #[test]
    fn a_manual_refresh_fetches_unless_someone_just_did() {
        let dir = TempDir::new().unwrap();
        let cache = cache(&dir);
        let clock = clock();
        let calls = AtomicUsize::new(0);
        let get = |force: bool| {
            cache
                .shared("x", at(&clock), |_: &u32| Duration::from_secs(30), force, || {
                    Ok(calls.fetch_add(1, Ordering::SeqCst) as u32)
                })
                .unwrap()
        };
        get(false);
        *clock.lock().unwrap() += secs(1);
        get(true);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "a refresh one second after a fetch reuses it");
        *clock.lock().unwrap() += secs(5);
        get(true);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn errors_are_returned_and_never_cached() {
        let dir = TempDir::new().unwrap();
        let cache = cache(&dir);
        let clock = clock();
        let failed = cache.shared::<u32>("x", at(&clock), |_| Duration::from_secs(30), false, || anyhow::bail!("boom"));
        assert_eq!(failed.unwrap_err().to_string(), "boom");
        let ok = cache.shared("x", at(&clock), |_: &u32| Duration::from_secs(30), false, || Ok(7)).unwrap();
        assert_eq!(ok, 7, "the next call fetches again");
    }

    #[test]
    fn two_copies_asking_together_make_one_fetch() {
        let dir = TempDir::new().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let path = dir.path().to_path_buf();
                let calls = calls.clone();
                std::thread::spawn(move || {
                    let cache = Cache::open(path).unwrap();
                    cache
                        .shared("x", Utc::now, |_: &u32| Duration::from_secs(30), false, || {
                            std::thread::sleep(Duration::from_millis(400));
                            Ok(calls.fetch_add(1, Ordering::SeqCst) as u32)
                        })
                        .unwrap()
                })
            })
            .collect();
        let results: Vec<u32> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "{results:?}");
        assert!(results.iter().all(|r| *r == results[0]), "everyone got the same data");
    }

    #[test]
    fn a_cache_that_cannot_be_used_just_fetches() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("not-a-folder");
        std::fs::write(&file, "x").unwrap();
        assert!(Cache::open(file).is_none());
        let cache = Cache { dir: dir.path().join("missing/deeper") };
        let value = cache.shared("x", Utc::now, |_: &u32| Duration::from_secs(30), false, || Ok(5)).unwrap();
        assert_eq!(value, 5);
    }

    #[test]
    fn keys_are_stable_and_distinguish_their_parts() {
        assert_eq!(key_of(&["a", "b"]), key_of(&["a", "b"]));
        assert_ne!(key_of(&["a", "b"]), key_of(&["ab"]));
        assert_ne!(key_of(&["a"]), key_of(&["b"]));
        assert_eq!(key_of(&["a"]).len(), 16);
    }

    #[test]
    fn the_folder_follows_the_override_then_xdg_then_home() {
        let d = |o, x, h| dir_from(o, x, h).map(|p| p.to_string_lossy().to_string());
        assert_eq!(d(Some("/o"), Some("/x"), Some("/h")).as_deref(), Some("/o"));
        assert_eq!(d(None, Some("/x"), Some("/h")).as_deref(), Some("/x/capcom"));
        assert_eq!(d(None, None, Some("/h")).as_deref(), Some("/h/.cache/capcom"));
        assert_eq!(d(Some(""), Some(""), None), None);
    }

    #[cfg(unix)]
    #[test]
    fn cache_files_are_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let cache = cache(&dir);
        cache.shared("x", Utc::now, |_: &u32| Duration::from_secs(30), false, || Ok(1)).unwrap();
        let mode = std::fs::metadata(dir.path().join("x.json")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
