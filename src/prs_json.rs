//! `capcom prs`: your open and recently merged PRs as one JSON document, for the Herdr app's PR sidebar.
//! It reads GitHub (through the shared cache, so it can be polled often), the sessions that made each PR
//! and the Herdr terminals running them, and needs no `capcom web`.
use crate::herdr_app::{terminal_of, HerdrApp};
use crate::durations::Durations;
use crate::prs::{GhSource, PrSource, PullRequest, SharedSource, Cadence};
use crate::runs::{Batch, Fetcher, GhApi, Run, RunSource, Watch};
use crate::session::{Index, PrSession};
use crate::settings::Settings;
use crate::web::view::{autofix_on, autoreview_on, check_state, rail, run_state};
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

/// One entry per check that says something about the code, in a steady order, for the progress strip.
/// `done` is how far a running check is through its usual time (0 to 1) when capcom has seen it pass before.
fn strip(pr: &PullRequest, durations: &Durations) -> Vec<Value> {
    let now = chrono::Utc::now();
    pr.checks
        .iter()
        .filter(|k| !matches!(k.state, crate::prs::CheckState::Skipped | crate::prs::CheckState::Cancelled))
        .map(|k| {
            let done = (k.state == crate::prs::CheckState::Running)
                .then(|| Some(crate::prs::check_seconds(k, now)? as f64 / durations.estimate(&pr.repo, k)?.max(1) as f64))
                .flatten()
                .map(|f| (f.clamp(0.0, 0.95) * 100.0).round() / 100.0);
            json!({ "state": check_state(k.state), "group": k.workflow, "label": k.label(), "done": done })
        })
        .collect()
}

fn open_pr(pr: &PullRequest, settings: &Settings, durations: &Durations, sessions: &[(String, PrSession)], terminals: &[(String, String)]) -> Value {
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
        "strip": strip(pr, durations),
        "autofix": autofix_on(settings, pr),
        "autoreview": autoreview_on(settings, pr),
        "updatedAt": pr.updated_at,
        "mine": pr.feedback.mine,
        "agent": agent_of(&pr.url, sessions, terminals),
    })
}

/// A run for the sidebar: its state and the stages that are not passing.
fn run_json(run: &Run) -> Value {
    let stages: Vec<Value> = run
        .jobs
        .iter()
        .filter(|j| !matches!(j.state, crate::runs::RunState::Success | crate::runs::RunState::Skipped))
        .map(|j| json!({ "name": j.name, "state": run_state(j.state), "url": j.url }))
        .collect();
    json!({
        "repo": run.repo,
        "id": run.id,
        "name": run.name,
        "title": run.title,
        "branch": run.branch,
        "url": run.url,
        "state": run_state(run.state),
        "active": run.is_active(),
        "watched": run.watched,
        "startedAt": run.started_at,
        "createdAt": run.created_at,
        "updatedAt": run.updated_at,
        "stages": stages,
    })
}

