//! GitHub pull requests for the TUI: one `gh api graphql` query, parsed, polled in the background.
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use crate::cache::{self, Cache};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use capcom::refresh::run_with_timeout;

pub const DEFAULT_QUERY: &str =
    "is:pr author:@me state:open archived:false sort:updated-desc -label:icebox";

/// The PR list. `reviewThreads` is left out on purpose: GitHub charges for the largest result a
/// connection could return, so 50 threads each with a comment, on up to 50 PRs, cost about 28 points a poll.
/// What is asked of every PR, in the search and in a lookup by number.
macro_rules! pr_fields {
    () => {
        r#"id number title url isDraft viewerDidAuthor headRefOid mergeable mergeStateStatus createdAt updatedAt reviewDecision comments { totalCount } reviewRequests(first: 10) { nodes { requestedReviewer { __typename ... on Bot { login } } } } latestReviews(first: 20) { nodes { author { login } state submittedAt } } repository { nameWithOwner } labels(first: 10) { nodes { name } } statusCheckRollup { state contexts(first: 100) { nodes { __typename ... on CheckRun { name status conclusion startedAt completedAt detailsUrl checkSuite { workflowRun { databaseId workflow { name } } } } ... on StatusContext { context state targetUrl } } } }"#
    };
}

const GRAPHQL: &str = concat!(
    "query($q: String!) { rateLimit { cost remaining resetAt } search(query: $q, type: ISSUE, first: 50) { nodes { ... on PullRequest { ",
    pr_fields!(),
    " } } } }"
);

/// The conversations, asked only for the few PRs Copilot has reviewed.
const THREADS_GRAPHQL: &str = r#"query($ids: [ID!]!) { rateLimit { cost remaining resetAt } nodes(ids: $ids) { ... on PullRequest { id reviewThreads(first: 50) { nodes { id isResolved isOutdated comments(first: 1) { nodes { author { login } } } } } } } }"#;

/// At most this many PRs get their conversations looked up in one poll.
const MAX_THREAD_LOOKUPS: usize = 20;

const GH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Review {
    None,
    Approved,
    ChangesRequested,
    Required,
}

/// What an all-green, out-of-draft PR is waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiting {
    /// GitHub says a review is required and it has not been given.
    Reviewer,
    /// Approved: only the merge is left.
    Merge,
    /// No review is required: it is simply green.
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckState {
    Queued,
    Running,
    Passed,
    Failed,
    /// Stopped or superseded by a newer run: it says nothing about the code.
    Cancelled,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub workflow: Option<String>,
    pub state: CheckState,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub url: Option<String>,
    /// A commit status reported by another service (Chromatic, Jenkins), not a GitHub Actions check.
    /// It can sit "pending" for as long as it takes someone to act, so it never means CI is still busy.
    #[serde(default)]
    pub external: bool,
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
    pub cancelled: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(default)]
    pub created_at: String,
    pub checks: Vec<Check>,
    /// What GitHub knows about reviews and comments, and who wrote the PR.
    pub feedback: Feedback,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CopilotState {
    /// Copilot has not been asked to review (or its review was dismissed).
    #[default]
    None,
    /// A review is requested and has not arrived yet.
    Requested,
    Reviewed,
}

/// Whether the PR's branch can be merged into its base as it stands.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeState {
    /// GitHub has not worked it out yet (it does so lazily).
    #[default]
    Unknown,
    Clean,
    /// The base has moved on; merging it in is needed but nothing conflicts.
    Behind,
    Conflicting,
}

/// One conversation on the PR's diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    pub id: String,
    pub resolved: bool,
    pub outdated: bool,
    pub by_copilot: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feedback {
    /// You wrote this PR.
    pub mine: bool,
    /// You pasted this PR in: it is someone else's, but capcom treats it like yours (auto-fix starts off).
    #[serde(default)]
    pub pasted: bool,
    pub head: String,
    pub copilot: CopilotState,
    pub copilot_reviewed_at: Option<String>,
    pub threads: Vec<Thread>,
    #[serde(default)]
    pub merge: MergeState,
    /// What the repository's merge bot is set up to do (None when it has none, or we could not tell).
    #[serde(default)]
    pub kodiak: Option<crate::kodiak::Kodiak>,
}

pub fn is_copilot(login: &str) -> bool {
    login.to_ascii_lowercase().contains("copilot")
}

impl PullRequest {
    /// A PR capcom acts on: one you wrote, or one you pasted in.
    pub fn managed(&self) -> bool {
        self.feedback.mine || self.feedback.pasted
    }

    /// Unresolved conversations started by Copilot.
    pub fn copilot_open(&self) -> Vec<&Thread> {
        self.feedback.threads.iter().filter(|t| t.by_copilot && !t.resolved).collect()
    }

    /// Checks that really failed, not ones that were cancelled.
    pub fn real_failures(&self) -> Vec<&Check> {
        self.checks_in(CheckState::Failed)
    }

    /// Something is wrong with the PR that its author has to act on: a failing check or a merge conflict.
    pub fn is_red(&self) -> bool {
        !self.real_failures().is_empty() || self.feedback.merge == MergeState::Conflicting
    }

    pub fn has_label(&self, name: &str) -> bool {
        self.labels.iter().any(|l| l.eq_ignore_ascii_case(name))
    }

    /// The labels that turn automerge on in this PR's repo (none when it has no merge bot).
    pub fn automerge_labels(&self) -> &[String] {
        self.feedback.kodiak.as_ref().map_or(&[], |k| k.automerge.as_slice())
    }

    pub fn automerge_on(&self) -> bool {
        self.automerge_labels().iter().any(|l| self.has_label(l))
    }

    /// The merge bot keeps this PR's branch up to date, so capcom need not: it updates every PR in the repo,
    /// or this one has its update label. Without its config, a label that looks like one of its labels counts.
    pub fn bot_updates(&self) -> bool {
        match &self.feedback.kodiak {
            Some(k) => k.update_always || k.autoupdate.iter().any(|l| self.has_label(l)),
            None => self.labels.iter().any(|l| {
                let l = l.to_lowercase();
                l.contains("autoupdate") || l.contains("automerge")
            }),
        }
    }

    /// Why a PR would not merge right now, in a few words, or None when nothing is in the way.
    pub fn merge_blocker(&self) -> Option<String> {
        let blocking = self.labels.iter().find(|l| {
            let l = l.to_lowercase().replace(['\u{2019}', '-'], "'").replace(' ', "");
            l.contains("donotmerge") || l.contains("don'tmerge") || l.contains("dontmerge")
        });
        if self.is_draft {
            Some("it is a draft".into())
        } else if let Some(label) = blocking {
            Some(format!("it is labelled \"{label}\""))
        } else if self.feedback.merge == MergeState::Conflicting {
            Some("it has merge conflicts".into())
        } else if !self.real_failures().is_empty() {
            Some("a check is failing".into())
        } else if self.checks.iter().any(|c| matches!(c.state, CheckState::Running | CheckState::Queued)) {
            Some("checks are still running".into())
        } else if self.review == Review::ChangesRequested {
            Some("changes were requested".into())
        } else if self.review == Review::Required {
            Some("it needs an approval".into())
        } else if self.feedback.merge == MergeState::Behind {
            Some("it is behind the base branch".into())
        } else {
            None
        }
    }

