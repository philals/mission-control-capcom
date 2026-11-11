//! What the service does by itself whenever the PR or run lists change: tell you what needs you, learn how
//! long checks take, and, for the PRs you ticked, the parts of auto-fix and auto-review that need no agent
//! (ask Copilot to review, bring a behind branch up to date), and the part that needs an agent: fixing red CI,
//! a conflict and Copilot's comments, by sending `/pr-address` to the agent that made the PR through the Herdr
//! app. Where that cannot be done the PR is marked instead, with a one-click session, for you to act.
use super::engine::lock;
use super::view::{autofix_on, autoreview_on};
use super::*;
use crate::autopilot::{fix_needed, FixNeed};
use crate::fixstate::{self, Claim};
use crate::runs::{Run, RunState};

const BEHIND_COOLDOWN: Duration = Duration::from_secs(15 * 60);
pub(super) const MAX_ROUNDS: u32 = 5;
/// Minutes after an ask with nothing changing on the PR before it says the agent did not act.
const STALL_MINUTES: i64 = 10;
/// How long to leave a PR alone after its agent could not be reached.
const UNREACHABLE_COOLDOWN: Duration = Duration::from_secs(10 * 60);
/// A watched run stays listed this long after it finishes.
const WATCH_KEEP_MINUTES: i64 = 30;

fn name_of(pr: &PullRequest) -> String {
    format!("{}#{}", pr.repo, pr.number)
}

impl Server {
    /// Called with every fresh PR list, before it is shown.
    pub(super) fn on_prs(self: &Arc<Self>, items: &[PullRequest]) {
        self.alert_awaiting_review(items);
        self.alert_copilot_reviews(items);
        self.run_autopilot(items);
        self.learn_durations(items);
        self.forget_gone_ticks(items);
        self.forget_gone_followed();
        self.sync_run_repos(items);
    }

    /// A followed PR that GitHub says is merged, closed or gone is let go.
    fn forget_gone_followed(&self) {
        let gone = std::mem::take(&mut lock(&self.eng.pasted).gone);
        if gone.is_empty() {
            return;
        }
        self.change_settings(|s| {
            s.pasted_prs.retain(|u| !gone.contains(u));
            s.autofix_prs.retain(|(u, _)| !gone.contains(u));
            s.autocopilot_prs.retain(|(u, _)| !gone.contains(u));
        });
        for url in gone {
            self.event(format!("{url} is merged, closed or gone: no longer followed"));
        }
    }

    /// Tell about each PR that has just turned green with no review yet. The first look only remembers.
    fn alert_awaiting_review(&self, items: &[PullRequest]) {
        let now: std::collections::HashSet<String> = items.iter().filter(|p| p.waiting_for() == Some(Waiting::Reviewer)).map(|p| p.url.clone()).collect();
        let newly: Vec<&PullRequest> = {
            let mut auto = lock(&self.eng.auto);
            let newly = match &auto.awaiting {
                Some(before) => items.iter().filter(|p| now.contains(&p.url) && !before.contains(&p.url)).collect(),
                None => Vec::new(),
            };
            auto.awaiting = Some(now);
            newly
        };
        for pr in newly {
            self.notify_if_wanted("CAPCOM: PR green, needs a reviewer", &format!("{} · {}", name_of(pr), pr.title));
        }
    }

    /// Tell when Copilot's review of one of your PRs arrives. A PR first seen already reviewed stays quiet.
    fn alert_copilot_reviews(&self, items: &[PullRequest]) {
        let now: std::collections::HashMap<String, Option<String>> = items.iter().map(|p| (p.url.clone(), p.feedback.copilot_reviewed_at.clone())).collect();
        let before = std::mem::replace(&mut lock(&self.eng.auto).copilot_seen, Some(now));
        let Some(before) = before else {
            return;
        };
        for pr in items.iter().filter(|p| p.managed()) {
            let Some(at) = &pr.feedback.copilot_reviewed_at else {
                continue;
            };
            if before.get(&pr.url).is_some_and(|was| was.as_ref() != Some(at)) {
                let open = pr.copilot_open().len();
                let what = if open == 0 { "no comments to address".to_string() } else { format!("{open} comment{} to address", if open == 1 { "" } else { "s" }) };
                self.notify_if_wanted("CAPCOM: Copilot finished reviewing", &format!("{} · {what}", name_of(pr)));
            }
        }
    }

