//! What capcom has already asked an agent to do about each PR, kept in a small file in the shared
//! cache folder so a restart, or a second copy of the TUI, does not ask twice. Every change is made
//! under a file lock.
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::PathBuf;
use std::time::{Duration as Wait, Instant};

const KEEP_DAYS: i64 = 7;
const LOCK_WAIT: Wait = Wait::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// What was wrong when the agent was last asked: head commit, failing checks, open threads.
    pub fingerprint: String,
    pub attempts: u32,
    pub at: String,
    pub gave_up: bool,
    pub stall_told: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    fixes: BTreeMap<String, Entry>,
    copilot_asked: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// Ask the agent: this is round `round`. `undo` puts the record back if the ask fails.
    Go { round: u32, undo: Option<Entry> },
    /// The PR looks just as it did when the agent was last asked.
    Same,
    /// Out of rounds (reported once, then quiet).
    GaveUp,
    /// The state file could not be locked in time: try again on the next look.
    Busy,
}

#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Store {
        Store { dir }
    }

    fn with<R>(&self, change: impl FnOnce(&mut State) -> R) -> Option<R> {
        std::fs::create_dir_all(&self.dir).ok()?;
        let lock = OpenOptions::new().create(true).write(true).truncate(false).open(self.dir.join("autofix.lock")).ok()?;
        let start = Instant::now();
        loop {
            match File::try_lock(&lock) {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if start.elapsed() < LOCK_WAIT => std::thread::sleep(Wait::from_millis(25)),
                Err(_) => return None,
            }
        }
        let path = self.dir.join("autofix.json");
        let mut state: State = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let result = change(&mut state);
        let cutoff = Utc::now() - Duration::days(KEEP_DAYS);
        state.fixes.retain(|_, e| DateTime::parse_from_rfc3339(&e.at).map_or(true, |t| t > cutoff));
        state.copilot_asked.truncate(500);
        let tmp = path.with_extension("tmp");
        let saved = serde_json::to_vec(&state).ok().is_some_and(|b| std::fs::write(&tmp, b).is_ok() && std::fs::rename(&tmp, &path).is_ok());
        let _ = File::unlock(&lock);
        saved.then_some(result)
    }

    /// Decide whether to ask the agent about this PR now, and record the ask if so. `manual` is you
    /// pressing the button: it always goes, and starts the count again.
    pub fn claim(&self, url: &str, fingerprint: &str, max: u32, manual: bool) -> Claim {
        self.with(|state| {
            let undo = state.fixes.get(url).cloned();
            let (attempts, gave_up) = match &undo {
                Some(e) if !manual => {
                    if e.fingerprint == fingerprint {
                        return Claim::Same;
                    }
                    if e.attempts >= max {
                        if e.gave_up {
                            return Claim::Same;
                        }
                        let mut told = e.clone();
                        told.gave_up = true;
                        state.fixes.insert(url.to_string(), told);
                        return Claim::GaveUp;
                    }
                    (e.attempts + 1, false)
                }
                _ => (1, false),
            };
            let entry = Entry { fingerprint: fingerprint.to_string(), attempts, at: Utc::now().to_rfc3339(), gave_up, stall_told: false };
            state.fixes.insert(url.to_string(), entry);
            Claim::Go { round: attempts, undo }
        })
        .unwrap_or(Claim::Busy)
    }

    /// Put a record back after an ask that did not happen (the agent was busy, or it failed).
    pub fn undo(&self, url: &str, previous: Option<Entry>) {
        self.with(|state| match previous {
            Some(entry) => {
                state.fixes.insert(url.to_string(), entry);
            }
            None => {
                state.fixes.remove(url);
            }
        });
    }

    /// The PR needs nothing any more: forget its rounds.
    pub fn clear(&self, url: &str) {
        self.with(|state| state.fixes.remove(url));
    }

    /// True once when the agent was asked `minutes` ago and the PR still looks exactly the same.
    pub fn stalled(&self, url: &str, fingerprint: &str, minutes: i64) -> bool {
        self.with(|state| {
            let Some(entry) = state.fixes.get_mut(url) else {
                return false;
            };
            let old = DateTime::parse_from_rfc3339(&entry.at).is_ok_and(|t| Utc::now() - t.with_timezone(&Utc) > Duration::minutes(minutes));
            if entry.fingerprint == fingerprint && old && !entry.stall_told {
                entry.stall_told = true;
                return true;
            }
            false
        })
        .unwrap_or(false)
    }

    /// True the first time this PR is seen: the caller should ask Copilot to review it.
    pub fn claim_copilot(&self, url: &str) -> bool {
        self.with(|state| {
            if state.copilot_asked.iter().any(|u| u == url) {
                return false;
            }
            state.copilot_asked.insert(0, url.to_string());
            true
        })
        .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://github.com/acme/api/pull/7";

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        (dir, store)
    }

    fn round(claim: Claim) -> u32 {
        match claim {
            Claim::Go { round, .. } => round,
            other => panic!("expected a go, got {other:?}"),
        }
    }

    #[test]
    fn the_agent_is_asked_once_per_look_of_the_pr_and_again_only_when_the_pr_changed() {
        let (_dir, store) = store();
        assert_eq!(round(store.claim(URL, "sha1|test|", 5, false)), 1);
        assert_eq!(store.claim(URL, "sha1|test|", 5, false), Claim::Same, "nothing changed since the ask");
        assert_eq!(round(store.claim(URL, "sha2|test|", 5, false)), 2, "a new commit that is still red");
        assert_eq!(round(store.claim(URL, "sha2|test|t1", 5, false)), 3, "or fewer or other open threads");
    }

    #[test]
    fn it_stops_after_the_last_round_and_says_so_only_once() {
        let (_dir, store) = store();
        for i in 1..=3 {
            assert_eq!(round(store.claim(URL, &format!("sha{i}"), 3, false)), i);
        }
        assert_eq!(store.claim(URL, "sha4", 3, false), Claim::GaveUp);
        assert_eq!(store.claim(URL, "sha5", 3, false), Claim::Same, "quiet after that");
        assert_eq!(round(store.claim(URL, "sha6", 3, true)), 1, "your button always goes and starts the count again");
    }

    #[test]
    fn a_failed_ask_can_be_undone_so_the_next_look_tries_again() {
        let (_dir, store) = store();
        let Claim::Go { undo, .. } = store.claim(URL, "sha1", 5, false) else { panic!() };
        store.undo(URL, undo);
        assert_eq!(round(store.claim(URL, "sha1", 5, false)), 1);
        let Claim::Go { undo, .. } = store.claim(URL, "sha2", 5, false) else { panic!() };
        store.undo(URL, undo);
        assert_eq!(store.claim(URL, "sha1", 5, false), Claim::Same, "the earlier record is back");
    }

    #[test]
    fn a_pr_that_needs_nothing_starts_from_zero_next_time() {
        let (_dir, store) = store();
        store.claim(URL, "sha1", 5, false);
        store.clear(URL);
        assert_eq!(round(store.claim(URL, "sha1", 5, false)), 1);
    }

    #[test]
    fn two_stores_on_the_same_folder_do_not_both_ask() {
        let (dir, a) = store();
        let b = Store::new(dir.path().to_path_buf());
        assert_eq!(round(a.claim(URL, "sha1", 5, false)), 1);
        assert_eq!(b.claim(URL, "sha1", 5, false), Claim::Same, "another copy, or a restart, sees the record");
    }

    #[test]
    fn a_pr_that_does_not_change_after_an_ask_is_reported_stalled_once() {
        let (_dir, store) = store();
        store.claim(URL, "sha1", 5, false);
        assert!(!store.stalled(URL, "sha1", 10), "asked just now");
        assert!(store.stalled(URL, "sha1", -1), "asked more than -1 minutes ago and still the same");
        assert!(!store.stalled(URL, "sha1", -1), "only once");
        store.claim(URL, "sha2", 5, false);
        assert!(!store.stalled(URL, "sha3", -1), "a changed PR is not stalled");
    }

    #[test]
    fn copilot_is_requested_once_per_pr_ever() {
        let (_dir, store) = store();
        assert!(store.claim_copilot(URL));
        assert!(!store.claim_copilot(URL));
        assert!(store.claim_copilot("https://github.com/acme/api/pull/8"));
    }
}