    /// How much the PR needs its author, most first: 0 red (a failing check or a conflict), 1 something
    /// to answer (Copilot's comments, changes requested), 2 waiting for a reviewer, 3 ready to merge,
    /// 4 checks running, 5 nothing to do, 6 a draft with nothing to do.
    pub fn urgency(&self) -> u8 {
        if self.is_red() {
            0
        } else if !self.copilot_open().is_empty() || self.review == Review::ChangesRequested {
            1
        } else {
            match self.waiting_for() {
                Some(Waiting::Reviewer) => 2,
                Some(Waiting::Merge) => 3,
                _ if self.is_busy() => 4,
                _ if self.is_draft => 6,
                _ => 5,
            }
        }
    }

    /// No GitHub Actions check is still running or queued. A status that another service leaves
    /// pending (such as a visual review waiting for someone to accept it) does not count.
    pub fn settled(&self) -> bool {
        !self.is_busy()
    }

    /// Worth polling quickly: an Actions check is running or queued.
    pub fn is_busy(&self) -> bool {
        self.checks.iter().any(|c| !c.external && matches!(c.state, CheckState::Running | CheckState::Queued))
    }
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
                CheckState::Cancelled => c.cancelled += 1,
            }
        }
        c
    }

    /// A PR that is out of draft with every check passed (skipped ones do not count against it), and
    /// what it is now waiting for. None for drafts, PRs with changes requested, and any that still has
    /// a check running, queued or failed, or no passing check at all.
    pub fn waiting_for(&self) -> Option<Waiting> {
        let c = self.counts();
        if self.is_draft || c.failed + c.cancelled + c.running + c.queued > 0 || c.passed == 0 {
            return None;
        }
        match self.review {
            Review::Required => Some(Waiting::Reviewer),
            Review::Approved => Some(Waiting::Merge),
            Review::None => Some(Waiting::Nothing),
            Review::ChangesRequested => None,
        }
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
            CheckState::Cancelled,
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
            "FAILURE" | "TIMED_OUT" | "STARTUP_FAILURE" | "ACTION_REQUIRED" => CheckState::Failed,
            "CANCELLED" | "STALE" => CheckState::Cancelled,
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
            external: false,
        }),
        "StatusContext" => Some(Check {
            name: text(ctx, "/context")?,
            workflow: None,
            state: status_state(&text(ctx, "/state").unwrap_or_default()),
            started_at: None,
            completed_at: None,
            url: text(ctx, "/targetUrl"),
            external: true,
        }),
        _ => None,
    }
}

/// The checks of one list of contexts. A re-run starts a newer run of the workflow, and a check of
/// the older run that the new one has not reached yet is not a result any more: GitHub shows it pending.
fn superseded_as_waiting(contexts: &[Value]) -> Vec<Check> {
    let run_id = |c: &Value| c.pointer("/checkSuite/workflowRun/databaseId").and_then(Value::as_u64);
    let workflow = |c: &Value| text(c, "/checkSuite/workflowRun/workflow/name");
    let newest = |name: &Option<String>| contexts.iter().filter(|c| &workflow(c) == name).filter_map(run_id).max();
    contexts
        .iter()
        .filter_map(|c| {
            let mut check = parse_check(c)?;
            let superseded = run_id(c).is_some_and(|id| newest(&workflow(c)).is_some_and(|n| n > id));
            let replaced = || contexts.iter().any(|o| workflow(o) == workflow(c) && text(o, "/name") == text(c, "/name") && run_id(o) > run_id(c));
            if superseded && replaced() {
                return None;
            }
            if superseded && !matches!(check.state, CheckState::Passed | CheckState::Skipped) {
                check.state = CheckState::Queued;
                check.started_at = None;
                check.completed_at = None;
            }
            Some(check)
        })
        .collect()
}

/// A re-run leaves the older run of a check in the list beside the new one; GitHub shows only the newest,
/// so an old failure must not stay red once the same check has passed. A run not started yet is the newest.
fn latest_runs(checks: Vec<Check>) -> Vec<Check> {
    let age = |c: &Check| (c.started_at.is_none(), c.started_at.clone());
    let mut kept: Vec<Check> = Vec::with_capacity(checks.len());
    for check in checks {
        match kept.iter().position(|k| k.label() == check.label() && k.external == check.external) {
            Some(i) if age(&check) >= age(&kept[i]) => kept[i] = check,
            Some(_) => {}
            None => kept.push(check),
        }
    }
    kept
}

fn parse_threads(node: &Value) -> Vec<Thread> {
    let nodes = node.pointer("/reviewThreads/nodes").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    nodes
        .iter()
        .filter_map(|t| {
            Some(Thread {
                id: text(t, "/id")?,
                resolved: t.get("isResolved").and_then(Value::as_bool).unwrap_or(false),
                outdated: t.get("isOutdated").and_then(Value::as_bool).unwrap_or(false),
                by_copilot: text(t, "/comments/nodes/0/author/login").is_some_and(|l| is_copilot(&l)),
            })
        })
        .collect()
}

fn parse_feedback(node: &Value) -> Feedback {
    let nodes = |pointer: &str| node.pointer(pointer).and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let requested = nodes("/reviewRequests/nodes").iter().any(|n| text(n, "/requestedReviewer/login").is_some_and(|l| is_copilot(&l)));
    let reviewed_at = nodes("/latestReviews/nodes")
        .iter()
        .filter(|n| text(n, "/author/login").is_some_and(|l| is_copilot(&l)))
        .filter_map(|n| text(n, "/submittedAt"))
        .max();
    let threads = parse_threads(node);
    Feedback {
        mine: node.get("viewerDidAuthor").and_then(Value::as_bool).unwrap_or(false),
        pasted: false,
        head: text(node, "/headRefOid").unwrap_or_default(),
        copilot: if requested {
            CopilotState::Requested
        } else if reviewed_at.is_some() {
            CopilotState::Reviewed
        } else {
            CopilotState::None
        },
        copilot_reviewed_at: reviewed_at,
        threads,
        merge: match (text(node, "/mergeable").as_deref(), text(node, "/mergeStateStatus").as_deref()) {
            (Some("CONFLICTING"), _) | (_, Some("DIRTY")) => MergeState::Conflicting,
            (_, Some("BEHIND")) => MergeState::Behind,
            (Some("MERGEABLE"), _) | (_, Some("CLEAN" | "UNSTABLE" | "HAS_HOOKS" | "BLOCKED")) => MergeState::Clean,
            _ => MergeState::Unknown,
        },
        kodiak: None,
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
        .map(|c| latest_runs(superseded_as_waiting(c)))
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
        created_at: text(node, "/createdAt").unwrap_or_default(),
        checks,
        feedback: parse_feedback(node),
    })
}

/// What GitHub says is left of the hourly GraphQL budget, from the `rateLimit` field of a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    /// Points the last poll cost.
    pub cost: u32,
    pub remaining: u32,
    pub reset_at: DateTime<Utc>,
}

fn parse_rate(value: &Value) -> Option<Rate> {
    let limit = value.pointer("/data/rateLimit")?;
    Some(Rate {
        cost: limit.get("cost")?.as_u64()? as u32,
        remaining: limit.get("remaining")?.as_u64()? as u32,
        reset_at: parse_time(limit.get("resetAt")?.as_str()?)?,
    })
}

/// The same from the headers of any GraphQL answer (`gh api -i`), which GitHub sends even when the
/// budget is spent.
pub fn parse_rate_headers(output: &str) -> Option<Rate> {
    let (mut remaining, mut reset) = (None, None);
    for line in output.lines().take_while(|l| !l.trim().is_empty()) {
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("x-ratelimit-remaining:") {
            remaining = v.trim().parse::<u32>().ok();
        } else if let Some(v) = lower.strip_prefix("x-ratelimit-reset:") {
            reset = v.trim().parse::<i64>().ok().and_then(|t| DateTime::from_timestamp(t, 0));
        }
    }
    Some(Rate { cost: 1, remaining: remaining?, reset_at: reset? })
}