    fn learn_durations(&self, items: &[PullRequest]) {
        let mut durations = lock(&self.eng.durations);
        if durations.learn(items) {
            if let Some(path) = lock(&self.eng.durations_path).as_ref() {
                durations.save(path);
            }
        }
    }

    /// A PR that is no longer listed (merged, closed) takes its ticks with it.
    fn forget_gone_ticks(&self, items: &[PullRequest]) {
        if items.is_empty() {
            return;
        }
        let gone = |url: &String| !items.iter().any(|p| same_url(&p.url, url));
        let s = self.settings();
        if s.autofix_prs.iter().any(|(u, _)| gone(u)) || s.autocopilot_prs.iter().any(|(u, _)| gone(u)) {
            self.change_settings(|s| {
                s.autofix_prs.retain(|(u, _)| !gone(u));
                s.autocopilot_prs.retain(|(u, _)| !gone(u));
            });
        }
    }

    /// The repos to look for manual runs in: those of your open PRs, of the PRs on the boards, and the extra ones.
    pub(super) fn deploy_repos(&self, items: &[PullRequest]) -> Vec<String> {
        let mut repos: Vec<String> = items.iter().map(|p| p.repo.clone()).collect();
        for key in store::list_keys(&self.root).unwrap_or_default() {
            if let Ok(board) = store::load(&self.root, &key) {
                repos.extend(board.tasks.iter().flat_map(|t| t.prs.iter()).filter_map(|p| prs::pr_ref(&p.url).map(|(owner, name, _)| format!("{owner}/{name}"))));
            }
        }
        repos.extend(lock(&self.eng.extra_repos).iter().cloned());
        repos.retain(|r| crate::runs::valid_repo(r));
        repos.sort();
        repos.dedup();
        repos
    }

    fn sync_run_repos(&self, items: &[PullRequest]) {
        let wanted = self.deploy_repos(items);
        let mut current = lock(&self.eng.run_repos);
        if *current != wanted {
            *current = wanted;
            drop(current);
            if let Some(wake) = lock(&self.eng.run_wake).as_ref() {
                let _ = wake.send(());
            }
        }
    }

    pub(super) fn run_autopilot(self: &Arc<Self>, items: &[PullRequest]) {
        let Some(store) = lock(&self.eng.fix_store).clone() else {
            return;
        };
        let settings = self.settings();
        let tools = self.tools();
        for pr in items.iter().filter(|p| p.managed()) {
            if autoreview_on(&settings, pr) && pr.feedback.copilot == CopilotState::None && lock(&self.eng.auto).copilot_tried.insert(pr.url.clone()) {
                let (me, store, url, name, ask) = (self.clone(), store.clone(), pr.url.clone(), name_of(pr), tools.copilot.clone());
                std::thread::spawn(move || {
                    if store.claim_copilot(&url) {
                        match ask(&url) {
                            Ok(()) => me.event(format!("asked Copilot to review {name}")),
                            Err(e) => me.event(format!("could not ask Copilot to review {name}: {e}")),
                        }
                    }
                });
            }
            if !autofix_on(&settings, pr) {
                continue;
            }
            match fix_needed(pr) {
                None => {
                    let had = {
                        let mut auto = lock(&self.eng.auto);
                        auto.fix_sent.remove(&pr.url);
                        auto.fix_failed.remove(&pr.url);
                        auto.fix_seen.remove(&pr.url).is_some()
                    };
                    if had {
                        let (store, url) = (store.clone(), pr.url.clone());
                        std::thread::spawn(move || store.clear(&url));
                    }
                }
                Some(need) => self.note_fix_need(pr, need, &store),
            }
        }
    }

