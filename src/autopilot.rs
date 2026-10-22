//! What is wrong with a PR that an agent could fix, shared by the TUI and the web service.
use crate::prs::{CopilotState, MergeState, PullRequest};

/// What to ask a PR's agent to fix, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixNeed {
    /// The state of the PR that is being asked about: the same state is never asked twice.
    pub fingerprint: String,
    pub why: String,
    /// Nothing is wrong but the branch being behind its base.
    pub only_behind: bool,
}

/// What is wrong with one of your PRs that an agent could fix: failing checks (once CI has
/// settled) and Copilot's unresolved comments (once its review is in), a merge conflict, or a
/// branch that is behind.
pub fn fix_needed(pr: &PullRequest) -> Option<FixNeed> {
    if !pr.managed() {
        return None;
    }
    let mut failing: Vec<String> = pr.fixable_failures().iter().map(|c| c.name.clone()).collect();
    let mut threads: Vec<String> =
        if pr.feedback.copilot == CopilotState::Requested { Vec::new() } else { pr.copilot_open().iter().map(|t| t.id.clone()).collect() };
    let conflicts = pr.feedback.merge == MergeState::Conflicting;
    // Behind the base: bring it up to date, but not while checks run (a push cancels them) and not on a
    // PR whose merge bot keeps it up to date already.
    let behind = pr.feedback.merge == MergeState::Behind && pr.settled() && !pr.bot_updates();
    if failing.is_empty() && threads.is_empty() && !conflicts && !behind {
        return None;
    }
    failing.sort();
    threads.sort();
    let mut why = Vec::new();
    if !failing.is_empty() {
        why.push(format!("{} failing check{}", failing.len(), if failing.len() == 1 { "" } else { "s" }));
    }
    if !threads.is_empty() {
        why.push(format!("{} unresolved Copilot comment{}", threads.len(), if threads.len() == 1 { "" } else { "s" }));
    }
    if conflicts {
        why.push("merge conflicts with the default branch".to_string());
    }
    if behind {
        why.push("the branch is behind the default branch".to_string());
    }
    let state = format!("{}|{}|{}", pr.feedback.head, failing.join(","), threads.join(","));
    // a conflict, or being behind, is its own state, so clearing it (or a new one) is a new ask; neither leaves the key as it was
    let fingerprint = if conflicts {
        format!("{state}|conflict")
    } else if behind {
        format!("{state}|behind")
    } else {
        state
    };
    Some(FixNeed { fingerprint, why: why.join(" and "), only_behind: behind && failing.is_empty() && threads.is_empty() })
}