/// How long to wait before the next poll so the budget lasts until it resets: never faster than
/// `base`, spread out when the budget is running low, and until the reset when it is nearly gone.
pub fn throttle(base: Duration, rate: &Rate, now: DateTime<Utc>) -> Duration {
    let to_reset = (rate.reset_at - now).num_seconds().max(0) as u64;
    let cost = u64::from(rate.cost.max(1));
    let remaining = u64::from(rate.remaining);
    if remaining < cost * 2 {
        return base.max(Duration::from_secs((to_reset + 5).min(3600)));
    }
    let calls_left = remaining / cost;
    base.max(Duration::from_secs(to_reset * 3 / 2 / calls_left))
}

/// One page of PRs: each with its GraphQL id (to look up its conversations), and the budget left.
pub struct Page {
    pub prs: Vec<(Option<String>, PullRequest)>,
    pub rate: Option<Rate>,
}

pub fn parse_page(json: &str) -> Result<Page> {
    let value: Value = serde_json::from_str(json).context("parsing gh output")?;
    let Some(nodes) = value.pointer("/data/search/nodes").and_then(Value::as_array) else {
        let message = text(&value, "/errors/0/message")
            .unwrap_or_else(|| "unexpected response from GitHub".to_string());
        bail!("{message}");
    };
    let prs = nodes.iter().filter_map(|n| parse_pr(n).map(|pr| (text(n, "/id"), pr))).collect();
    Ok(Page { prs, rate: parse_rate(&value) })
}

/// The owner, repository and number of a pull request link (`…/pull/12`, also with `/files` after it).
pub fn pr_ref(url: &str) -> Option<(String, String, u64)> {
    let rest = url.trim().strip_prefix("https://github.com/")?;
    let mut parts = rest.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    if parts.next()? != "pull" {
        return None;
    }
    let number: u64 = parts.next()?.parse().ok()?;
    let valid = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    (valid(owner) && valid(name) && number > 0).then(|| (owner.to_string(), name.to_string(), number))
}

/// One query for several PRs by number; the names and numbers travel as variables, never in the text.
fn lookup_query(refs: &[(String, String, u64)]) -> (String, Value) {
    let mut declared = Vec::new();
    let mut body = String::new();
    let mut vars = serde_json::Map::new();
    for (i, (owner, name, number)) in refs.iter().enumerate() {
        declared.push(format!("$o{i}: String!, $r{i}: String!, $n{i}: Int!"));
        body.push_str(&format!(" p{i}: repository(owner: $o{i}, name: $r{i}) {{ pullRequest(number: $n{i}) {{ state {} }} }}", pr_fields!()));
        vars.insert(format!("o{i}"), Value::from(owner.as_str()));
        vars.insert(format!("r{i}"), Value::from(name.as_str()));
        vars.insert(format!("n{i}"), Value::from(*number));
    }
    (format!("query({}) {{ rateLimit {{ cost remaining resetAt }}{body} }}", declared.join(", ")), Value::Object(vars))
}

/// The answer to `lookup_query`: for each PR asked about, the PR if it is open (marked as pasted), else
/// None (merged, closed, or not found).
fn parse_lookup(json: &str, count: usize) -> Result<(Vec<Option<(Option<String>, PullRequest)>>, Option<Rate>)> {
    let value: Value = serde_json::from_str(json).context("parsing gh output")?;
    let Some(data) = value.get("data").filter(|d| d.is_object()) else {
        let message = text(&value, "/errors/0/message").unwrap_or_else(|| "unexpected response from GitHub".to_string());
        bail!("{message}");
    };
    let found = (0..count)
        .map(|i| {
            let node = data.pointer(&format!("/p{i}/pullRequest")).filter(|n| text(n, "/state").as_deref() == Some("OPEN"))?;
            let mut pr = parse_pr(node)?;
            pr.feedback.pasted = true;
            Some((text(node, "/id"), pr))
        })
        .collect();
    Ok((found, parse_rate(&value)))
}

#[cfg(test)]
pub fn parse(json: &str) -> Result<Vec<PullRequest>> {
    Ok(parse_page(json)?.prs.into_iter().map(|(_, pr)| pr).collect())
}

/// The conversations of each PR in a `nodes(ids: …)` answer, keyed by the PR's id.
pub fn parse_threads_answer(json: &str) -> Result<(std::collections::HashMap<String, Vec<Thread>>, Option<Rate>)> {
    let value: Value = serde_json::from_str(json).context("parsing gh output")?;
    let Some(nodes) = value.pointer("/data/nodes").and_then(Value::as_array) else {
        let message = text(&value, "/errors/0/message")
            .unwrap_or_else(|| "unexpected response from GitHub".to_string());
        bail!("{message}");
    };
    let found = nodes.iter().filter_map(|n| Some((text(n, "/id")?, parse_threads(n)))).collect();
    Ok((found, parse_rate(&value)))
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

    /// A manual refresh: skip any "recent enough" shortcut.
    fn fetch_forced(&self) -> Result<Vec<PullRequest>> {
        self.fetch()
    }

    /// The GraphQL budget as of the last call, when this source knows it.
    fn rate(&self) -> Option<Rate> {
        None
    }
}

/// Shares one fetch between every open copy through the on-disk cache.
pub struct SharedSource {
    pub inner: Arc<dyn PrSource>,
    pub cache: Cache,
    pub name: String,
    pub cadence: Cadence,
}

impl SharedSource {
    pub fn new(inner: Arc<dyn PrSource>, cache: Cache, query: &str, cadence: Cadence) -> SharedSource {
        SharedSource { inner, cache, name: format!("prs-{}", cache::key_of(&[query])), cadence }
    }

    fn get(&self, force: bool) -> Result<Vec<PullRequest>> {
        let cadence = self.cadence;
        self.cache.shared(
            &self.name,
            Utc::now,
            move |prs: &Vec<PullRequest>| cadence.interval(prs),
            force,
            || if force { self.inner.fetch_forced() } else { self.inner.fetch() },
        )
    }
}

impl PrSource for SharedSource {
    fn rate(&self) -> Option<Rate> {
        self.inner.rate()
    }

    fn fetch(&self) -> Result<Vec<PullRequest>> {
        self.get(false)
    }

    fn fetch_forced(&self) -> Result<Vec<PullRequest>> {
        self.get(true)
    }
}

pub struct GhSource {
    pub query: String,
    pub rate: Mutex<Option<Rate>>,
    /// Each repo's merge bot settings, looked up now and then.
    kodiak: crate::kodiak::Cache,
}

impl GhSource {
    pub fn new(query: String) -> GhSource {
        GhSource { query, rate: Mutex::new(None), kodiak: crate::kodiak::Cache::default() }
    }

    fn graphql(&self, query: &str, vars: &Value) -> Result<String> {
        capcom::github::graphql(query, vars)
    }

    fn fetch_all(&self) -> Result<Vec<PullRequest>> {
        let page = parse_page(&self.graphql(GRAPHQL, &serde_json::json!({ "q": self.query }))?)?;
        let mut rate = page.rate;
        let mut prs = page.prs;
        self.enrich(&mut prs, &mut rate)?;
        *self.rate.lock().expect("rate lock") = rate;
        Ok(prs.into_iter().map(|(_, pr)| pr).collect())
    }

