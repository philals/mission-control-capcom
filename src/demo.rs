//! Invented stories, pull requests and runs, so capcom can be tried (and photographed for the README)
//! without a stories folder, GitHub or Herdr. Nothing here is real data.
use crate::prs::{Check, CheckState, CopilotState, Feedback, PullRequest, Review, Thread};
use crate::runs::{Batch, Job, Run, RunState};
use crate::model::{PrState, Status, StoryStatus, TaskType};
use crate::{ops, rules, store};
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

fn story(root: &Path, key: &str, title: &str, build: impl FnOnce(&Path, &mut crate::model::Board) -> anyhow::Result<()>) {
    ops::init_story(root, key, title, Some(&format!("https://jira.example.com/browse/{key}"))).expect("demo story");
    let dir = root.join(key);
    store::update(root, key, |b| build(&dir, b)).expect("demo board");
}

fn plan(b: &mut crate::model::Board, id: &str) -> anyhow::Result<()> {
    rules::transition(b, id, Status::Planning)?;
    rules::transition(b, id, Status::Planned)
}

fn implement(b: &mut crate::model::Board, id: &str) -> anyhow::Result<()> {
    plan(b, id)?;
    rules::transition(b, id, Status::Implementing)
}

fn finish(b: &mut crate::model::Board, id: &str, repo: &str, number: u64) -> anyhow::Result<()> {
    implement(b, id)?;
    let url = format!("https://github.com/acme/{repo}/pull/{number}");
    ops::add_pr(b, id, repo, &url, PrState::Merged)?;
    rules::transition(b, id, Status::Done)
}

