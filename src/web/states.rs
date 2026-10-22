//! The real state (draft, ready, merged, closed) of the PRs recorded on tasks, which GitHub's list of your
//! open PRs cannot vouch for once they merge. It lets a task say "all PRs merged" and, with auto-sync on,
//! writes what it finds to the board so a merged PR finishes its task.
use super::engine::lock;
use super::*;
use crate::refresh;

/// How often the recorded PRs are looked up while a page is looking.
const EVERY: Duration = Duration::from_secs(60);

impl Server {
    /// The recorded PRs of every unfinished task that the open list does not already cover.
    pub(super) fn recorded_urls(&self) -> Vec<String> {
        let open = lock(&self.prs).items.clone();
        let mut urls = Vec::new();
        for key in store::list_keys(&self.root).unwrap_or_default() {
            if let Ok(board) = store::load(&self.root, &key) {
                urls.extend(refresh::pending_urls(&board));
            }
        }
        urls.sort();
        urls.dedup();
        urls.retain(|u| !open.iter().any(|p| same_url(&p.url, u)));
        urls
    }

    /// Look the recorded PRs up now (or when `check_now` was asked for) and keep what they say.
    pub(super) fn check_recorded(&self, cache: Option<&crate::cache::Cache>, force: bool) {
        let urls = self.recorded_urls();
        if urls.is_empty() {
            return;
        }
        let results = crate::pr_state::look_up(&urls, self.tools().lookup.as_ref(), cache, force);
        {
            let mut known = lock(&self.eng.known);
            for (url, state) in &results {
                if let Ok(state) = state {
                    known.insert(url.clone(), *state);
                }
            }
        }
        if self.settings().auto_sync {
            self.write_states(&results);
        }
    }

    /// Record what the lookups found on every board that it changes.
    pub(super) fn write_states(&self, results: &refresh::Lookups) {
        for key in store::list_keys(&self.root).unwrap_or_default() {
            let Ok(board) = store::load(&self.root, &key) else {
                continue;
            };
            let mut trial = board;
            let changes: Vec<String> = refresh::apply(&mut trial, results).into_iter().filter(|m| m.contains("->")).collect();
            if changes.is_empty() {
                continue;
            }
            if store::update(&self.root, &key, |b| Ok(refresh::apply(b, results))).is_ok() {
                self.event(format!("auto-sync {key}: {}", changes.join("; ")));
            }
        }
    }

    /// Check the recorded PRs every minute while a page looks (and once at the start), and when asked.
    pub fn watch_pr_states(self: &Arc<Self>, tick: Duration) {
        let me = self.clone();
        std::thread::spawn(move || {
            let cache = crate::cache::Cache::default_dir().and_then(crate::cache::Cache::open);
            let mut last: Option<std::time::Instant> = None;
            loop {
                let asked = me.eng.check_now.swap(false, std::sync::atomic::Ordering::SeqCst);
                let due = last.is_none_or(|t| t.elapsed() >= me.attention.wait(EVERY));
                if asked || (due && (last.is_none() || me.looking())) {
                    last = Some(std::time::Instant::now());
                    me.check_recorded(cache.as_ref(), asked);
                }
                std::thread::sleep(tick);
            }
        });
    }
}
