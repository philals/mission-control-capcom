//! `capcom prs`: your open and recently merged PRs as one JSON document, for the Herdr app's PR sidebar.
//! It reads GitHub (through the shared cache, so it can be polled often), the sessions that made each PR
//! and the Herdr terminals running them, and needs no `capcom web`.
use crate::herdr_app::{terminal_of, HerdrApp};
use crate::prs::{GhSource, PrSource, PullRequest, SharedSource, Cadence};
use crate::runs::{Fetcher, GhApi, RunSource};
use crate::session::{Index, PrSession};
use anyhow::Result;
use serde_json::{json, Value};
use std::sync::Arc;

fn same_url(a: &str, b: &str) -> bool {
    a.trim_end_matches('/').eq_ignore_ascii_case(b.trim_end_matches('/'))
}

/// Where a PR's agent is: the session that made it, its folder, and the Herdr terminal running it now.
fn agent_of(url: &str, sessions: &[(String, PrSession)], terminals: &[(String, String)]) -> Value {
    let Some((_, made)) = sessions.iter().find(|(u, _)| same_url(u, url)) else {
        return Value::Null;
    };
    let terminal = terminals.iter().find(|(s, _)| *s == made.session).map(|(_, t)| t.as_str());
    json!({ "session": made.session, "cwd": made.cwd, "terminal": terminal })
}

fn ci_of(pr: &PullRequest) -> &'static str {
    let c = pr.counts();
    if pr.is_red() {
        "red"
    } else if c.running + c.queued > 0 {
        "pending"
    } else {
        "green"
    }
}

fn open_pr(pr: &PullRequest, sessions: &[(String, PrSession)], terminals: &[(String, String)]) -> Value {
    let c = pr.counts();
    json!({
        "repo": pr.repo,
        "number": pr.number,
        "title": pr.title,
        "url": pr.url,
        "branch": pr.feedback.branch,
        "draft": pr.is_draft,
        "ci": ci_of(pr),
        "counts": { "passed": c.passed, "failed": c.failed, "running": c.running, "queued": c.queued },
        "failing": pr.real_failures().iter().map(|k| k.label()).collect::<Vec<_>>(),
        "conflict": pr.feedback.merge == crate::prs::MergeState::Conflicting,
        "automerge": { "available": !pr.automerge_labels().is_empty(), "on": pr.automerge_on() },
        "blocker": pr.merge_blocker(),
        "updatedAt": pr.updated_at,
        "agent": agent_of(&pr.url, sessions, terminals),
    })
}

/// Live Herdr terminals, each with the Claude session it runs. Empty when the app is not running.
fn terminals(config: Option<&std::path::Path>) -> Vec<(String, String)> {
    let Some(config) = config else { return Vec::new() };
    let live = crate::claude::live(config);
    let by_pid = crate::herdr_app::sessions_by_pid(&live);
    HerdrApp::new()
        .agents(&by_pid)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|a| Some((a.session?, terminal_of(&a.pane)?.to_string())))
        .collect()
}

pub fn build(query: &str, force: bool) -> Value {
    let cache_dir = crate::cache::Cache::default_dir();
    let shared = || cache_dir.clone().and_then(crate::cache::Cache::open);
    let live: Arc<dyn PrSource> = Arc::new(GhSource::new(query.to_string()));
    let source: Arc<dyn PrSource> = match shared() {
        Some(cache) => Arc::new(SharedSource::new(live, cache, query, Cadence::default())),
        None => live,
    };
    let opened: Result<Vec<PullRequest>> = if force { source.fetch_forced() } else { source.fetch() };

    let merged_live: Arc<dyn RunSource> = Arc::new(Fetcher::new(GhApi::default(), chrono::Duration::minutes(30)));
    let merged_source: Arc<dyn RunSource> = match shared() {
        Some(cache) => Arc::new(crate::runs::SharedSource { inner: merged_live, cache, cadence: crate::runs::RunCadence::default() }),
        None => merged_live,
    };
    let merged = merged_source.fetch_merged(force);

    let config = crate::session::config_dir();
    let mut index = cache_dir.as_ref().map(|d| d.join("prs-links.json")).map(|f| (Index::load(&f), f));
    let sessions = match (&mut index, &config) {
        (Some((index, file)), Some(config)) => {
            if index.refresh(config) {
                let _ = index.save(file);
            }
            index.sessions()
        }
        _ => Vec::new(),
    };
    let terminals = terminals(config.as_deref());

    let mut errors = Vec::new();
    let mut open = match &opened {
        Ok(prs) => prs.iter().map(|p| (p, open_pr(p, &sessions, &terminals))).collect::<Vec<_>>(),
        Err(e) => {
            errors.push(format!("open PRs: {e:#}"));
            Vec::new()
        }
    };
    let rank = |ci: &str| match ci {
        "red" => 0,
        "pending" => 1,
        _ => 2,
    };
    open.sort_by(|a, b| rank(a.1["ci"].as_str().unwrap_or("")).cmp(&rank(b.1["ci"].as_str().unwrap_or(""))).then(b.0.updated_at.cmp(&a.0.updated_at)));
    let merged = match merged {
        Ok(batch) => batch
            .runs
            .iter()
            .map(|r| json!({
                "repo": r.repo,
                "number": r.id,
                "title": r.title,
                "url": r.url,
                "mergedAt": r.updated_at,
                "agent": agent_of(&r.url, &sessions, &terminals),
            }))
            .collect(),
        Err(e) => {
            errors.push(format!("merged PRs: {e:#}"));
            Vec::new()
        }
    };
    json!({
        "at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "open": open.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
        "merged": merged,
        "errors": errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn made(url: &str, session: &str) -> (String, PrSession) {
        (url.to_lowercase(), PrSession { session: session.into(), cwd: "/work/repo".into() })
    }

    #[test]
    fn an_agent_is_found_by_the_pr_link_and_its_terminal_by_the_session() {
        let sessions = vec![made("https://github.com/acme/widgets/pull/7", "s1")];
        let live = vec![("s1".to_string(), "t9".to_string())];
        let a = agent_of("https://github.com/Acme/widgets/pull/7/", &sessions, &live);
        assert_eq!(a["session"], "s1");
        assert_eq!(a["terminal"], "t9");
        assert_eq!(a["cwd"], "/work/repo");
    }

    #[test]
    fn a_closed_agent_has_a_session_but_no_terminal() {
        let sessions = vec![made("https://github.com/acme/widgets/pull/7", "s1")];
        let a = agent_of("https://github.com/acme/widgets/pull/7", &sessions, &[]);
        assert_eq!(a["session"], "s1");
        assert!(a["terminal"].is_null());
    }

    #[test]
    fn a_pr_made_by_hand_has_no_agent() {
        assert!(agent_of("https://github.com/acme/widgets/pull/8", &[], &[]).is_null());
    }
}