    /// Add what the list query leaves out: the conversations of Copilot-reviewed PRs, and each repo's
    /// merge bot settings.
    fn enrich(&self, prs: &mut Vec<(Option<String>, PullRequest)>, rate: &mut Option<Rate>) -> Result<()> {
        let reviewed: Vec<String> = prs
            .iter()
            .filter(|(_, pr)| pr.feedback.copilot == CopilotState::Reviewed)
            .filter_map(|(id, _)| id.clone())
            .take(MAX_THREAD_LOOKUPS)
            .collect();
        if !reviewed.is_empty() {
            let vars = serde_json::json!({ "ids": reviewed });
            // the list is still worth showing without the conversations, so only a spent budget stops it
            match self.graphql(THREADS_GRAPHQL, &vars).and_then(|json| parse_threads_answer(&json)) {
                Ok((threads, more)) => {
                    merge_threads(prs, &threads);
                    if let (Some(first), Some(second)) = (rate.as_mut(), more) {
                        first.cost += second.cost;
                        first.remaining = second.remaining;
                    }
                }
                Err(e) if is_rate_limited(&format!("{e:#}")) => return Err(e),
                Err(_) => {}
            }
        }
        let mut repos: Vec<String> = prs.iter().filter(|(_, pr)| pr.managed()).map(|(_, pr)| pr.repo.clone()).collect();
        repos.sort();
        repos.dedup();
        apply_kodiak(prs, &self.kodiak.lookup(&repos, &crate::kodiak::fetch));
        Ok(())
    }

    /// The pasted PRs, looked up by number: those that are still open, each marked as pasted.
    pub fn fetch_pasted(&self, urls: &[String]) -> Result<Vec<PullRequest>> {
        let refs: Vec<(String, String, u64)> = urls.iter().filter_map(|u| pr_ref(u)).collect();
        if refs.is_empty() {
            return Ok(Vec::new());
        }
        let (query, vars) = lookup_query(&refs);
        let result = self.graphql(&query, &vars).and_then(|json| parse_lookup(&json, refs.len()));
        let (found, mut rate) = match result {
            Err(e) if is_rate_limited(&format!("{e:#}")) => return Err(self.limited()),
            other => other?,
        };
        let mut prs: Vec<(Option<String>, PullRequest)> = found.into_iter().flatten().collect();
        match self.enrich(&mut prs, &mut rate) {
            Err(e) if is_rate_limited(&format!("{e:#}")) => return Err(self.limited()),
            other => other?,
        }
        Ok(prs.into_iter().map(|(_, pr)| pr).collect())
    }

    /// The budget is spent: find out when it comes back, remember it, and say so.
    fn limited(&self) -> anyhow::Error {
        let from_headers = capcom::github::graphql_budget().and_then(|b| DateTime::from_timestamp(b.reset, 0));
        let reset_at = from_headers.or_else(|| {
            // the direct route was not used, so ask for the headers through gh
            let mut cmd = Command::new("gh");
            cmd.args(["api", "-i", "graphql", "-f", "query={rateLimit{cost}}"]);
            run_with_timeout(cmd, GH_TIMEOUT)
                .ok()
                .flatten()
                .and_then(|out| parse_rate_headers(&String::from_utf8_lossy(&out.stdout)))
                .map(|r| r.reset_at)
        });
        let reset_at = reset_at.unwrap_or_else(|| Utc::now() + chrono::Duration::minutes(10));
        let cost = self.rate.lock().expect("rate lock").map_or(1, |r| r.cost);
        *self.rate.lock().expect("rate lock") = Some(Rate { cost, remaining: 0, reset_at });
        anyhow::anyhow!(
            "GitHub's hourly GraphQL budget is used up; capcom waits until {}",
            reset_at.with_timezone(&chrono::Local).format("%H:%M")
        )
    }
}

/// Give each PR the merge bot settings of its repo.
fn apply_kodiak(prs: &mut [(Option<String>, PullRequest)], configs: &std::collections::HashMap<String, Option<crate::kodiak::Kodiak>>) {
    for (_, pr) in prs.iter_mut() {
        if let Some(Some(config)) = configs.get(&pr.repo) {
            pr.feedback.kodiak = Some(config.clone());
        }
    }
}

fn merge_threads(prs: &mut [(Option<String>, PullRequest)], threads: &std::collections::HashMap<String, Vec<Thread>>) {
    for (id, pr) in prs.iter_mut() {
        if let Some(found) = id.as_ref().and_then(|id| threads.get(id)) {
            pr.feedback.threads = found.clone();
        }
    }
}

fn is_rate_limited(message: &str) -> bool {
    message.to_ascii_lowercase().contains("rate limit")
}

impl PrSource for GhSource {
    fn rate(&self) -> Option<Rate> {
        *self.rate.lock().expect("rate lock")
    }

    fn fetch(&self) -> Result<Vec<PullRequest>> {
        match self.fetch_all() {
            Err(e) if is_rate_limited(&format!("{e:#}")) => Err(self.limited()),
            other => other,
        }
    }
}

/// The PRs pasted into the panel, shared between the board and the feed. The feed notes the ones GitHub
/// says are no longer open (merged, closed, gone) in `gone` and drops them, so the board can say so.
#[derive(Debug, Default)]
pub struct Pasted {
    pub urls: Vec<String>,
    pub gone: Vec<String>,
}

/// A source that adds the pasted PRs to what `inner` lists. Each copy of capcom has its own, so they are
/// looked up here and never through the cache the copies share.
pub struct WithPasted {
    pub inner: Arc<dyn PrSource>,
    pub gh: Arc<GhSource>,
    pub pasted: Arc<Mutex<Pasted>>,
}

impl WithPasted {
    fn add(&self, mut prs: Vec<PullRequest>) -> Result<Vec<PullRequest>> {
        let urls = self.pasted.lock().expect("pasted lock").urls.clone();
        if urls.is_empty() {
            return Ok(prs);
        }
        let extra = self.gh.fetch_pasted(&urls)?;
        let open = |url: &str| extra.iter().any(|p| p.url.trim_end_matches('/').eq_ignore_ascii_case(url.trim_end_matches('/')));
        let mut shared = self.pasted.lock().expect("pasted lock");
        let gone: Vec<String> = urls.iter().filter(|u| !open(u)).cloned().collect();
        shared.urls.retain(|u| !gone.contains(u));
        shared.gone.extend(gone);
        prs.retain(|p| !extra.iter().any(|e| e.url.eq_ignore_ascii_case(&p.url)));
        prs.extend(extra);
        Ok(prs)
    }
}

impl PrSource for WithPasted {
    fn rate(&self) -> Option<Rate> {
        self.inner.rate()
    }

    fn fetch(&self) -> Result<Vec<PullRequest>> {
        self.add(self.inner.fetch()?)
    }

    fn fetch_forced(&self) -> Result<Vec<PullRequest>> {
        self.add(self.inner.fetch_forced()?)
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
            Ok(prs) => self.interval(prs),
            _ => self.idle,
        }
    }

    pub fn interval(&self, prs: &[PullRequest]) -> Duration {
        if prs.iter().any(PullRequest::is_busy) {
            self.busy
        } else {
            self.idle
        }
    }
}

pub enum PrMsg {
    Started,
    Result(Result<Vec<PullRequest>, String>),
}

/// Fetch in a background thread. Send on the returned sender to refresh right away.
#[cfg(test)]
pub fn spawn(source: Arc<dyn PrSource>, cadence: Cadence) -> (Receiver<PrMsg>, Sender<()>) {
    spawn_with(source, cadence, crate::attention::Attention::new())
}