/// Repos to look for your manual runs in: those of your open PRs and `CAPCOM_DEPLOY_REPOS`.
fn run_repos(opened: &Result<Vec<PullRequest>>) -> Vec<String> {
    let mut repos: Vec<String> = std::env::var("CAPCOM_DEPLOY_REPOS").unwrap_or_default().split(',').map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect();
    if let Ok(prs) = opened {
        repos.extend(prs.iter().map(|p| p.repo.clone()));
    }
    repos.sort();
    repos.dedup();
    repos
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
        "watch" => {
            let link = url.trim();
            if crate::prs::pr_ref(link).is_some() {
                change(&|s| {
                    s.pasted_prs.retain(|u| !same_url(u, link));
                    s.pasted_prs.push(link.to_string());
                })?;
                Ok("watching this PR".into())
            } else if let Some((repo, id)) = crate::runs::parse_run_url(link) {
                let added_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                change(&|s| {
                    s.watched.retain(|w| !(w.repo == repo && w.id == id));
                    s.watched.push(Watch { repo: repo.clone(), id, added_at: added_at.clone() });
                })?;
                Ok("watching this run".into())
            } else {
                Err("copy a link to a GitHub pull request or Actions run first".into())
            }
        }
        "unwatch" => {
            let link = url.trim();
            let run = crate::runs::parse_run_url(link);
            change(&|s| {
                s.pasted_prs.retain(|u| !same_url(u, link));
                if let Some((repo, id)) = &run {
                    s.watched.retain(|w| !(&w.repo == repo && w.id == *id));
                }
            })?;
            Ok("no longer watching it".into())
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

/// The same document from invented PRs, agents and settings (`capcom prs --demo`).
pub fn build_demo() -> Value {
    let terminals: Vec<(String, String)> = crate::demo::agent_specs().iter().filter_map(|a| Some((a.session.to_string(), crate::demo::terminal_of(a.session)?))).collect();
    let fetched = Fetched { opened: Ok(crate::demo::pull_requests()), watching: Ok(Vec::new()), merged: Ok(crate::demo::merged()), merged_watched: Vec::new(), runs: Ok(crate::demo::runs()) };
    assemble(fetched, crate::demo::sessions(), terminals, Settings::default(), Durations::default(), true)
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
    let settings = load_settings();
    let watching: Result<Vec<PullRequest>> = if settings.pasted_prs.is_empty() { Ok(Vec::new()) } else { GhSource::new(query.to_string()).fetch_pasted(&settings.pasted_prs) };

    let merged_live: Arc<dyn RunSource> = Arc::new(Fetcher::new(GhApi::default(), chrono::Duration::minutes(30)));
    let merged_source: Arc<dyn RunSource> = match shared() {
        Some(cache) => Arc::new(crate::runs::SharedSource { inner: merged_live, cache, cadence: crate::runs::RunCadence::default() }),
        None => merged_live,
    };
    let merged = merged_source.fetch_merged(force);
    let open_urls: Vec<String> = opened.iter().flatten().chain(watching.iter().flatten()).map(|p| p.url.clone()).collect();
    let (merged_watched, forget) = if watching.is_ok() { merged_watched(&settings, &open_urls, chrono::Utc::now()) } else { Default::default() };
    if !forget.is_empty() {
        let _ = act_forget(&forget);
    }
    let runs = fetch_runs(&merged_source, &run_repos(&opened), &settings, force);

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
    let durations = cache_dir.as_ref().map(|d| Durations::load(&d.join("check-durations.json"))).unwrap_or_default();

    assemble(Fetched { opened, watching, merged, merged_watched, runs }, sessions, terminals, settings, durations, autopilot_running())
}

/// Watched PRs that are no longer open and were merged in the last `MERGED_KEEP_MINUTES`, as merged rows.
/// Watched PRs that are closed, or merged longer ago, are forgotten.
fn merged_watched(settings: &Settings, open_urls: &[String], now: chrono::DateTime<chrono::Utc>) -> (Vec<Value>, Vec<String>) {
    let mut merged = Vec::new();
    let mut forget = Vec::new();
    for url in settings.pasted_prs.iter().filter(|u| !open_urls.iter().any(|o| same_url(o, u))) {
        let Some((owner, name, number)) = crate::prs::pr_ref(url) else { continue };
        let Ok(text) = crate::github::rest(&format!("repos/{owner}/{name}/pulls/{number}")) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        let at = v.get("merged_at").and_then(Value::as_str).filter(|a| merged_recently(a, now));
        match at {
            Some(at) => merged.push(json!({
                "repo": format!("{owner}/{name}"),
                "number": number,
                "title": v.get("title").and_then(Value::as_str).unwrap_or_default(),
                "url": url,
                "mergedAt": at,
                "watched": true,
            })),
            None if v.get("state").and_then(Value::as_str) == Some("closed") => forget.push(url.clone()),
            None => {}
        }
    }
    (merged, forget)
}

/// Your manual runs and the runs you watch, one list; a run that is both counts as watched.
fn fetch_runs(source: &Arc<dyn RunSource>, repos: &[String], settings: &Settings, force: bool) -> Result<Batch, String> {
    let manual = if repos.is_empty() { Ok(Batch::default()) } else if force { source.fetch_forced(repos) } else { source.fetch(repos) };
    let followed: Vec<(String, u64)> = settings.watched.iter().map(|w| (w.repo.clone(), w.id)).collect();
    let watching = if followed.is_empty() { Ok(Batch::default()) } else { source.fetch_watched(&followed, force) };
    crate::runs::merge(manual.map_err(|e| format!("{e:#}")), watching.map_err(|e| format!("{e:#}")))
}

/// What the sidebar's lists are made from; each part can fail on its own.
struct Fetched {
    opened: Result<Vec<PullRequest>>,
    watching: Result<Vec<PullRequest>>,
    merged: Result<Batch>,
    merged_watched: Vec<Value>,
    runs: Result<Batch, String>,
}

fn act_forget(urls: &[String]) -> Result<(), String> {
    let path = crate::settings::default_path().ok_or("no home folder for the settings")?;
    let mut s = crate::settings::load(&path);
    s.pasted_prs.retain(|u| !urls.iter().any(|f| same_url(f, u)));
    crate::settings::save(&path, &s).map_err(|e| format!("{e:#}"))
}

fn assemble(
    fetched: Fetched,
    sessions: Vec<(String, PrSession)>,
    terminals: Vec<(String, String)>,
    settings: Settings,
    durations: Durations,
    autopilot: bool,
) -> Value {
    let Fetched { opened, watching, merged, merged_watched, runs } = fetched;
    let mut errors = Vec::new();
    let mut open = match &opened {
        Ok(prs) => prs.iter().map(|p| (p, open_pr(p, &settings, &durations, &sessions, &terminals))).collect::<Vec<_>>(),
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
    let mut watch: Vec<(&PullRequest, Value)> = match &watching {
        Ok(prs) => prs.iter().filter(|p| !open.iter().any(|(o, _)| same_url(&o.url, &p.url))).map(|p| (p, open_pr(p, &settings, &durations, &sessions, &terminals))).collect(),
        Err(e) => {
            errors.push(format!("watched PRs: {e:#}"));
            Vec::new()
        }
    };
    watch.sort_by(|a, b| b.0.updated_at.cmp(&a.0.updated_at));
    let runs: Vec<Value> = match runs {
        Ok(mut batch) => {
            batch.runs.sort_by(|a, b| b.is_active().cmp(&a.is_active()).then(b.created_at.cmp(&a.created_at)));
            batch.runs.iter().map(run_json).collect()
        }
        Err(e) => {
            errors.push(format!("runs: {e}"));
            Vec::new()
        }
    };
    open.sort_by(|a, b| rank(a.1["ci"].as_str().unwrap_or("")).cmp(&rank(b.1["ci"].as_str().unwrap_or(""))).then(b.0.updated_at.cmp(&a.0.updated_at)));
    let mut merged: Vec<Value> = match merged {
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
    merged.extend(merged_watched.into_iter().map(|mut m| {
        m["agent"] = agent_of(m["url"].as_str().unwrap_or_default(), &sessions, &terminals);
        m
    }));
    merged.sort_by(|a, b| b["mergedAt"].as_str().cmp(&a["mergedAt"].as_str()));
    json!({
        "at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "open": open.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
        "merged": merged,
        "watching": watch.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
        "runs": runs,
        "settings": { "autofix": settings.autofix, "autoreview": settings.autocopilot },
        "autopilot": autopilot,
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
