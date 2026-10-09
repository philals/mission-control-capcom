//! Finishing a task from the board: check its PRs on GitHub, record what was found and, only when
//! every PR is merged, move the task to done. Runs on a background thread.
use crate::app::AppMsg;
use capcom::model::{Board, PrState, Status, Task};
use capcom::refresh::{self, PrLookup};
use capcom::{rules, store};
use std::path::Path;

fn notice(text: impl Into<String>) -> Vec<AppMsg> {
    vec![AppMsg::Notice(text.into())]
}

fn pr_number(url: &str) -> String {
    format!("#{}", url.rsplit('/').next().unwrap_or(url))
}

fn state_words(state: PrState) -> &'static str {
    match state {
        PrState::Draft => "is still a draft",
        PrState::Ready => "is still open",
        PrState::Merged => "is merged",
        PrState::Closed => "is closed",
    }
}

/// Why a task did not finish: its PRs that are not merged, and anything GitHub or the board said.
fn not_done(task: &Task, messages: &[String]) -> String {
    let mut reasons: Vec<String> = task
        .prs
        .iter()
        .filter(|p| p.state != PrState::Merged && p.state != PrState::Closed)
        .map(|p| format!("{} {}", pr_number(&p.url), state_words(p.state)))
        .collect();
    let prefix = format!("{}:", task.id);
    reasons.extend(
        messages
            .iter()
            .filter(|m| m.starts_with(&prefix) && !m.contains("->"))
            .map(|m| m[prefix.len()..].trim().to_string()),
    );
    if reasons.is_empty() {
        reasons.push("it has no merged PR yet".into());
    }
    format!("{} is not done: {}", task.id, reasons.join("; "))
}

/// True when the task's progress notes record a worktree path (a line with both).
pub fn mentions_worktrees(text: &str) -> bool {
    text.lines().any(|l| l.to_lowercase().contains("worktree") && l.contains('/'))
}

fn every_task_finished(board: &Board) -> bool {
    board.tasks.iter().all(|t| t.status.is_terminal())
}

/// Check one task's PRs and move it to done if they are all merged (or it has none to wait for).
pub fn finish(root: &Path, key: &str, id: &str, lookup: &dyn PrLookup) -> Vec<AppMsg> {
    let board = match store::load(root, key) {
        Ok(b) => b,
        Err(e) => return notice(format!("could not read {key}: {e:#}")),
    };
    let urls = refresh::task_urls(&board, id);
    let lookups = refresh::lookup_all(&urls, lookup);
    let result = store::update(root, key, |b| {
        if urls.is_empty() {
            rules::transition(b, id, Status::Done)?;
            Ok(Vec::new())
        } else {
            Ok(refresh::apply(b, &lookups))
        }
    });
    let messages = match result {
        Ok(m) => m,
        Err(e) => return notice(format!("{id} is not done: {e:#}")),
    };
    let after = match store::load(root, key) {
        Ok(b) => b,
        Err(e) => return notice(format!("could not read {key}: {e:#}")),
    };
    let Some(task) = after.task(id) else {
        return notice(format!("no task {id}"));
    };
    if task.status != Status::Done {
        return notice(not_done(task, &messages));
    }
    let mut text = if urls.is_empty() { format!("{id} is done") } else { format!("{id} is done: every PR is merged") };
    if every_task_finished(&after) {
        text.push_str(&format!(". Every task is done: run /story-review {key}"));
    }
    let mut out = notice(text);
    let notes = store::story_dir(root, key).ok().and_then(|d| std::fs::read_to_string(d.join(&task.file)).ok());
    if notes.is_some_and(|t| mentions_worktrees(&t)) {
        out.push(AppMsg::Cleanup(id.to_string()));
    }
    out
}