/// Like `spawn`, and polls more slowly while `attention` says nobody is looking.
pub fn spawn_with(
    source: Arc<dyn PrSource>,
    cadence: Cadence,
    attention: Arc<crate::attention::Attention>,
) -> (Receiver<PrMsg>, Sender<()>) {
    let (tx, rx) = mpsc::channel();
    let (wake_tx, wake_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut forced = false;
        loop {
        if tx.send(PrMsg::Started).is_err() {
            break;
        }
        let result = if forced { source.fetch_forced() } else { source.fetch() }.map_err(|e| format!("{e:#}"));
        let wait = match source.rate() {
            Some(rate) => throttle(cadence.next(&result), &rate, Utc::now()),
            None => cadence.next(&result),
        };
        let wait = attention.wait(wait);
        if tx.send(PrMsg::Result(result)).is_err() {
            break;
        }
        match wake_rx.recv_timeout(wait) {
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => forced = false,
            Ok(()) => forced = true,
        }
        while wake_rx.try_recv().is_ok() {}
        }
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
        assert!(a.is_busy());
        assert_eq!(build.url.as_deref(), Some("https://github.com/acme/widgets/actions/runs/101/job/7"));
        assert_eq!(lint.url, None, "a check without a link has none");
        let legacy = a.checks.iter().find(|c| c.name == "ci/legacy").unwrap();
        assert_eq!(legacy.url.as_deref(), Some("https://ci.example/legacy/1"));
        let b = &prs[1];
        assert!(b.is_draft && b.checks.is_empty() && b.review == Review::None);
        assert!(!b.is_busy());
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

    struct Counting {
        calls: AtomicUsize,
        forced: AtomicUsize,
    }

    impl PrSource for Counting {
        fn fetch(&self) -> Result<Vec<PullRequest>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            parse(FIXTURE)
        }

        fn fetch_forced(&self) -> Result<Vec<PullRequest>> {
            self.forced.fetch_add(1, Ordering::SeqCst);
            self.fetch()
        }
    }

    fn counting() -> Arc<Counting> {
        Arc::new(Counting { calls: AtomicUsize::new(0), forced: AtomicUsize::new(0) })
    }

    #[test]
    fn two_copies_with_the_same_query_share_one_github_fetch_and_other_queries_do_not() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = || Cache::open(dir.path().to_path_buf()).unwrap();
        let cadence = Cadence::default();
        let (a, b, c) = (counting(), counting(), counting());
        let first = SharedSource::new(a.clone(), cache(), "is:pr author:@me", cadence);
        let second = SharedSource::new(b.clone(), cache(), "is:pr author:@me", cadence);
        let other = SharedSource::new(c.clone(), cache(), "is:pr author:someone", cadence);
        let got = first.fetch().unwrap();
        assert_eq!(second.fetch().unwrap(), got, "the second copy gets the first copy's data");
        other.fetch().unwrap();
        assert_eq!(a.calls.load(Ordering::SeqCst), 1);
        assert_eq!(b.calls.load(Ordering::SeqCst), 0, "the second copy never called GitHub");
        assert_eq!(c.calls.load(Ordering::SeqCst), 1, "a different query is fetched on its own");
    }

    #[test]
    fn a_manual_refresh_reaches_the_source_as_a_forced_fetch() {
        let dir = tempfile::TempDir::new().unwrap();
        let inner = counting();
        let cadence = Cadence { busy: Duration::from_secs(5), idle: Duration::from_secs(30) };
        let shared = SharedSource::new(inner.clone(), Cache::open(dir.path().to_path_buf()).unwrap(), "q", cadence);
        shared.fetch().unwrap();
        std::thread::sleep(Duration::from_millis(3100));
        shared.fetch_forced().unwrap();
        assert_eq!((inner.calls.load(Ordering::SeqCst), inner.forced.load(Ordering::SeqCst)), (2, 1));
    }

    #[test]
    fn the_poller_forces_the_fetch_after_a_wake_but_not_on_the_timer() {
        let inner = counting();
        let cadence = Cadence { busy: Duration::from_millis(30), idle: Duration::from_millis(30) };
        let (rx, wake) = spawn(inner.clone(), cadence);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(inner.forced.load(Ordering::SeqCst), 0, "timer fetches are not forced");
        let (_, _) = (&rx, ());
        wake.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(inner.forced.load(Ordering::SeqCst) >= 1, "a wake is a manual refresh");
    }

    fn green_pr(review: Review) -> PullRequest {
        let check = |state| Check { name: "x".into(), workflow: None, state, started_at: None, completed_at: None, url: None, external: false };
        PullRequest {
            feedback: Default::default(),
            repo: "acme/api".into(),
            number: 1,
            title: "t".into(),
            url: "https://github.com/acme/api/pull/1".into(),
            is_draft: false,
            labels: vec![],
            review,
            comments: 0,
            updated_at: "2026-10-08T01:00:00Z".into(),
            created_at: "2026-10-08T01:00:00Z".into(),
            checks: vec![check(CheckState::Passed), check(CheckState::Passed), check(CheckState::Skipped)],
        }
    }

    #[test]
    fn an_all_green_pr_out_of_draft_says_what_it_is_waiting_for() {
        assert_eq!(green_pr(Review::Required).waiting_for(), Some(Waiting::Reviewer));
        assert_eq!(green_pr(Review::Approved).waiting_for(), Some(Waiting::Merge));
        assert_eq!(green_pr(Review::None).waiting_for(), Some(Waiting::Nothing));
    }

    #[test]
    fn drafts_changes_requested_and_unfinished_or_failing_checks_are_never_waiting() {
        let mut draft = green_pr(Review::Required);
        draft.is_draft = true;
        assert_eq!(draft.waiting_for(), None);
        assert_eq!(green_pr(Review::ChangesRequested).waiting_for(), None);
        for state in [CheckState::Failed, CheckState::Running, CheckState::Queued] {
            let mut pr = green_pr(Review::Required);
            pr.checks[2].state = state;
            assert_eq!(pr.waiting_for(), None, "{state:?}");
        }
        let mut none_passed = green_pr(Review::Required);
        none_passed.checks.iter_mut().for_each(|c| c.state = CheckState::Skipped);
        assert_eq!(none_passed.waiting_for(), None, "skipped only is not green");
        let mut no_checks = green_pr(Review::Required);
        no_checks.checks.clear();
        assert_eq!(no_checks.waiting_for(), None);
    }

    #[test]
    fn a_pull_request_link_names_its_repository_and_number_and_nothing_else_is_one() {
        assert_eq!(pr_ref("https://github.com/acme/api/pull/12"), Some(("acme".into(), "api".into(), 12)));
        assert_eq!(pr_ref("  https://github.com/acme/api/pull/12/files\n"), Some(("acme".into(), "api".into(), 12)));
        for bad in ["https://github.com/acme/api/actions/runs/12", "https://github.com/acme/api/pull/x", "https://github.com/acme/api/pull/0", "http://github.com/acme/api/pull/1", "https://github.com/ac me/api/pull/1", "https://github.com/acme/api/issues/1", "nonsense"] {
            assert_eq!(pr_ref(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_lookup_sends_names_and_numbers_as_variables_and_reads_only_open_prs_as_pasted() {
        let refs = vec![("acme".to_string(), "api".to_string(), 1), ("acme".to_string(), "web".to_string(), 2), ("acme".to_string(), "gone".to_string(), 3)];
        let (query, vars) = lookup_query(&refs);
        assert!(!query.contains("acme") && !query.contains("web"), "nothing typed is spliced into the query text");
        assert_eq!((vars["o1"].as_str(), vars["r1"].as_str(), vars["n1"].as_u64()), (Some("acme"), Some("web"), Some(2)));
        let open = r#"{"state":"OPEN","id":"X1","number":1,"title":"One","url":"https://github.com/acme/api/pull/1","isDraft":false,"viewerDidAuthor":false,"repository":{"nameWithOwner":"acme/api"}}"#;
        let merged = open.replace("OPEN", "MERGED");
        let json = format!(r#"{{"data":{{"rateLimit":{{"cost":1,"remaining":4990,"resetAt":"2026-10-12T01:00:00Z"}},"p0":{{"pullRequest":{open}}},"p1":{{"pullRequest":{merged}}},"p2":{{"pullRequest":null}}}}}}"#);
        let (found, rate) = parse_lookup(&json, 3).unwrap();
        let first = found[0].as_ref().unwrap();
        assert!(first.1.feedback.pasted && !first.1.feedback.mine && first.1.managed());
        assert!(found[1].is_none() && found[2].is_none(), "merged and not-found PRs are not listed");
        assert_eq!(rate.map(|r| r.remaining), Some(4990));
        assert!(parse_lookup(r#"{"errors":[{"message":"Bad credentials"}]}"#, 1).unwrap_err().to_string().contains("Bad credentials"));
    }

    #[test]
    fn a_check_that_was_rerun_counts_only_by_its_newest_run() {
        let run = |state, started: Option<&str>| Check { started_at: started.map(String::from), ..check_of("GreenPipeline", state, false) };
        let kept = latest_runs(vec![
            run(CheckState::Failed, Some("2026-10-12T18:23:02Z")),
            run(CheckState::Passed, Some("2026-10-12T18:24:50Z")),
            check_of("lint", CheckState::Passed, false),
        ]);
        assert_eq!(kept.iter().map(|c| (c.name.as_str(), c.state)).collect::<Vec<_>>(), vec![("GreenPipeline", CheckState::Passed), ("lint", CheckState::Passed)]);
        let older_last = latest_runs(vec![run(CheckState::Passed, Some("2026-10-12T18:24:50Z")), run(CheckState::Failed, Some("2026-10-12T18:23:02Z"))]);
        assert_eq!(older_last[0].state, CheckState::Passed, "the order GitHub lists them in does not matter");
        let queued = latest_runs(vec![run(CheckState::Failed, Some("2026-10-12T18:23:02Z")), run(CheckState::Queued, None)]);
        assert_eq!(queued[0].state, CheckState::Queued, "a re-run waiting to start replaces the old result");
    }

    #[test]
    fn a_failure_from_an_older_run_waits_for_the_rerun_to_reach_it() {
        let ctx = |name: &str, conclusion: &str, run: u64| {
            serde_json::json!({"__typename":"CheckRun","name":name,"status":"COMPLETED","conclusion":conclusion,
                "startedAt":"2026-10-12T18:00:00Z","checkSuite":{"workflowRun":{"databaseId":run,"workflow":{"name":"CI"}}}})
        };
        let state_of = |checks: &[Check], name: &str| checks.iter().find(|c| c.name == name).map(|c| c.state);
        let got = latest_runs(superseded_as_waiting(&[
            ctx("GreenPipeline", "FAILURE", 1),
            ctx("unit", "FAILURE", 1),
            ctx("unit", "SUCCESS", 2),
            ctx("lint", "SUCCESS", 1),
        ]));
        assert_eq!(state_of(&got, "GreenPipeline"), Some(CheckState::Queued), "the new run has not reached it: pending, not red");
        assert_eq!(state_of(&got, "unit"), Some(CheckState::Passed), "the new run's result wins");
        assert_eq!(state_of(&got, "lint"), Some(CheckState::Passed));
        let same_run = latest_runs(superseded_as_waiting(&[ctx("GreenPipeline", "FAILURE", 2), ctx("unit", "SUCCESS", 2)]));
        assert_eq!(state_of(&same_run, "GreenPipeline"), Some(CheckState::Failed), "a failure in the newest run is real");
    }

    #[test]
    fn a_pr_whose_checks_were_only_cancelled_is_not_red_and_not_counted_as_failed() {
        let pr = pr_with(vec![check_of("unit", CheckState::Cancelled, false), check_of("lint", CheckState::Passed, false)]);
        assert_eq!((pr.counts().failed, pr.counts().cancelled), (0, 1));
        assert!(!pr.is_red() && pr.real_failures().is_empty());
        assert!(pr.open_checks().is_empty(), "a superseded run is not a stage to watch");
        assert_eq!(pr.waiting_for(), None, "but nothing has passed in its place yet");
    }

    #[test]
    fn cancelled_and_action_required_checks_are_not_green() {
        assert_eq!(check_state("COMPLETED", "CANCELLED"), CheckState::Cancelled, "superseded, not broken");
        assert_eq!(check_state("COMPLETED", "ACTION_REQUIRED"), CheckState::Failed);
        assert_eq!(check_state("COMPLETED", "NEUTRAL"), CheckState::Skipped);
        assert_eq!(check_state("COMPLETED", "SKIPPED"), CheckState::Skipped);
    }

    fn pr_with(checks: Vec<Check>) -> PullRequest {
        let mut pr = parse(FIXTURE).unwrap().remove(1);
        pr.checks = checks;
        pr
    }

    fn check_of(name: &str, state: CheckState, external: bool) -> Check {
        Check { name: name.into(), workflow: None, state, started_at: None, completed_at: None, url: None, external }
    }

    #[test]
    fn a_pending_status_from_another_service_does_not_keep_ci_busy_but_a_queued_action_does() {
        let waiting_for_a_person = pr_with(vec![
            check_of("unit", CheckState::Failed, false),
            check_of("UI Tests", CheckState::Queued, true),
        ]);
        assert!(waiting_for_a_person.settled() && !waiting_for_a_person.is_busy());
        let queued = pr_with(vec![check_of("unit", CheckState::Failed, false), check_of("lint", CheckState::Queued, false)]);
        assert!(!queued.settled() && queued.is_busy());
        let parsed = parse(FIXTURE).unwrap();
        let legacy = parsed[0].checks.iter().find(|c| c.name == "ci/legacy").unwrap();
        assert!(legacy.external, "a StatusContext is another service's status");
        assert!(!parsed[0].checks.iter().find(|c| c.name == "build").unwrap().external);
    }

    #[test]
    fn the_list_query_leaves_out_the_costly_conversations_and_asks_for_the_budget() {
        assert!(!GRAPHQL.contains("reviewThreads"), "nested conversations multiplied the cost");
        assert!(GRAPHQL.contains("rateLimit") && GRAPHQL.contains("resetAt"));
        assert!(THREADS_GRAPHQL.contains("reviewThreads") && THREADS_GRAPHQL.contains("nodes(ids: $ids)"));
    }

    #[test]
    fn a_page_carries_each_prs_id_and_the_budget_left() {
        let json = r#"{"data":{"rateLimit":{"cost":3,"remaining":4900,"resetAt":"2026-10-09T06:00:00Z"},
          "search":{"nodes":[{"id":"PR_1","number":1,"title":"t","url":"https://github.com/acme/api/pull/1",
          "repository":{"nameWithOwner":"acme/api"}}]}}}"#;
        let page = parse_page(json).unwrap();
        assert_eq!(page.prs[0].0.as_deref(), Some("PR_1"));
        let rate = page.rate.unwrap();
        assert_eq!((rate.cost, rate.remaining), (3, 4900));
        assert_eq!(rate.reset_at.to_rfc3339(), "2026-10-09T06:00:00+00:00");
    }

    #[test]
    fn conversations_come_back_keyed_by_pr_and_know_who_started_them() {
        let json = r#"{"data":{"rateLimit":{"cost":1,"remaining":10,"resetAt":"2026-10-09T06:00:00Z"},"nodes":[
          {"id":"PR_1","reviewThreads":{"nodes":[
            {"id":"T1","isResolved":false,"isOutdated":false,"comments":{"nodes":[{"author":{"login":"copilot-pull-request-reviewer"}}]}},
            {"id":"T2","isResolved":true,"isOutdated":false,"comments":{"nodes":[{"author":{"login":"someone"}}]}}]}},
          null]}}"#;
        let (threads, rate) = parse_threads_answer(json).unwrap();
        let found = &threads["PR_1"];
        assert_eq!(found.len(), 2);
        assert!(found[0].by_copilot && !found[0].resolved && !found[1].by_copilot && found[1].resolved);
        assert_eq!(rate.unwrap().remaining, 10);
    }

    #[test]
    fn looked_up_conversations_are_attached_to_the_right_pr_and_others_are_left_alone() {
        let mut prs = vec![
            (Some("PR_1".to_string()), pr_with(vec![])),
            (Some("PR_2".to_string()), pr_with(vec![])),
            (None, pr_with(vec![])),
        ];
        let thread = Thread { id: "T1".into(), resolved: false, outdated: false, by_copilot: true };
        merge_threads(&mut prs, &[("PR_2".to_string(), vec![thread.clone()])].into_iter().collect());
        assert!(prs[0].1.feedback.threads.is_empty());
        assert_eq!(prs[1].1.feedback.threads, vec![thread]);
        assert!(prs[2].1.feedback.threads.is_empty());
    }

    #[test]
    fn merge_conflicts_and_a_branch_behind_its_base_are_read_from_the_query() {
        let merge = |mergeable: &str, status: &str| {
            let json = format!(r#"{{"number":1,"title":"t","url":"https://github.com/acme/api/pull/1","repository":{{"nameWithOwner":"acme/api"}},"mergeable":"{mergeable}","mergeStateStatus":"{status}"}}"#);
            parse_feedback(&serde_json::from_str::<Value>(&json).unwrap()).merge
        };
        assert_eq!(merge("CONFLICTING", "DIRTY"), MergeState::Conflicting);
        assert_eq!(merge("MERGEABLE", "DIRTY"), MergeState::Conflicting, "a dirty state is a conflict whatever else is said");
        assert_eq!(merge("MERGEABLE", "BEHIND"), MergeState::Behind);
        assert_eq!(merge("MERGEABLE", "CLEAN"), MergeState::Clean);
        assert_eq!(merge("MERGEABLE", "BLOCKED"), MergeState::Clean, "blocked on reviews is not a branch problem");
        assert_eq!(merge("UNKNOWN", "UNKNOWN"), MergeState::Unknown, "GitHub works it out lazily");
        assert!(GRAPHQL.contains("mergeable") && GRAPHQL.contains("mergeStateStatus"));
        let mut pr = pr_with(vec![]);
        assert!(!pr.is_red());
        pr.feedback.merge = MergeState::Conflicting;
        assert!(pr.is_red(), "a conflict is red even when every check is green");
        pr.feedback.merge = MergeState::Behind;
        assert!(!pr.is_red(), "behind is only a nudge");
    }

    #[test]
    fn urgency_ranks_what_needs_the_author_most_first() {
        let rank = |pr: &PullRequest| pr.urgency();
        let ok = |state| check_of("build", state, false);
        let mut red = pr_with(vec![ok(CheckState::Failed)]);
        assert_eq!(rank(&red), 0);
        red.is_draft = true;
        assert_eq!(rank(&red), 0, "a draft that is red is still red");
        let mut conflict = pr_with(vec![ok(CheckState::Passed)]);
        conflict.feedback.merge = MergeState::Conflicting;
        assert_eq!(rank(&conflict), 0, "a conflict is red");
        let mut comments = pr_with(vec![ok(CheckState::Passed)]);
        comments.feedback.copilot = CopilotState::Reviewed;
        comments.feedback.threads = vec![Thread { id: "t".into(), resolved: false, outdated: false, by_copilot: true }];
        assert_eq!(rank(&comments), 1);
        let mut changes = pr_with(vec![ok(CheckState::Passed)]);
        changes.review = Review::ChangesRequested;
        assert_eq!(rank(&changes), 1, "changes requested is something to answer");
        let mut waiting = pr_with(vec![ok(CheckState::Passed)]);
        waiting.review = Review::Required;
        waiting.is_draft = false;
        assert_eq!(rank(&waiting), 2);
        waiting.review = Review::Approved;
        assert_eq!(rank(&waiting), 3);
        let mut running = pr_with(vec![ok(CheckState::Running)]);
        running.is_draft = false;
        assert_eq!(rank(&running), 4);
        let mut quiet = pr_with(vec![ok(CheckState::Passed)]);
        quiet.is_draft = false;
        quiet.review = Review::None;
        assert_eq!(rank(&quiet), 5);
        quiet.is_draft = true;
        assert_eq!(rank(&quiet), 6);
        let pending_elsewhere = pr_with(vec![ok(CheckState::Passed), check_of("UI Tests", CheckState::Queued, true)]);
        assert_eq!(rank(&pending_elsewhere), 6, "a status waiting on a person is not 'running'");
    }

    fn kodiak_with(automerge: &[&str], autoupdate: &[&str], always: bool) -> crate::kodiak::Kodiak {
        crate::kodiak::Kodiak {
            automerge: automerge.iter().map(|s| s.to_string()).collect(),
            autoupdate: autoupdate.iter().map(|s| s.to_string()).collect(),
            update_always: always,
        }
    }

    #[test]
    fn automerge_is_on_when_the_pr_has_one_of_the_labels_its_repos_bot_reads() {
        let mut pr = pr_with(vec![]);
        assert!(pr.automerge_labels().is_empty() && !pr.automerge_on(), "no bot config, no automerge");
        pr.feedback.kodiak = Some(kodiak_with(&["automerge 🚀", "automerge"], &[], false));
        assert!(!pr.automerge_on());
        pr.labels = vec!["Automerge".into()];
        assert!(pr.automerge_on(), "a label matches whatever its case");
        pr.labels = vec!["automerge 🚀 extra".into(), "bug".into()];
        assert!(!pr.automerge_on(), "only the exact names count");
    }

    #[test]
    fn the_bot_updating_a_pr_comes_from_the_repos_config_not_from_how_a_label_looks() {
        let mut pr = pr_with(vec![]);
        pr.labels = vec!["autoupdate".into()];
        assert!(pr.bot_updates(), "no config: a label that looks like the bot's counts");
        pr.labels = vec![];
        assert!(!pr.bot_updates());
        pr.feedback.kodiak = Some(kodiak_with(&["automerge"], &["autoupdate"], true));
        assert!(pr.bot_updates(), "update.always: every PR");
        pr.feedback.kodiak = Some(kodiak_with(&["automerge 🚀"], &[], false));
        pr.labels = vec!["automerge 🚀".into()];
        assert!(!pr.bot_updates(), "this repo's bot does not update branches, whatever the label says");
        pr.feedback.kodiak = Some(kodiak_with(&["automerge"], &["autoupdate 🚀"], false));
        pr.labels = vec!["autoupdate 🚀".into()];
        assert!(pr.bot_updates(), "the label the repo names, emoji and all");
        pr.labels = vec!["autoupdate".into()];
        assert!(!pr.bot_updates(), "a different spelling is not that repo's label");
    }

    #[test]
    fn why_a_pr_would_not_merge_is_said_in_order_of_what_matters() {
        let mut pr = pr_with(vec![check_of("build", CheckState::Passed, false)]);
        pr.is_draft = false;
        pr.review = Review::Approved;
        assert_eq!(pr.merge_blocker(), None, "approved, green, not behind: nothing in the way");
        pr.review = Review::Required;
        assert_eq!(pr.merge_blocker().as_deref(), Some("it needs an approval"));
        pr.feedback.merge = MergeState::Behind;
        assert_eq!(pr.merge_blocker().as_deref(), Some("it needs an approval"), "approval comes before being behind");
        pr.review = Review::Approved;
        assert_eq!(pr.merge_blocker().as_deref(), Some("it is behind the base branch"));
        pr.checks.push(check_of("e2e", CheckState::Running, false));
        assert_eq!(pr.merge_blocker().as_deref(), Some("checks are still running"));
        pr.checks.push(check_of("lint", CheckState::Failed, false));
        assert_eq!(pr.merge_blocker().as_deref(), Some("a check is failing"));
        pr.feedback.merge = MergeState::Conflicting;
        assert_eq!(pr.merge_blocker().as_deref(), Some("it has merge conflicts"));
        pr.labels = vec!["Don't merge on a Friday".into()];
        assert_eq!(pr.merge_blocker().as_deref(), Some("it is labelled \"Don't merge on a Friday\""));
        pr.labels = vec!["DO NOT MERGE".into()];
        assert!(pr.merge_blocker().unwrap().contains("DO NOT MERGE"));
        pr.is_draft = true;
        assert_eq!(pr.merge_blocker().as_deref(), Some("it is a draft"), "a draft is the first thing in the way");
        let mut pending_elsewhere = pr_with(vec![check_of("UI Tests", CheckState::Queued, true)]);
        pending_elsewhere.is_draft = false;
        pending_elsewhere.review = Review::Approved;
        assert_eq!(pending_elsewhere.merge_blocker().as_deref(), Some("checks are still running"), "a pending status counts against merging, as GitHub counts it");
    }

    #[test]
    fn each_pr_gets_the_settings_of_its_own_repo() {
        let mut prs = vec![(None, pr_with(vec![])), (None, pr_with(vec![]))];
        prs[1].1.repo = "acme/web".into();
        let configs: std::collections::HashMap<String, Option<crate::kodiak::Kodiak>> =
            [("acme/widgets".to_string(), None), ("acme/web".to_string(), Some(kodiak_with(&["automerge"], &[], false)))].into_iter().collect();
        apply_kodiak(&mut prs, &configs);
        assert!(prs[0].1.feedback.kodiak.is_none() && prs[1].1.feedback.kodiak.is_some());
    }

    #[test]
    fn the_budget_headers_are_read_even_from_a_refused_call() {
        let out = "HTTP/2.0 200 OK\nX-Ratelimit-Limit: 5000\nX-Ratelimit-Remaining: 0\nX-Ratelimit-Reset: 1791525190\nX-Ratelimit-Resource: graphql\n\n{}";
        let rate = parse_rate_headers(out).unwrap();
        assert_eq!((rate.remaining, rate.reset_at.timestamp()), (0, 1791525190));
        assert!(parse_rate_headers("HTTP/2.0 500\n\n{}").is_none());
        assert!(is_rate_limited("gh failed: GraphQL: API rate limit already exceeded for user ID 1."));
        assert!(!is_rate_limited("gh failed: could not resolve host"));
    }

    #[test]
    fn polling_slows_down_to_make_the_budget_last_and_stops_when_it_is_spent() {
        let now = Utc.with_ymd_and_hms(2026, 10, 9, 5, 0, 0).unwrap();
        let base = Duration::from_secs(5);
        let rate = |cost, remaining, minutes| Rate { cost, remaining, reset_at: now + chrono::Duration::minutes(minutes) };
        assert_eq!(throttle(base, &rate(3, 4000, 30), now), base, "plenty left: no change");
        let low = throttle(base, &rate(3, 90, 30), now);
        assert!(low >= Duration::from_secs(90), "30 calls left for 30 minutes must not poll every 5 s: {low:?}");
        let spent = throttle(base, &rate(3, 0, 20), now);
        assert!(spent >= Duration::from_secs(20 * 60), "wait for the reset: {spent:?}");
        assert!(throttle(base, &rate(3, 0, 600), now) <= Duration::from_secs(3600), "never longer than an hour");
        assert_eq!(throttle(base, &rate(3, 0, -5), now), base.max(Duration::from_secs(5)), "a reset in the past waits only a moment");
    }

    #[test]
    fn parse_reads_who_wrote_the_pr_copilots_review_and_the_open_threads() {
        let json = r#"{"data":{"search":{"nodes":[{"number":7,"title":"t","url":"https://github.com/acme/api/pull/7",
          "isDraft":false,"viewerDidAuthor":true,"headRefOid":"abc123","updatedAt":"2026-10-09T01:00:00Z","reviewDecision":null,
          "comments":{"totalCount":0},"repository":{"nameWithOwner":"acme/api"},"labels":{"nodes":[]},
          "reviewRequests":{"nodes":[]},
          "latestReviews":{"nodes":[{"author":{"login":"copilot-pull-request-reviewer"},"state":"COMMENTED","submittedAt":"2026-10-09T02:46:40Z"},
                                    {"author":{"login":"sam"},"state":"APPROVED","submittedAt":"2026-10-09T03:20:05Z"}]},
          "reviewThreads":{"nodes":[
            {"id":"T1","isResolved":false,"isOutdated":false,"comments":{"nodes":[{"author":{"login":"copilot-pull-request-reviewer"}}]}},
            {"id":"T2","isResolved":true,"isOutdated":true,"comments":{"nodes":[{"author":{"login":"copilot-pull-request-reviewer"}}]}},
            {"id":"T3","isResolved":false,"isOutdated":false,"comments":{"nodes":[{"author":{"login":"sam"}}]}}]},
          "statusCheckRollup":{"state":"FAILURE","contexts":{"nodes":[
            {"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"CANCELLED"},
            {"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"FAILURE"}]}}}]}}}"#;
        let pr = parse(json).unwrap().remove(0);
        assert!(pr.feedback.mine);
        assert_eq!(pr.feedback.head, "abc123");
        assert_eq!(pr.feedback.copilot, CopilotState::Reviewed);
        assert_eq!(pr.feedback.copilot_reviewed_at.as_deref(), Some("2026-10-09T02:46:40Z"));
        assert_eq!(pr.copilot_open().iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["T1"]);
        assert_eq!(pr.real_failures().iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["test"], "a cancelled check is not a failure");
        assert!(pr.settled());
        let requested = json.replace(r#""reviewRequests":{"nodes":[]}"#, r#""reviewRequests":{"nodes":[{"requestedReviewer":{"__typename":"Bot","login":"copilot-pull-request-reviewer"}}]}"#);
        assert_eq!(parse(&requested).unwrap()[0].feedback.copilot, CopilotState::Requested);
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
