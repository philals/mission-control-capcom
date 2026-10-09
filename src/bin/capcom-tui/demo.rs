//! Invented stories, pull requests and runs, so capcom-tui can be tried (and photographed for the
//! README) without a stories folder, GitHub or Herdr. Nothing here is real data.
use crate::app::App;
use crate::prs::{Check, CheckState, CopilotState, Feedback, PullRequest, Review, Thread};
use crate::runs::{Batch, Job, Run, RunState};
use capcom::model::{PrState, Status, StoryStatus, TaskType};
use capcom::{ops, rules, store};
use chrono::{Duration, SecondsFormat, Utc};
use std::path::{Path, PathBuf};

fn ago(minutes: i64) -> String {
    (Utc::now() - Duration::minutes(minutes)).to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn run_url(repo: &str, run: u64) -> String {
    format!("https://github.com/{repo}/actions/runs/{run}")
}

pub fn make_root() -> PathBuf {
    static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("capcom-demo-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    root
}

fn story(root: &Path, key: &str, title: &str, build: impl FnOnce(&Path, &mut capcom::model::Board) -> anyhow::Result<()>) {
    ops::init_story(root, key, title, Some(&format!("https://jira.example.com/browse/{key}"))).expect("demo story");
    let dir = root.join(key);
    store::update(root, key, |b| build(&dir, b)).expect("demo board");
}

fn plan(b: &mut capcom::model::Board, id: &str) -> anyhow::Result<()> {
    rules::transition(b, id, Status::Planning)?;
    rules::transition(b, id, Status::Planned)
}

fn implement(b: &mut capcom::model::Board, id: &str) -> anyhow::Result<()> {
    plan(b, id)?;
    rules::transition(b, id, Status::Implementing)
}

fn finish(b: &mut capcom::model::Board, id: &str, repo: &str, number: u64) -> anyhow::Result<()> {
    implement(b, id)?;
    let url = format!("https://github.com/acme/{repo}/pull/{number}");
    ops::add_pr(b, id, repo, &url, PrState::Merged)?;
    rules::transition(b, id, Status::Done)
}

fn build_stories(root: &Path) {
    story(root, "DEMO-101", "Notification preferences", |dir, b| {
        use TaskType::{Pr, Spike};
        ops::add_task(dir, b, "Add preferences endpoint", Pr, vec![], vec!["api".into()])?;
        ops::add_task(dir, b, "Build preferences screen", Pr, vec!["T1".into()], vec!["web".into()])?;
        ops::add_task(dir, b, "Email digest job", Pr, vec!["T1".into()], vec!["worker".into()])?;
        ops::add_task(dir, b, "Spike: push provider options", Spike, vec![], vec![])?;
        ops::add_task(dir, b, "Backfill existing users", Pr, vec!["T1".into()], vec!["api".into(), "api".into()])?;
        ops::add_task(dir, b, "Manual test and UX review", Spike, vec!["T2".into(), "T3".into()], vec![])?;
        finish(b, "T1", "api", 201)?;
        implement(b, "T2")?;
        ops::add_pr(b, "T2", "web", "https://github.com/acme/web/pull/212", PrState::Draft)?;
        plan(b, "T3")?;
        rules::transition(b, "T4", Status::Planning)?;
        Ok(())
    });
    story(root, "DEMO-102", "Audit log export", |dir, b| {
        use TaskType::Pr;
        ops::add_task(dir, b, "Export job", Pr, vec![], vec!["worker".into()])?;
        ops::add_task(dir, b, "Download button", Pr, vec!["T1".into()], vec!["web".into()])?;
        ops::add_task(dir, b, "Retention docs", Pr, vec![], vec!["docs".into()])?;
        finish(b, "T3", "docs", 17)?;
        implement(b, "T1")?;
        ops::add_pr(b, "T1", "worker", "https://github.com/acme/worker/pull/41", PrState::Ready)?;
        Ok(())
    });
    story(root, "DEMO-104", "Onboarding checklist", |dir, b| {
        use TaskType::Pr;
        ops::add_task(dir, b, "Checklist component", Pr, vec![], vec!["web".into()])?;
        ops::add_task(dir, b, "Progress endpoint", Pr, vec![], vec!["api".into()])?;
        Ok(())
    });
    story(root, "DEMO-105", "Billing export to CSV", |_, _| Ok(()));
    story(root, "DEMO-103", "Dark mode", |dir, b| {
        ops::add_task(dir, b, "Theme tokens", TaskType::Pr, vec![], vec!["web".into()])?;
        finish(b, "T1", "web", 190)?;
        rules::story_transition(b, StoryStatus::InReview)
    });
}

fn check(workflow: &str, name: &str, state: CheckState, minutes_ago: i64, minutes_taken: i64) -> Check {
    let started = Some(ago(minutes_ago));
    let finished = matches!(state, CheckState::Passed | CheckState::Failed | CheckState::Skipped);
    Check {
        name: name.into(),
        workflow: Some(workflow.into()),
        state,
        started_at: if state == CheckState::Queued { None } else { started },
        completed_at: finished.then(|| ago(minutes_ago - minutes_taken)),
        url: Some("https://github.com/acme/web/actions/runs/1/job/1".into()),
        external: false,
    }
}

fn pull_requests() -> Vec<PullRequest> {
    vec![
        PullRequest {
            feedback: Feedback {
                mine: true,
                head: "9f2c1ab".into(),
                copilot: CopilotState::Reviewed,
                copilot_reviewed_at: Some(ago(20)),
                threads: ["a", "b"].iter().map(|id| Thread { id: id.to_string(), resolved: false, outdated: false, by_copilot: true }).collect(),
                cancelled: vec![],
                merge: crate::prs::MergeState::Conflicting,
            },
            repo: "acme/web".into(),
            number: 212,
            title: "feat: DEMO-101 build the notification preferences screen (T2)".into(),
            url: "https://github.com/acme/web/pull/212".into(),
            is_draft: true,
            labels: vec!["frontend".into()],
            review: Review::None,
            comments: 2,
            updated_at: ago(6),
            checks: vec![
                check("CI", "lint", CheckState::Passed, 9, 1),
                check("CI", "build", CheckState::Passed, 9, 3),
                check("CI", "unit tests", CheckState::Running, 6, 0),
                check("CI", "e2e", CheckState::Queued, 0, 0),
                check("CI", "visual diff", CheckState::Failed, 8, 2),
            ],
        },
        PullRequest {
            feedback: Feedback { mine: true, head: "4be7710".into(), copilot: CopilotState::Requested, ..Feedback::default() },
            repo: "acme/worker".into(),
            number: 41,
            title: "feat: DEMO-102 add the audit log export job (T1)".into(),
            url: "https://github.com/acme/worker/pull/41".into(),
            is_draft: false,
            labels: vec!["backend".into(), "needs-docs".into()],
            review: Review::Required,
            comments: 0,
            updated_at: ago(25),
            checks: vec![
                check("CI", "lint", CheckState::Passed, 30, 1),
                check("CI", "tests", CheckState::Passed, 30, 7),
                check("Security", "dependency scan", CheckState::Running, 4, 0),
            ],
        },
        PullRequest {
            feedback: Default::default(),
            repo: "acme/web".into(),
            number: 215,
            title: "docs: explain the notification digest settings".into(),
            url: "https://github.com/acme/web/pull/215".into(),
            is_draft: false,
            labels: vec!["docs".into()],
            review: Review::Required,
            comments: 0,
            updated_at: ago(45),
            checks: vec![
                check("CI", "lint", CheckState::Passed, 50, 1),
                check("CI", "build", CheckState::Passed, 50, 3),
                check("CI", "unit tests", CheckState::Passed, 50, 5),
                check("CI", "e2e", CheckState::Skipped, 50, 0),
            ],
        },
        PullRequest {
            feedback: Default::default(),
            repo: "acme/api".into(),
            number: 198,
            title: "fix: retry token refresh when the identity provider is slow".into(),
            url: "https://github.com/acme/api/pull/198".into(),
            is_draft: false,
            labels: vec!["bug".into()],
            review: Review::Approved,
            comments: 3,
            updated_at: ago(130),
            checks: vec![
                check("CI", "lint", CheckState::Passed, 140, 1),
                check("CI", "tests", CheckState::Passed, 140, 6),
            ],
        },
    ]
}

fn job(name: &str, state: RunState, id: u64, n: u64, minutes_ago: i64, taken: i64) -> Job {
    let finished = matches!(state, RunState::Success | RunState::Failed);
    Job {
        name: name.into(),
        state,
        url: format!("{}/job/{n}", run_url("acme/web", id)),
        started_at: (state != RunState::Queued && state != RunState::Waiting).then(|| ago(minutes_ago)),
        completed_at: finished.then(|| ago(minutes_ago - taken)),
    }
}

fn runs() -> Batch {
    Batch {
        runs: vec![
            Run {
                repo: "acme/web".into(),
                id: 9001,
                name: "Deploy to staging".into(),
                title: "Deploy to staging".into(),
                branch: "feat/preferences-screen".into(),
                url: run_url("acme/web", 9001),
                state: RunState::Running,
                created_at: ago(5),
                started_at: Some(ago(5)),
                updated_at: ago(1),
                jobs: vec![
                    job("build", RunState::Success, 9001, 1, 5, 2),
                    job("migrate database", RunState::Running, 9001, 2, 3, 0),
                    job("smoke tests", RunState::Queued, 9001, 3, 0, 0),
                    job("approve production", RunState::Waiting, 9001, 4, 0, 0),
                ],
                watched: false,
            },
            Run {
                repo: "acme/web".into(),
                id: 9003,
                name: "Continuous Integration".into(),
                title: "feat: DEMO-101 build the notification preferences screen (T2)".into(),
                branch: "feat/preferences-screen".into(),
                url: run_url("acme/web", 9003),
                state: RunState::Running,
                created_at: ago(12),
                started_at: Some(ago(12)),
                updated_at: ago(0),
                jobs: vec![
                    job("lint", RunState::Success, 9003, 1, 12, 1),
                    job("unit tests", RunState::Running, 9003, 2, 10, 0),
                    job("e2e", RunState::Queued, 9003, 3, 0, 0),
                ],
                watched: true,
            },
            Run {
                repo: "acme/api".into(),
                id: 8990,
                name: "Deploy to staging".into(),
                title: "Deploy to staging".into(),
                branch: "main".into(),
                url: run_url("acme/api", 8990),
                state: RunState::Success,
                created_at: ago(95),
                started_at: Some(ago(95)),
                updated_at: ago(80),
                jobs: vec![],
                watched: false,
            },
        ],
        warnings: vec![],
    }
}

/// An app showing the invented data (no polling, no GitHub, no Herdr).
pub fn app(root: &Path) -> App {
    build_stories(root);
    let mut app = App::new(root.to_path_buf(), None);
    app.prs.disabled = false;
    app.apply_prs(Ok(pull_requests()));
    app.apply_runs(Ok(runs()));
    app
}

#[cfg(test)]
mod tests {
    use super::*;
    use capcom::model::Board;

    #[test]
    fn the_demo_has_stories_in_every_column_and_nothing_real() {
        let root = make_root();
        let app = app(&root);
        assert_eq!(app.keys, vec!["DEMO-101", "DEMO-102", "DEMO-103", "DEMO-104", "DEMO-105"]);
        let board: Board = store::load(&root, "DEMO-101").unwrap();
        let statuses: Vec<Status> = board.tasks.iter().map(|t| t.status).collect();
        for want in [Status::Todo, Status::Planning, Status::Planned, Status::Implementing, Status::Done] {
            assert!(statuses.contains(&want), "{want:?} missing in {statuses:?}");
        }
        assert_eq!(app.pr_rows().len(), 4);
        let text = std::fs::read_to_string(root.join("DEMO-101/board.json")).unwrap();
        assert!(text.contains("acme/") && text.contains("jira.example.com"), "only invented names and hosts");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_repo_listed_twice_shows_the_backfill_task_expecting_two_prs() {
        let root = make_root();
        let _ = app(&root);
        let board = store::load(&root, "DEMO-101").unwrap();
        assert_eq!(rules::repo_summary(&board.task("T5").unwrap().repos), "api ×2");
        let _ = std::fs::remove_dir_all(&root);
    }
}
