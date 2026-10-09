//! Looks up the real state (draft, ready, merged, closed) of the PRs recorded on a story's tasks,
//! in the background, so the board can tell you a task is finished and, if you choose, update itself.
use crate::cache::Cache;
use capcom::model::PrState;
use capcom::refresh::{Lookups, PrLookup};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

pub enum Msg {
    /// The PRs to watch from now on.
    Urls(Vec<String>),
    /// Look them all up again right now.
    Refresh,
}

fn max_age(state: &PrState) -> Duration {
    match state {
        PrState::Merged | PrState::Closed => Duration::from_secs(3600),
        _ => Duration::from_secs(60),
    }
}

/// Look up each url, sharing results with other copies through the cache when there is one.
pub fn look_up(urls: &[String], lookup: &dyn PrLookup, cache: Option<&Cache>, force: bool) -> Lookups {
    urls.iter()
        .map(|url| {
            let fetch = || lookup.state(url);
            let state = match cache {
                Some(cache) => {
                    let name = format!("pr-{}", crate::cache::key_of(&[url]));
                    cache.shared(&name, chrono::Utc::now, max_age, force, fetch)
                }
                None => fetch(),
            };
            (url.clone(), state.map_err(|e| format!("{e:#}")))
        })
        .collect()
}

/// A background thread that rechecks the watched PRs every `every`, and on request.
pub fn spawn(
    lookup: Arc<dyn PrLookup + Send + Sync>,
    cache: Option<Cache>,
    every: Duration,
) -> (Sender<Msg>, Receiver<Lookups>) {
    let (tx, rx) = mpsc::channel::<Msg>();
    let (out_tx, out_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut urls: Vec<String> = Vec::new();
        loop {
            let mut force = false;
            match rx.recv_timeout(every) {
                Ok(Msg::Urls(next)) => urls = next,
                Ok(Msg::Refresh) => force = true,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            while let Ok(extra) = rx.try_recv() {
                match extra {
                    Msg::Urls(next) => urls = next,
                    Msg::Refresh => force = true,
                }
            }
            if urls.is_empty() {
                continue;
            }
            let results = look_up(&urls, lookup.as_ref(), cache.as_ref(), force);
            if out_tx.send(results).is_err() {
                break;
            }
        }
    });
    (tx, out_rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    struct Fake {
        calls: AtomicUsize,
        state: PrState,
    }

    impl PrLookup for Fake {
        fn state(&self, url: &str) -> Result<PrState> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if url.ends_with("/bad") {
                anyhow::bail!("not found");
            }
            Ok(self.state)
        }
    }

    fn fake(state: PrState) -> Fake {
        Fake { calls: AtomicUsize::new(0), state }
    }

    fn urls(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| format!("https://github.com/o/r/pull/{n}")).collect()
    }

    #[test]
    fn every_url_gets_its_state_or_its_error() {
        let found = look_up(&urls(&["1", "bad"]), &fake(PrState::Merged), None, false);
        assert_eq!(found[&urls(&["1"])[0]], Ok(PrState::Merged));
        assert_eq!(found[&urls(&["bad"])[0]], Err("not found".to_string()));
    }

    #[test]
    fn the_cache_spares_github_a_second_look_and_a_refresh_skips_it() {
        let dir = TempDir::new().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        let f = fake(PrState::Ready);
        look_up(&urls(&["1"]), &f, Some(&cache), false);
        look_up(&urls(&["1"]), &f, Some(&cache), false);
        assert_eq!(f.calls.load(Ordering::SeqCst), 1, "the second look reuses the cached state");
        std::thread::sleep(Duration::from_millis(3100));
        look_up(&urls(&["1"]), &f, Some(&cache), true);
        assert_eq!(f.calls.load(Ordering::SeqCst), 2, "a manual refresh looks again");
    }

    #[test]
    fn errors_are_not_cached() {
        let dir = TempDir::new().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        let f = fake(PrState::Ready);
        look_up(&urls(&["bad"]), &f, Some(&cache), false);
        look_up(&urls(&["bad"]), &f, Some(&cache), false);
        assert_eq!(f.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_thread_looks_up_what_it_is_told_to_watch_and_again_on_refresh() {
        let f = Arc::new(fake(PrState::Merged));
        let (tx, rx) = spawn(f.clone(), None, Duration::from_secs(3600));
        tx.send(Msg::Urls(urls(&["1", "2"]))).unwrap();
        let first = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(first.len(), 2);
        tx.send(Msg::Refresh).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(f.calls.load(Ordering::SeqCst), 4);
        tx.send(Msg::Urls(vec![])).unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err(), "nothing to watch, nothing looked up");
    }

    #[test]
    fn the_thread_rechecks_on_its_own_timer() {
        let f = Arc::new(fake(PrState::Ready));
        let (tx, rx) = spawn(f.clone(), None, Duration::from_millis(50));
        tx.send(Msg::Urls(urls(&["1"]))).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(f.calls.load(Ordering::SeqCst) >= 2);
    }
}
