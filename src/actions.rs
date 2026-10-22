//! Things done to a pull request through GitHub (the `gh` program or its API), shared by the programs.
use std::process::Command;

pub fn gh_mark_ready(url: &str) -> Result<(), String> {
    gh_pr_ready(url, false)
}

/// Back to draft: `gh pr ready --undo` (GitHub only offers this through its GraphQL API, which gh speaks).
pub fn gh_mark_draft(url: &str) -> Result<(), String> {
    gh_pr_ready(url, true)
}

pub fn gh_pr_ready(url: &str, undo: bool) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("not a pull request link".into());
    }
    let mut cmd = Command::new("gh");
    cmd.args(["pr", "ready", url]);
    if undo {
        cmd.arg("--undo");
    }
    match crate::refresh::run_with_timeout(cmd, std::time::Duration::from_secs(30)) {
        Ok(Some(out)) if out.status.success() => Ok(()),
        Ok(Some(out)) => Err(String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("gh failed").to_string()),
        Ok(None) => Err("gh timed out".into()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn gh_request_copilot(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("not a pull request link".into());
    }
    let mut cmd = Command::new("gh");
    cmd.args(["pr", "edit", url, "--add-reviewer", "@copilot"]);
    match crate::refresh::run_with_timeout(cmd, std::time::Duration::from_secs(30)) {
        Ok(Some(out)) if out.status.success() => Ok(()),
        Ok(Some(out)) => Err(String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("gh failed").to_string()),
        Ok(None) => Err("gh timed out".into()),
        Err(e) => Err(e.to_string()),
    }
}

/// Put a label on a PR, or take it off.
pub fn gh_set_label(repo: &str, number: u64, label: &str, on: bool) -> Result<(), String> {
    let result = if on { crate::github::add_label(repo, number, label) } else { crate::github::remove_label(repo, number, label) };
    result.map_err(|e| format!("{e:#}"))
}

/// Bring a PR's branch up to date with its base through GitHub, as the "Update branch" button does.
pub fn gh_update_branch(repo: &str, number: u64, head: &str) -> Result<(), String> {
    crate::github::update_branch(repo, number, head).map_err(|e| format!("{e:#}"))
}