    fn note_fix_need(self: &Arc<Self>, pr: &PullRequest, need: FixNeed, store: &fixstate::Store) {
        {
            let mut auto = lock(&self.eng.auto);
            if auto.fix_seen.get(&pr.url) == Some(&need.fingerprint) {
                drop(auto);
                self.check_stalled(pr, &need.fingerprint, store);
                return;
            }
            if need.only_behind && auto.behind_asked.get(&pr.url).is_some_and(|at| at.elapsed() < BEHIND_COOLDOWN) {
                return;
            }
            if auto.fix_failed.get(&pr.url).is_some_and(|at| at.elapsed() < UNREACHABLE_COOLDOWN) {
                return;
            }
            if need.only_behind {
                auto.behind_asked.insert(pr.url.clone(), std::time::Instant::now());
            }
            auto.fix_seen.insert(pr.url.clone(), need.fingerprint.clone());
        }
        if need.only_behind {
            self.update_branch(pr, need, store);
        } else {
            self.ask_agent(pr, need, store);
        }
    }

    /// Ask the agent that made the PR to fix it, once per state of the PR and up to MAX_ROUNDS times. With no
    /// way to reach an agent the PR is only marked, and you are told.
    fn ask_agent(self: &Arc<Self>, pr: &PullRequest, need: FixNeed, store: &fixstate::Store) {
        let Some(fix) = self.tools().fix else {
            self.notify_if_wanted("CAPCOM: a PR needs fixing", &format!("{}: {}", name_of(pr), need.why));
            return;
        };
        let (me, store, url, name) = (self.clone(), store.clone(), pr.url.clone(), name_of(pr));
        let template = self.fix_ask(pr);
        std::thread::spawn(move || match store.claim(&url, &need.fingerprint, MAX_ROUNDS, false) {
            Claim::Same => {}
            Claim::Busy => {
                lock(&me.eng.auto).fix_seen.remove(&url);
            }
            Claim::GaveUp => me.notify_if_wanted("CAPCOM: giving up on a PR", &format!("{name} is still not right after {MAX_ROUNDS} rounds: over to you (untick and tick auto-fix on it to start again)")),
            Claim::Go { round, undo } => {
                let mut ask = template;
                ask.prompt = format!("/pr-address {url} (automatic round {round} of {MAX_ROUNDS}: {})", need.why);
                match fix(&ask) {
                    Ok(crate::dispatch::Sent::Busy) => {
                        store.undo(&url, undo);
                        lock(&me.eng.auto).fix_seen.remove(&url);
                    }
                    Ok(_) => {
                        lock(&me.eng.auto).fix_sent.insert(url, (round, std::time::Instant::now()));
                        me.event(format!("asked the agent to fix {name} (round {round} of {MAX_ROUNDS}: {})", need.why));
                    }
                    Err(e) => {
                        store.undo(&url, undo);
                        let mut auto = lock(&me.eng.auto);
                        auto.fix_seen.remove(&url);
                        auto.fix_failed.insert(url, std::time::Instant::now());
                        drop(auto);
                        me.notify_if_wanted("CAPCOM: a PR needs fixing", &format!("{name}: {}; could not reach its agent: {e}", need.why));
                    }
                }
            }
        });
    }

    /// Who to message about a PR, less the prompt: the session that made it (from the transcripts), and the
    /// story's space when it is a task's PR.
    fn fix_ask(&self, pr: &PullRequest) -> crate::dispatch::Ask {
        let name = name_of(pr);
        let session = lock(&self.eng.sessions).iter().find(|(u, _)| same_url(u, &pr.url)).map(|(_, s)| (s.session.clone(), s.cwd.clone()));
        let owner = store::list_keys(&self.root).unwrap_or_default().into_iter().find_map(|key| {
            let board = store::load(&self.root, &key).ok()?;
            let task = board.tasks.iter().find(|t| t.prs.iter().any(|p| same_url(&p.url, &pr.url)))?;
            Some((board.story.key.clone(), agents::short_name(&board.story.title), task.id.clone()))
        });
        let (space, label, tab) = match owner {
            Some((key, short, task)) => {
                let label = if short.is_empty() { key.clone() } else { format!("{key} - {short}") };
                (key, label, format!("{task} resume"))
            }
            None => (String::new(), String::new(), format!("{name} agent")),
        };
        crate::dispatch::Ask {
            prompt: String::new(),
            session,
            space,
            label,
            tab,
            repo: pr.repo.rsplit('/').next().unwrap_or_default().to_string(),
            workdir: lock(&self.eng.workdir).clone(),
        }
    }