pub fn build_stories(root: &Path) {
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
    let finished = matches!(state, CheckState::Passed | CheckState::Failed | CheckState::Cancelled | CheckState::Skipped);
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

/// The merge bot settings of the demo repos (invented).
fn kodiak() -> crate::kodiak::Kodiak {
    crate::kodiak::Kodiak { automerge: vec!["automerge 🚀".into()], autoupdate: vec!["autoupdate".into()], update_always: false }
}

pub fn pull_requests() -> Vec<PullRequest> {
    vec![
        PullRequest {
            feedback: Feedback {
                mine: true,
                pasted: false,
                head: "9f2c1ab".into(),
                branch: "feature/demo-9f2c1ab".into(),
                copilot: CopilotState::Reviewed,
                copilot_reviewed_at: Some(ago(20)),
                threads: ["a", "b"].iter().map(|id| Thread { id: id.to_string(), resolved: false, outdated: false, by_copilot: true }).collect(),
                merge: crate::prs::MergeState::Conflicting,
                kodiak: Some(kodiak()),
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
            created_at: ago(6),
            checks: vec![
                check("CI", "lint", CheckState::Passed, 9, 1),
                check("CI", "build", CheckState::Passed, 9, 3),
                check("CI", "unit tests", CheckState::Running, 6, 0),
                check("CI", "e2e", CheckState::Queued, 0, 0),
                check("CI", "visual diff", CheckState::Failed, 8, 2),
            ],
        },
        PullRequest {
            feedback: Feedback { mine: true, head: "4be7710".into(), branch: "feature/audit-log-export".into(), copilot: CopilotState::Requested, kodiak: Some(kodiak()), ..Feedback::default() },
            repo: "acme/worker".into(),
            number: 41,
            title: "feat: DEMO-102 add the audit log export job (T1)".into(),
            url: "https://github.com/acme/worker/pull/41".into(),
            is_draft: false,
            labels: vec!["backend".into(), "needs-docs".into()],
            review: Review::Required,
            comments: 0,
            updated_at: ago(25),
            created_at: ago(25),
            checks: vec![
                check("CI", "lint", CheckState::Passed, 30, 1),
                check("CI", "tests", CheckState::Passed, 30, 7),
                check("Security", "dependency scan", CheckState::Running, 4, 0),
            ],
        },
        PullRequest {
            feedback: Feedback { mine: true, head: "c0ffee1".into(), branch: "docs/digest-settings".into(), kodiak: Some(kodiak()), ..Feedback::default() },
            repo: "acme/web".into(),
            number: 215,
            title: "docs: explain the notification digest settings".into(),
            url: "https://github.com/acme/web/pull/215".into(),
            is_draft: false,
            labels: vec!["docs".into(), "automerge 🚀".into()],
            review: Review::Required,
            comments: 0,
            updated_at: ago(45),
            created_at: ago(45),
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
            created_at: ago(130),
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

pub fn runs() -> Batch {
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

/// Your PRs merged a little while ago (shown in purple for a while).
pub fn merged() -> Batch {
    let run = |repo: &str, number: u64, title: &str, minutes: i64| Run {
        repo: repo.into(),
        id: number,
        name: "Merged".into(),
        title: title.into(),
        branch: "main".into(),
        url: format!("https://github.com/{repo}/pull/{number}"),
        state: RunState::Merged,
        created_at: ago(minutes + 60),
        started_at: None,
        updated_at: ago(minutes),
        jobs: vec![],
        watched: false,
    };
    Batch {
        runs: vec![
            run("acme/api", 201, "feat: DEMO-101 add the preferences endpoint (T1)", 9),
            run("acme/docs", 17, "docs: retention policy for audit logs", 22),
        ],
        warnings: vec![],
    }
}

/// An invented agent: what it is called, what it is doing, and the PR it made (if any).
pub struct Agent {
    pub name: &'static str,
    pub status: crate::agents::Status,
    pub session: &'static str,
    pub task: &'static str,
    pub progress: &'static str,
    pub waiting: &'static str,
    pub needs_you: bool,
    pub next: &'static str,
    pub pr: Option<&'static str>,
    pub cwd: &'static str,
}

/// The invented agents, in the order Herdr's terminals are started for the pictures (t1, t2, ...).
pub fn agent_specs() -> Vec<Agent> {
    use crate::agents::Status::{Blocked, Idle, Working};
    vec![
        Agent {
            name: "demo-101-t2-preferences-screen",
            status: Working,
            session: "demo-session-1",
            task: "Build the notification preferences screen",
            progress: "Screen and form done; fixing the visual diff that failed in CI",
            waiting: "",
            needs_you: false,
            next: "",
            pr: Some("https://github.com/acme/web/pull/212"),
            cwd: "/home/demo/code/acme/web",
        },
        Agent {
            name: "demo-102-t1-export-job",
            status: Blocked,
            session: "demo-session-2",
            task: "Add the audit log export job",
            progress: "PR is ready and CI is green",
            waiting: "A reviewer to approve the PR",
            needs_you: true,
            next: "Ask a teammate for a review, or approve it yourself",
            pr: Some("https://github.com/acme/worker/pull/41"),
            cwd: "/home/demo/code/acme/worker",
        },
        Agent {
            name: "demo-101-t3-digest-docs",
            status: Idle,
            session: "demo-session-3",
            task: "Explain the notification digest settings",
            progress: "Docs PR open with automerge on",
            waiting: "Checks to finish",
            needs_you: false,
            next: "",
            pr: Some("https://github.com/acme/web/pull/215"),
            cwd: "/home/demo/code/acme/web",
        },
        Agent {
            name: "demo-104-t1-checklist-component",
            status: Working,
            session: "demo-session-4",
            task: "Build the onboarding checklist component",
            progress: "Planning the component states",
            waiting: "",
            needs_you: false,
            next: "",
            pr: None,
            cwd: "/home/demo/code/acme/web",
        },
    ]
}

/// The terminal each demo session runs in when the pictures are taken: `CAPCOM_DEMO_TERMINALS` (comma
/// separated, in the order of [`agent_specs`]), else t1, t2, ...
pub fn terminal_of(session: &str) -> Option<String> {
    let at = agent_specs().iter().position(|a| a.session == session)?;
    match std::env::var("CAPCOM_DEMO_TERMINALS") {
        Ok(list) if !list.is_empty() => list.split(',').nth(at).map(|t| t.trim().to_string()),
        _ => Some(format!("t{}", at + 1)),
    }
}

pub fn agents() -> Vec<crate::agents::AgentInfo> {
    agent_specs()
        .into_iter()
        .enumerate()
        .map(|(i, a)| crate::agents::AgentInfo {
            pane: format!("demo-pane-{}", i + 1),
            name: Some(a.name.into()),
            status: a.status,
            topic: a.task.into(),
            workspace: "demo".into(),
            cwd: a.cwd.into(),
            session: Some(a.session.into()),
        })
        .collect()
}

/// The PRs the demo agents made, and the session that made each.
pub fn sessions() -> Vec<(String, crate::session::PrSession)> {
    agent_specs().into_iter().filter_map(|a| Some((a.pr?.to_string(), crate::session::PrSession { session: a.session.into(), cwd: a.cwd.into() }))).collect()
}

/// Write each demo agent's note into `dir`.
pub fn write_notes(dir: &Path) {
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    for a in agent_specs() {
        let update = crate::notes::Update {
            task: Some(a.task.into()),
            progress: Some(a.progress.into()),
            waiting: Some(a.waiting.into()),
            needs_you: Some(a.needs_you),
            next: Some(a.next.into()),
            cwd: Some(a.cwd.into()),
        };
        let _ = crate::notes::write(dir, a.session, update, &now);
    }
}

/// A plan-usage file the page can read: 5-hour and weekly windows partly used.
pub fn write_usage(path: &Path) {
    let now = Utc::now();
    let json = serde_json::json!({
        "at": now.timestamp(),
        "rate_limits": {
            "five_hour": { "used_percentage": 38.0, "resets_at": (now + Duration::minutes(130)).timestamp() },
            "seven_day": { "used_percentage": 54.0, "resets_at": (now + Duration::hours(70)).timestamp() },
        },
    });
    let _ = std::fs::write(path, json.to_string());
}

/// Serves the invented PRs and runs wherever a GitHub source is wanted.
pub struct Source;

impl crate::prs::PrSource for Source {
    fn fetch(&self) -> anyhow::Result<Vec<PullRequest>> {
        Ok(pull_requests())
    }
}

impl crate::runs::RunSource for Source {
    fn fetch(&self, _repos: &[String]) -> anyhow::Result<Batch> {
        Ok(runs())
    }

    fn fetch_merged(&self, _force: bool) -> anyhow::Result<Batch> {
        Ok(merged())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_demo_agent_pr_is_one_of_the_demo_prs_and_has_a_terminal() {
        let urls: Vec<String> = pull_requests().into_iter().map(|p| p.url).collect();
        for agent in agent_specs() {
            if let Some(pr) = agent.pr {
                assert!(urls.iter().any(|u| u == pr), "{pr} is not a demo PR");
            }
            assert!(terminal_of(agent.session).is_some());
        }
        assert_eq!(sessions().len(), agent_specs().iter().filter(|a| a.pr.is_some()).count());
    }

    #[test]
    fn the_demo_prs_json_names_the_agent_of_each_pr_it_has_one_for() {
        let doc = crate::prs_json::build_demo();
        let open = doc["open"].as_array().unwrap();
        assert_eq!(open.len(), pull_requests().len());
        assert_eq!(open.iter().filter(|p| !p["agent"].is_null()).count(), sessions().len());
        assert!(!doc["merged"].as_array().unwrap().is_empty());
    }

    #[test]
    fn nothing_in_the_demo_looks_real() {
        let text = format!("{:?}{:?}", agent_specs().iter().map(|a| (a.cwd, a.task)).collect::<Vec<_>>(), pull_requests().iter().map(|p| &p.url).collect::<Vec<_>>());
        assert!(text.contains("acme/") && text.contains("/home/demo/"));
    }
}
