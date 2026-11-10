//! `capcom prs`: your open and recently merged PRs as one JSON document, for the Herdr app's PR sidebar.
//! It reads GitHub (through the shared cache, so it can be polled often), the sessions that made each PR
//! and the Herdr terminals running them, and needs no `capcom web`.
use crate::herdr_app::{terminal_of, HerdrApp};
use crate::prs::{GhSource, PrSource, PullRequest, SharedSource, Cadence};
use crate::runs::{Fetcher, GhApi, RunSource};
use crate::session::{Index, PrSession};
use crate::settings::Settings;
use crate::web::view::{autofix_on, autoreview_on, check_state, rail};
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

/// Whether a merge at `merged_at` (RFC 3339) is still within the time merged PRs are kept.
fn merged_recently(merged_at: &str, now: chrono::DateTime<chrono::Utc>) -> bool {
    chrono::DateTime::parse_from_rfc3339(merged_at).is_ok_and(|t| now - t.with_timezone(&chrono::Utc) < chrono::Duration::minutes(MERGED_KEEP_MINUTES))
}

fn ci_of(pr: &PullRequest) -> &'static str {
    let c = pr.counts();
    if pr.is_red() {
        "red"
    } else if c.running + c.queued > 0 {
        "pending"
    } else if c.passed == 0 {
        "none"
    } else {
        "green"
    }
}

/// Checks that are not passing (failed, running, queued), which is what a person wants to see.
/// The labels that make the repo's merge bot keep this PR's branch up to date (none without its config).
fn autoupdate_labels(pr: &PullRequest) -> &[String] {
    pr.feedback.kodiak.as_ref().map_or(&[], |k| k.autoupdate.as_slice())
}

fn open_checks(pr: &PullRequest) -> Vec<Value> {
    pr.ordered_checks()
        .iter()
        .filter(|k| !matches!(k.state, crate::prs::CheckState::Passed | crate::prs::CheckState::Skipped))
        .map(|k| json!({ "label": k.label(), "state": check_state(k.state), "startedAt": k.started_at, "url": k.url }))
        .collect()
}

fn open_pr(pr: &PullRequest, settings: &Settings, sessions: &[(String, PrSession)], terminals: &[(String, String)]) -> Value {
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
        "autoupdate": { "available": !autoupdate_labels(pr).is_empty(), "on": autoupdate_labels(pr).iter().any(|l| pr.has_label(l)), "always": pr.feedback.kodiak.as_ref().is_some_and(|k| k.update_always) },
        "blocker": pr.merge_blocker(),
        "managed": pr.managed(),
        "labels": pr.labels,
        "rail": rail(pr),
        "checks": open_checks(pr),
        "autofix": autofix_on(settings, pr),
        "autoreview": autoreview_on(settings, pr),
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

/// Merged PRs stay this long in the sidebar.
pub const MERGED_KEEP_MINUTES: i64 = 30;

fn load_settings() -> Settings {
    crate::settings::default_path().map(|p| crate::settings::load(&p)).unwrap_or_default()
}

/// Auto-fix and auto-review are done by a poller inside `capcom web` (or `capcom-tui`); the sidebar can say
/// whether `capcom web` is there to do them.
fn autopilot_running() -> bool {
    use std::net::{SocketAddr, TcpStream};
    let addr = SocketAddr::from(([127, 0, 0, 1], 1969));
    TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(200)).is_ok()
}

/// What `capcom pr-do` can change.
pub fn act(action: &str, url: &str, on: Option<bool>, query: &str) -> Result<String, String> {
    let on_or = |what: &str| on.ok_or_else(|| format!("{what} needs --on true or --on false"));
    let path = crate::settings::default_path();
    let change = |f: &dyn Fn(&mut Settings)| -> Result<(), String> {
        let path = path.as_ref().ok_or("no home folder for the settings")?;
        let mut s = crate::settings::load(path);
        f(&mut s);
        crate::settings::save(path, &s).map_err(|e| format!("{e:#}"))
    };
    match action {
        "setting-autofix" => {
            let on = on_or("autofix")?;
            change(&|s| s.autofix = on)?;
            Ok(format!("auto-fix is {} by default", if on { "on" } else { "off" }))
        }
        "setting-autoreview" => {
            let on = on_or("autoreview")?;
            change(&|s| s.autocopilot = on)?;
            Ok(format!("auto-review is {} by default", if on { "on" } else { "off" }))
        }
        "autofix" | "autoreview" => {
            let on = on_or(action)?;
            let fix = action == "autofix";
            change(&|s| {
                let list = if fix { &mut s.autofix_prs } else { &mut s.autocopilot_prs };
                list.retain(|(u, _)| !same_url(u, url));
                list.push((url.to_string(), on));
            })?;
            if on {
                if let Some(dir) = crate::cache::Cache::default_dir() {
                    let store = crate::fixstate::Store::new(dir);
                    if fix { store.clear(url) } else { store.forget_copilot(url) }
                }
            }
            Ok(format!("{action} is {} for this PR", if on { "on" } else { "off" }))
        }
        "ready" | "draft" => {
            crate::actions::gh_pr_ready(url, action == "draft")?;
            Ok(if action == "draft" { "it is a draft again".into() } else { "it is ready for review".into() })
        }
        "automerge" | "autoupdate" => {
            let on = on_or(action)?;
            let source = GhSource::new(query.to_string());
            let prs = source.fetch().map_err(|e| format!("{e:#}"))?;
            let pr = prs.iter().find(|p| same_url(&p.url, url) && p.managed()).ok_or("only your own PRs can be changed")?;
            let labels: Vec<String> = if action == "automerge" { pr.automerge_labels().to_vec() } else { autoupdate_labels(pr).to_vec() };
            if labels.is_empty() {
                return Err(format!("this repo's merge bot has no {action} label (a .kodiak.toml)"));
            }
            if on {
                crate::actions::gh_set_label(&pr.repo, pr.number, &labels[0], true)?;
                Ok(format!("{action} is on (added \"{}\")", labels[0]))
            } else {
                for l in labels.iter().filter(|l| pr.has_label(l)) {
                    crate::actions::gh_set_label(&pr.repo, pr.number, l, false)?;
                }
                Ok(format!("{action} is off"))
            }
        }
        _ => Err(format!("unknown action {action:?}")),
    }
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
    let settings = load_settings();

    let mut errors = Vec::new();
    let mut open = match &opened {
        Ok(prs) => prs.iter().map(|p| (p, open_pr(p, &settings, &sessions, &terminals))).collect::<Vec<_>>(),
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
            .filter(|r| merged_recently(&r.updated_at, chrono::Utc::now()))
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
        "settings": { "autofix": settings.autofix, "autoreview": settings.autocopilot },
        "autopilot": autopilot_running(),
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
    fn a_merged_pr_drops_off_after_thirty_minutes() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T03:00:00Z").unwrap().with_timezone(&chrono::Utc);
        assert!(merged_recently("2026-11-10T02:31:00Z", now));
        assert!(!merged_recently("2026-11-10T02:29:00Z", now));
        assert!(!merged_recently("not a time", now));
    }

    #[test]
    fn a_pr_made_by_hand_has_no_agent() {
        assert!(agent_of("https://github.com/acme/widgets/pull/8", &[], &[]).is_null());
    }
}