    /// Tell once if the agent was asked a while ago and the PR still looks exactly the same.
    fn check_stalled(self: &Arc<Self>, pr: &PullRequest, fingerprint: &str, store: &fixstate::Store) {
        {
            let mut auto = lock(&self.eng.auto);
            let waited = auto.fix_sent.get(&pr.url).is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(STALL_MINUTES as u64 * 60));
            if !waited || !auto.stall_told.insert(format!("{}|{fingerprint}", pr.url)) {
                return;
            }
        }
        let (me, store, url, fp, name) = (self.clone(), store.clone(), pr.url.clone(), fingerprint.to_string(), name_of(pr));
        std::thread::spawn(move || {
            if store.stalled(&url, &fp, STALL_MINUTES) {
                me.notify_if_wanted("CAPCOM: the agent has not acted", &format!("{name} looks the same {STALL_MINUTES} minutes after it was asked to fix it"));
            }
        });
    }

    /// A branch that is only behind its base is brought up to date by asking GitHub to merge the base in: no agent.
    fn update_branch(self: &Arc<Self>, pr: &PullRequest, need: FixNeed, store: &fixstate::Store) {
        let (me, store, update) = (self.clone(), store.clone(), self.tools().update_branch);
        let (url, repo, number, head, name) = (pr.url.clone(), pr.repo.clone(), pr.number, pr.feedback.head.clone(), name_of(pr));
        std::thread::spawn(move || match store.claim(&url, &need.fingerprint, MAX_ROUNDS, false) {
            Claim::Same => {}
            Claim::Busy => {
                lock(&me.eng.auto).fix_seen.remove(&url);
            }
            Claim::GaveUp => me.event(format!("{name}: still behind after {MAX_ROUNDS} automatic updates: over to you")),
            Claim::Go { undo, .. } => match update(&repo, number, &head) {
                Ok(()) => me.event(format!("brought {name} up to date with its base branch (no agent needed)")),
                Err(e) if e.to_lowercase().contains("conflict") => {
                    store.undo(&url, undo);
                    me.notify_if_wanted("CAPCOM: a PR needs fixing", &format!("{name}: {}, and updating it runs into a merge conflict", need.why));
                }
                Err(e) => {
                    store.undo(&url, undo);
                    lock(&me.eng.auto).fix_seen.remove(&url);
                    me.event(format!("could not update {name}: {e}"));
                }
            },
        });
    }

    /// Called with every fresh run list, before it is shown: drop watched runs that finished a while ago and
    /// tell about the ones that finished since the last look.
    pub(super) fn settle_runs(&self, mut runs: Vec<Run>) -> Vec<Run> {
        let now = Utc::now();
        let keep = chrono::Duration::minutes(WATCH_KEEP_MINUTES);
        let parse = |s: &str| chrono::DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc));
        let watches = self.settings().watched;
        let mut expired = Vec::new();
        runs.retain(|run| {
            if !run.watched || run.is_active() {
                return true;
            }
            let added = watches.iter().find(|w| w.repo == run.repo && w.id == run.id).and_then(|w| parse(&w.added_at));
            let since = parse(&run.updated_at).into_iter().chain(added).max();
            let fresh = since.is_none_or(|t| now - t <= keep);
            if !fresh {
                expired.push((run.repo.clone(), run.id));
            }
            fresh
        });
        let active_now: std::collections::HashSet<(String, u64)> = runs.iter().filter(|r| r.is_active()).map(|r| (r.repo.clone(), r.id)).collect();
        let before = std::mem::replace(&mut lock(&self.eng.auto).active_runs, Some(active_now));
        if let Some(before) = before {
            for run in runs.iter().filter(|r| !r.is_active() && before.contains(&(r.repo.clone(), r.id))) {
                let word = match run.state {
                    RunState::Success => "is GREEN",
                    RunState::Failed => "FAILED",
                    RunState::Cancelled => "was cancelled",
                    _ => "finished",
                };
                let title = if run.title.is_empty() { &run.name } else { &run.title };
                self.notify_if_wanted(&format!("CAPCOM: run {word}"), &format!("{} · {title}", run.repo));
            }
        }
        if !expired.is_empty() {
            self.change_settings(|s| s.watched.retain(|w| !expired.iter().any(|(r, id)| *r == w.repo && *id == w.id)));
        }
        runs
    }
}