/// What `capcom refresh` does: look up every unfinished task's PRs, record them, finish tasks whose
/// PRs are all merged.
pub fn sync_story(root: &Path, key: &str, lookup: &dyn PrLookup) -> Vec<AppMsg> {
    let board = match store::load(root, key) {
        Ok(b) => b,
        Err(e) => return notice(format!("could not read {key}: {e:#}")),
    };
    let lookups = refresh::lookup_all(&refresh::pending_urls(&board), lookup);
    match store::update(root, key, |b| Ok((refresh::apply(b, &lookups), every_task_finished(b)))) {
        Ok((messages, all_done)) if messages.is_empty() => {
            notice(if all_done { format!("{key}: nothing to update") } else { format!("{key}: PR states are already up to date") })
        }
        Ok((messages, all_done)) => {
            let mut text = format!("{key}: {}", messages.join("; "));
            if all_done {
                text.push_str(&format!(". Every task is done: run /story-review {key}"));
            }
            notice(text)
        }
        Err(e) => notice(format!("could not refresh {key}: {e:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use capcom::model::TaskType;
    use capcom::ops;
    use std::collections::HashMap;
    use tempfile::TempDir;

    struct Fake(HashMap<String, PrState>);

    impl PrLookup for Fake {
        fn state(&self, url: &str) -> Result<PrState> {
            self.0.get(url).copied().ok_or_else(|| anyhow::anyhow!("no such PR"))
        }
    }

    const U: &str = "https://github.com/acme/api/pull/7";

    fn board(kind: TaskType, with_pr: Option<PrState>) -> TempDir {
        let root = TempDir::new().unwrap();
        ops::init_story(root.path(), "PROJ-1", "Notices", None).unwrap();
        store::update(root.path(), "PROJ-1", |b| {
            let dir = root.path().join("PROJ-1");
            ops::add_task(&dir, b, "Do it", kind, vec![], vec!["api".into()])?;
            rules::transition(b, "T1", Status::Planning)?;
            rules::transition(b, "T1", Status::Planned)?;
            rules::transition(b, "T1", Status::Implementing)?;
            if let Some(state) = with_pr {
                ops::add_pr(b, "T1", "api", U, state)?;
            }
            Ok(())
        })
        .unwrap();
        root
    }

    fn status(root: &TempDir) -> Status {
        store::load(root.path(), "PROJ-1").unwrap().task("T1").unwrap().status
    }

    fn notices(msgs: &[AppMsg]) -> String {
        msgs.iter()
            .map(|m| match m {
                AppMsg::Notice(t) => t.clone(),
                AppMsg::Cleanup(id) => format!("cleanup {id}"),
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }

    #[test]
    fn a_merged_pr_finishes_the_task_and_updates_the_recorded_state() {
        let root = board(TaskType::Pr, Some(PrState::Draft));
        let out = finish(root.path(), "PROJ-1", "T1", &Fake([(U.to_string(), PrState::Merged)].into()));
        assert_eq!(status(&root), Status::Done);
        let b = store::load(root.path(), "PROJ-1").unwrap();
        assert_eq!(b.task("T1").unwrap().prs[0].state, PrState::Merged);
        let text = notices(&out);
        assert!(text.contains("T1 is done: every PR is merged"), "{text}");
        assert!(text.contains("run /story-review PROJ-1"), "the last task done points at the review: {text}");
    }

    #[test]
    fn an_open_pr_keeps_the_task_and_says_which_pr() {
        let root = board(TaskType::Pr, Some(PrState::Draft));
        let out = finish(root.path(), "PROJ-1", "T1", &Fake([(U.to_string(), PrState::Ready)].into()));
        assert_eq!(status(&root), Status::Implementing);
        assert!(notices(&out).contains("T1 is not done: #7 is still open"), "{}", notices(&out));
        let b = store::load(root.path(), "PROJ-1").unwrap();
        assert_eq!(b.task("T1").unwrap().prs[0].state, PrState::Ready, "what GitHub said is recorded");
    }

    #[test]
    fn a_github_failure_is_reported_and_nothing_moves() {
        let root = board(TaskType::Pr, Some(PrState::Draft));
        let out = finish(root.path(), "PROJ-1", "T1", &Fake(HashMap::new()));
        assert_eq!(status(&root), Status::Implementing);
        let text = notices(&out);
        assert!(text.contains("not done") && text.contains("could not refresh"), "{text}");
    }

    #[test]
    fn a_task_with_no_prs_just_finishes_unless_it_is_a_pr_task() {
        let spike = board(TaskType::Spike, None);
        let out = finish(spike.path(), "PROJ-1", "T1", &Fake(HashMap::new()));
        assert_eq!(status(&spike), Status::Done);
        assert!(notices(&out).starts_with("T1 is done"), "{}", notices(&out));
        let pr_task = board(TaskType::Pr, None);
        let out = finish(pr_task.path(), "PROJ-1", "T1", &Fake(HashMap::new()));
        assert_eq!(status(&pr_task), Status::Implementing);
        assert!(notices(&out).contains("not done"), "{}", notices(&out));
    }

    #[test]
    fn recorded_worktrees_lead_to_the_cleanup_question() {
        let root = board(TaskType::Pr, Some(PrState::Merged));
        let file = root.path().join("PROJ-1/tasks");
        let task_file = std::fs::read_dir(&file).unwrap().next().unwrap().unwrap().path();
        std::fs::write(&task_file, "## Progress\n- worktree: /work/api-t1\n").unwrap();
        let out = finish(root.path(), "PROJ-1", "T1", &Fake([(U.to_string(), PrState::Merged)].into()));
        assert!(out.iter().any(|m| matches!(m, AppMsg::Cleanup(id) if id == "T1")), "{}", notices(&out));
        let plain = board(TaskType::Pr, Some(PrState::Merged));
        let out = finish(plain.path(), "PROJ-1", "T1", &Fake([(U.to_string(), PrState::Merged)].into()));
        assert!(!out.iter().any(|m| matches!(m, AppMsg::Cleanup(_))));
    }

    #[test]
    fn a_merged_pr_is_not_enough_while_another_repo_still_needs_one() {
        let root = board(TaskType::Pr, Some(PrState::Draft));
        store::update(root.path(), "PROJ-1", |b| ops::set_repos(b, "T1", vec!["api".into(), "ui".into()])).unwrap();
        let out = finish(root.path(), "PROJ-1", "T1", &Fake([(U.to_string(), PrState::Merged)].into()));
        assert_eq!(status(&root), Status::Implementing);
        assert!(notices(&out).contains("T1 is not done: waiting for PRs in: ui"), "{}", notices(&out));
        store::update(root.path(), "PROJ-1", |b| ops::add_pr(b, "T1", "ui", "https://github.com/acme/ui/pull/3", PrState::Draft)).unwrap();
        let both = Fake([(U.to_string(), PrState::Merged), ("https://github.com/acme/ui/pull/3".to_string(), PrState::Ready)].into());
        let out = finish(root.path(), "PROJ-1", "T1", &both);
        assert!(notices(&out).contains("#3 is still open"), "the second PR is named once it exists: {}", notices(&out));
    }

    #[test]
    fn worktree_detection_needs_a_path_on_the_line() {
        assert!(mentions_worktrees("- worktree: /work/x"));
        assert!(mentions_worktrees("Worktree at ~/code/x-t1"));
        assert!(!mentions_worktrees("Main repo, not a worktree"));
        assert!(!mentions_worktrees("nothing here"));
    }

    #[test]
    fn syncing_the_story_records_states_and_finishes_merged_tasks() {
        let root = board(TaskType::Pr, Some(PrState::Draft));
        let out = sync_story(root.path(), "PROJ-1", &Fake([(U.to_string(), PrState::Merged)].into()));
        assert_eq!(status(&root), Status::Done);
        let text = notices(&out);
        assert!(text.contains("implementing -> done") && text.contains("run /story-review"), "{text}");
        let again = sync_story(root.path(), "PROJ-1", &Fake(HashMap::new()));
        assert!(notices(&again).contains("nothing to update"), "{}", notices(&again));
    }

    #[test]
    fn syncing_an_unchanged_story_says_so() {
        let root = board(TaskType::Pr, Some(PrState::Draft));
        let out = sync_story(root.path(), "PROJ-1", &Fake([(U.to_string(), PrState::Draft)].into()));
        assert_eq!(notices(&out), "PROJ-1: PR states are already up to date");
    }
}
