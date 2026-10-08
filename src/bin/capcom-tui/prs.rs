//! GitHub pull requests for the TUI: one `gh api graphql` query, parsed, polled in the background.
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;
use capcom::refresh::run_with_timeout;

pub const DEFAULT_QUERY: &str =
    "is:pr author:@me state:open archived:false sort:updated-desc -label:icebox";

const GRAPHQL: &str = r#"query($q: String!) { search(query: $q, type: ISSUE, first: 50) { nodes { ... on PullRequest { number title url isDraft updatedAt reviewDecision comments { totalCount } repository { nameWithOwner } labels(first: 10) { nodes { name } } statusCheckRollup { state contexts(first: 100) { nodes { __typename ... on CheckRun { name status conclusion startedAt completedAt detailsUrl checkSuite { workflowRun { workflow { name } } } } ... on StatusContext { context state targetUrl } } } } } } } }"#;

const GH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Review {
    None,
    Approved,
    ChangesRequested,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Queued,
    Running,
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub workflow: Option<String>,
    pub state: CheckState,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub url: Option<String>,
}

impl Check {
    pub fn label(&self) -> String {
        match &self.workflow {
            Some(w) if *w != self.name => format!("{w} / {}", self.name),
            _ => self.name.clone(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub passed: usize,
    pub failed: usize,
    pub running: usize,
    pub queued: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub is_draft: bool,
    pub labels: Vec<String>,
    pub review: Review,
    pub comments: u64,
    pub updated_at: String,
    pub checks: Vec<Check>,
}

impl PullRequest {
    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for check in &self.checks {
            match check.state {
                CheckState::Passed => c.passed += 1,
                CheckState::Failed => c.failed += 1,
                CheckState::Running => c.running += 1,
                CheckState::Queued => c.queued += 1,
                CheckState::Skipped => c.skipped += 1,
            }
        }
        c
    }

    pub fn is_active(&self) -> bool {
        self.checks.iter().any(|c| matches!(c.state, CheckState::Running | CheckState::Queued))
    }

    pub fn checks_in(&self, state: CheckState) -> Vec<&Check> {
        self.checks.iter().filter(|c| c.state == state).collect()
    }

    fn checks_in_order(&self, order: &[CheckState]) -> Vec<&Check> {
        order.iter().flat_map(|s| self.checks_in(*s)).collect()
    }

    /// The checks worth watching in the panel, one per line: running, queued, failed.
    pub fn open_checks(&self) -> Vec<&Check> {
        self.checks_in_order(&[CheckState::Running, CheckState::Queued, CheckState::Failed])
    }

    /// Every check in the order the detail sheet lists them.
    pub fn ordered_checks(&self) -> Vec<&Check> {
        self.checks_in_order(&[
            CheckState::Running,
            CheckState::Queued,
            CheckState::Failed,
            CheckState::Passed,
            CheckState::Skipped,
        ])
    }
}

fn text(v: &Value, pointer: &str) -> Option<String> {
    v.pointer(pointer).and_then(Value::as_str).map(str::to_string)
}

fn check_state(status: &str, conclusion: &str) -> CheckState {
    match status {
        "IN_PROGRESS" => CheckState::Running,
        "COMPLETED" => match conclusion {
            "SUCCESS" => CheckState::Passed,
            "FAILURE" | "TIMED_OUT" | "STARTUP_FAILURE" => CheckState::Failed,
            _ => CheckState::Skipped,
        },
        _ => CheckState::Queued,
    }
}

fn status_state(state: &str) -> CheckState {
    match state {
        "SUCCESS" => CheckState::Passed,
        "FAILURE" | "ERROR" => CheckState::Failed,
        _ => CheckState::Queued,
    }
}

fn parse_check(ctx: &Value) -> Option<Check> {
    match ctx.get("__typename")?.as_str()? {
        "CheckRun" => Some(Check {
            name: text(ctx, "/name")?,
            workflow: text(ctx, "/checkSuite/workflowRun/workflow/name"),
            state: check_state(
                &text(ctx, "/status").unwrap_or_default(),
                &text(ctx, "/conclusion").unwrap_or_default(),
            ),
            started_at: text(ctx, "/startedAt"),
            completed_at: text(ctx, "/completedAt"),
            url: text(ctx, "/detailsUrl"),
        }),
        "StatusContext" => Some(Check {
            name: text(ctx, "/context")?,
            workflow: None,
            state: status_state(&text(ctx, "/state").unwrap_or_default()),
            started_at: None,
            completed_at: None,
            url: text(ctx, "/targetUrl"),
        }),
        _ => None,
    }
}

fn parse_pr(node: &Value) -> Option<PullRequest> {
    let number = node.get("number")?.as_u64()?;
    let labels = node
        .pointer("/labels/nodes")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(|n| text(n, "/name")).collect())
        .unwrap_or_default();
    let checks = node
        .pointer("/statusCheckRollup/contexts/nodes")
        .and_then(Value::as_array)
        .map(|c| c.iter().filter_map(parse_check).collect())
        .unwrap_or_default();
    Some(PullRequest {
        repo: text(node, "/repository/nameWithOwner")?,
        number,
        title: text(node, "/title")?,
        url: text(node, "/url")?,
        is_draft: node.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        labels,
        review: match text(node, "/reviewDecision").as_deref() {
            Some("APPROVED") => Review::Approved,
            Some("CHANGES_REQUESTED") => Review::ChangesRequested,
            Some("REVIEW_REQUIRED") => Review::Required,
            _ => Review::None,
        },
        comments: node.pointer("/comments/totalCount").and_then(Value::as_u64).unwrap_or(0),
        updated_at: text(node, "/updatedAt").unwrap_or_default(),
        checks,
    })
}

pub fn parse(json: &str) -> Result<Vec<PullRequest>> {
    let value: Value = serde_json::from_str(json).context("parsing gh output")?;
    let Some(nodes) = value.pointer("/data/search/nodes").and_then(Value::as_array) else {
        let message = text(&value, "/errors/0/message")
            .unwrap_or_else(|| "unexpected response from GitHub".to_string());
        bail!("{message}");
    };
    Ok(nodes.iter().filter_map(parse_pr).collect())
}

fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc))
}

fn plural(n: i64, unit: &str) -> String {
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

pub fn relative(then: &str, now: DateTime<Utc>) -> String {
    let Some(then) = parse_time(then) else {
        return String::new();
    };
    let secs = (now - then).num_seconds().max(0);
    match secs {
        0..=59 => "just now".to_string(),
        60..=3599 => plural(secs / 60, "minute"),
        3600..=86399 => plural(secs / 3600, "hour"),
        86400..=2591999 => plural(secs / 86400, "day"),
        _ => plural(secs / 2592000, "month"),
    }
}

pub fn duration_text(secs: i64) -> String {
    match secs {
        i64::MIN..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// How long a check has run (still counting if it is running), or None if it has not started.
pub fn check_seconds(check: &Check, now: DateTime<Utc>) -> Option<i64> {
    let start = parse_time(check.started_at.as_deref()?)?;
    let end = match &check.completed_at {
        Some(done) => parse_time(done)?,
        None => now,
    };
    Some((end - start).num_seconds().max(0))
}

pub trait PrSource: Send + Sync {
    fn fetch(&self) -> Result<Vec<PullRequest>>;
}

pub struct GhSource {
    pub query: String,
}

impl PrSource for GhSource {
    fn fetch(&self) -> Result<Vec<PullRequest>> {
        let mut cmd = Command::new("gh");
        cmd.args(["api", "graphql", "-f"])
            .arg(format!("query={GRAPHQL}"))
            .arg("-f")
            .arg(format!("q={}", self.query));
        let Some(out) = run_with_timeout(cmd, GH_TIMEOUT).context("running gh")? else {
            bail!("gh timed out after {}s", GH_TIMEOUT.as_secs());
        };
        if !out.status.success() {
            bail!("gh failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        parse(&String::from_utf8_lossy(&out.stdout))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Cadence {
    pub busy: Duration,
    pub idle: Duration,
}

impl Default for Cadence {
    fn default() -> Self {
        Cadence { busy: Duration::from_secs(5), idle: Duration::from_secs(30) }
    }
}

impl Cadence {
    /// Poll quickly while any check is running or queued, slowly otherwise (and after errors).
    pub fn next(&self, result: &Result<Vec<PullRequest>, String>) -> Duration {
        match result {
            Ok(prs) if prs.iter().any(PullRequest::is_active) => self.busy,
            _ => self.idle,
        }
    }
}

pub enum PrMsg {
    Started,
    Result(Result<Vec<PullRequest>, String>),
}

/// Fetch in a background thread. Send on the returned sender to refresh right away.
pub fn spawn(source: Arc<dyn PrSource>, cadence: Cadence) -> (Receiver<PrMsg>, Sender<()>) {
    let (tx, rx) = mpsc::channel();
    let (wake_tx, wake_rx) = mpsc::channel();
    std::thread::spawn(move || loop {
        if tx.send(PrMsg::Started).is_err() {
            break;
        }
        let result = source.fetch().map_err(|e| format!("{e:#}"));
        let wait = cadence.next(&result);
        if tx.send(PrMsg::Result(result)).is_err() {
            break;
        }
        if let Err(mpsc::RecvTimeoutError::Disconnected) = wake_rx.recv_timeout(wait) {
            break;
        }
        while wake_rx.try_recv().is_ok() {}
    });
    (rx, wake_tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const FIXTURE: &str = r#"{"data":{"search":{"nodes":[
      {"number":12,"title":"Add notices","url":"https://github.com/acme/widgets/pull/12","isDraft":false,
       "updatedAt":"2026-10-08T01:00:00Z","reviewDecision":"APPROVED","comments":{"totalCount":3},
       "repository":{"nameWithOwner":"acme/widgets"},"labels":{"nodes":[{"name":"bug"},{"name":"ui"}]},
       "statusCheckRollup":{"state":"PENDING","contexts":{"nodes":[
         {"__typename":"CheckRun","name":"build","status":"IN_PROGRESS","conclusion":null,"startedAt":"2026-10-08T01:00:10Z","completedAt":null,"detailsUrl":"https://github.com/acme/widgets/actions/runs/101/job/7","checkSuite":{"workflowRun":{"workflow":{"name":"CI"}}}},
         {"__typename":"CheckRun","name":"deploy","status":"QUEUED","conclusion":null,"startedAt":null,"completedAt":null,"checkSuite":{"workflowRun":{"workflow":{"name":"CI"}}}},
         {"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"SUCCESS","startedAt":"2026-10-08T01:00:00Z","completedAt":"2026-10-08T01:01:30Z","checkSuite":{"workflowRun":null}},
         {"__typename":"CheckRun","name":"unit","status":"COMPLETED","conclusion":"FAILURE","startedAt":"2026-10-08T01:00:00Z","completedAt":"2026-10-08T01:02:00Z","checkSuite":{"workflowRun":{"workflow":{"name":"CI"}}}},
         {"__typename":"CheckRun","name":"docs","status":"COMPLETED","conclusion":"SKIPPED","startedAt":null,"completedAt":null,"checkSuite":{"workflowRun":null}},
         {"__typename":"StatusContext","context":"ci/legacy","state":"PENDING","targetUrl":"https://ci.example/legacy/1"},
         {"__typename":"StatusContext","context":"ci/other","state":"SUCCESS","targetUrl":null}
       ]}}},
      {"number":7,"title":"Draft thing","url":"https://github.com/acme/api/pull/7","isDraft":true,
       "updatedAt":"2026-10-07T01:00:00Z","reviewDecision":null,"comments":{"totalCount":0},
       "repository":{"nameWithOwner":"acme/api"},"labels":{"nodes":[]},"statusCheckRollup":null},
      {}
    ]}}}"#;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 8, 4, 0, 0).unwrap()
    }

    #[test]
    fn parse_reads_pull_requests_labels_review_and_every_check() {
        let prs = parse(FIXTURE).unwrap();
        assert_eq!(prs.len(), 2, "the empty non-PR node is skipped");
        let a = &prs[0];
        assert_eq!((a.repo.as_str(), a.number, a.title.as_str()), ("acme/widgets", 12, "Add notices"));
        assert_eq!(a.url, "https://github.com/acme/widgets/pull/12");
        assert_eq!(a.labels, vec!["bug", "ui"]);
        assert_eq!((a.review, a.comments, a.is_draft), (Review::Approved, 3, false));
        let c = a.counts();
        assert_eq!((c.passed, c.failed, c.running, c.queued, c.skipped), (2, 1, 1, 2, 1));
        let build = a.checks.iter().find(|c| c.name == "build").unwrap();
        assert_eq!(build.state, CheckState::Running);
        assert_eq!(build.workflow.as_deref(), Some("CI"));
        let lint = a.checks.iter().find(|c| c.name == "lint").unwrap();
        assert_eq!((lint.state, lint.workflow.as_deref()), (CheckState::Passed, None));
        assert!(a.is_active());
        assert_eq!(build.url.as_deref(), Some("https://github.com/acme/widgets/actions/runs/101/job/7"));
        assert_eq!(lint.url, None, "a check without a link has none");
        let legacy = a.checks.iter().find(|c| c.name == "ci/legacy").unwrap();
        assert_eq!(legacy.url.as_deref(), Some("https://ci.example/legacy/1"));
        let b = &prs[1];
        assert!(b.is_draft && b.checks.is_empty() && b.review == Review::None);
        assert!(!b.is_active());
    }

    #[test]
    fn checks_come_in_a_watching_order_for_the_panel_and_a_full_order_for_the_sheet() {
        let pr = &parse(FIXTURE).unwrap()[0];
        let names = |checks: Vec<&Check>| checks.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(pr.open_checks()), vec!["build", "deploy", "ci/legacy", "unit"], "running, queued, failed");
        assert_eq!(
            names(pr.ordered_checks()),
            vec!["build", "deploy", "ci/legacy", "unit", "lint", "ci/other", "docs"],
            "running, queued, failed, passed, skipped"
        );
    }

    #[test]
    fn parse_reports_graphql_errors_and_accepts_an_empty_result() {
        let err = parse(r#"{"errors":[{"message":"Bad credentials"}]}"#).unwrap_err();
        assert!(err.to_string().contains("Bad credentials"), "{err}");
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"data":{"search":{"nodes":[]}}}"#).unwrap().is_empty());
    }

    #[test]
    fn the_poll_interval_is_short_only_while_checks_are_running_or_queued() {
        let cadence = Cadence { busy: Duration::from_secs(5), idle: Duration::from_secs(30) };
        let active = parse(FIXTURE).unwrap();
        assert_eq!(cadence.next(&Ok(active.clone())), Duration::from_secs(5));
        assert_eq!(cadence.next(&Ok(active[1..].to_vec())), Duration::from_secs(30));
        assert_eq!(cadence.next(&Err("boom".into())), Duration::from_secs(30));
    }

    #[test]
    fn relative_time_reads_like_github() {
        let at = |h: u32, m: u32, s: u32| Utc.with_ymd_and_hms(2026, 10, 8, h, m, s).unwrap().to_rfc3339();
        assert_eq!(relative(&at(3, 59, 40), now()), "just now");
        assert_eq!(relative(&at(3, 59, 0), now()), "1 minute ago");
        assert_eq!(relative(&at(3, 55, 0), now()), "5 minutes ago");
        assert_eq!(relative(&at(1, 0, 0), now()), "3 hours ago");
        assert_eq!(relative("2026-10-06T04:00:00Z", now()), "2 days ago");
        assert_eq!(relative("2026-08-01T04:00:00Z", now()), "2 months ago");
        assert_eq!(relative("garbage", now()), "");
    }

    #[test]
    fn durations_are_short_and_readable() {
        assert_eq!(duration_text(45), "45s");
        assert_eq!(duration_text(125), "2m 05s");
        assert_eq!(duration_text(3725), "1h 02m");
        let prs = parse(FIXTURE).unwrap();
        let build = prs[0].checks.iter().find(|c| c.name == "build").unwrap();
        assert_eq!(check_seconds(build, now()), Some(3 * 3600 - 10));
        let unit = prs[0].checks.iter().find(|c| c.name == "unit").unwrap();
        assert_eq!(check_seconds(unit, now()), Some(120));
        let deploy = prs[0].checks.iter().find(|c| c.name == "deploy").unwrap();
        assert_eq!(check_seconds(deploy, now()), None);
    }

    struct Fake {
        calls: AtomicUsize,
    }

    impl PrSource for Fake {
        fn fetch(&self) -> Result<Vec<PullRequest>> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                bail!("boom");
            }
            Ok(Vec::new())
        }
    }

    fn next_result(rx: &Receiver<PrMsg>) -> Result<Vec<PullRequest>, String> {
        loop {
            match rx.recv_timeout(Duration::from_secs(3)).expect("a result in time") {
                PrMsg::Started => continue,
                PrMsg::Result(r) => return r,
            }
        }
    }

    #[test]
    fn the_poller_reports_an_error_then_keeps_going() {
        let source = Arc::new(Fake { calls: AtomicUsize::new(0) });
        let cadence = Cadence { busy: Duration::from_millis(20), idle: Duration::from_millis(20) };
        let (rx, _wake) = spawn(source, cadence);
        assert!(next_result(&rx).unwrap_err().contains("boom"));
        assert!(next_result(&rx).unwrap().is_empty());
        assert!(next_result(&rx).unwrap().is_empty());
    }

    #[test]
    fn a_wake_signal_refreshes_immediately() {
        let source = Arc::new(Fake { calls: AtomicUsize::new(1) });
        let cadence = Cadence { busy: Duration::from_secs(30), idle: Duration::from_secs(30) };
        let (rx, wake) = spawn(source, cadence);
        assert!(next_result(&rx).is_ok());
        wake.send(()).unwrap();
        assert!(next_result(&rx).is_ok());
    }
}
