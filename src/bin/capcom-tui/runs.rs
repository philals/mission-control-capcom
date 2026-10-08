//! Manual (workflow_dispatch) GitHub Actions runs that you started, across several repos.
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use capcom::refresh::run_with_timeout;

const GH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Queued,
    Running,
    Waiting,
    Success,
    Failed,
    Cancelled,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub name: String,
    pub state: RunState,
    pub url: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub repo: String,
    pub id: u64,
    pub name: String,
    pub title: String,
    pub branch: String,
    pub url: String,
    pub state: RunState,
    pub created_at: String,
    pub started_at: Option<String>,
    pub updated_at: String,
    pub jobs: Vec<Job>,
}

impl Run {
    pub fn is_active(&self) -> bool {
        matches!(self.state, RunState::Queued | RunState::Running | RunState::Waiting)
    }

    fn jobs_in_order(&self, order: &[RunState]) -> Vec<&Job> {
        order
            .iter()
            .flat_map(|state| self.jobs.iter().filter(move |j| j.state == *state))
            .collect()
    }

    /// The stages worth watching, one per line: running, then waiting, queued, then failed.
    pub fn open_jobs(&self) -> Vec<&Job> {
        self.jobs_in_order(&[RunState::Running, RunState::Waiting, RunState::Queued, RunState::Failed])
    }

    /// Every stage in the order the detail sheet lists them.
    pub fn ordered_jobs(&self) -> Vec<&Job> {
        self.jobs_in_order(&[
            RunState::Running,
            RunState::Waiting,
            RunState::Queued,
            RunState::Failed,
            RunState::Success,
            RunState::Cancelled,
            RunState::Skipped,
        ])
    }
}

#[derive(Debug, Clone, Default)]
pub struct Batch {
    pub runs: Vec<Run>,
    pub warnings: Vec<String>,
}

pub fn state_of(status: &str, conclusion: &str) -> RunState {
    match status {
        "in_progress" => RunState::Running,
        "waiting" => RunState::Waiting,
        "completed" => match conclusion {
            "success" => RunState::Success,
            "failure" | "timed_out" | "startup_failure" => RunState::Failed,
            "cancelled" => RunState::Cancelled,
            "action_required" => RunState::Waiting,
            _ => RunState::Skipped,
        },
        _ => RunState::Queued,
    }
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn error_message(v: &Value) -> anyhow::Error {
    anyhow!(text(v, "message").unwrap_or_else(|| "unexpected response from GitHub".to_string()))
}

pub fn parse_runs(repo: &str, json: &str) -> Result<Vec<Run>> {
    let value: Value = serde_json::from_str(json).context("parsing gh output")?;
    let Some(items) = value.get("workflow_runs").and_then(Value::as_array) else {
        return Err(error_message(&value));
    };
    Ok(items
        .iter()
        .filter_map(|r| {
            let name = text(r, "name").or_else(|| text(r, "path"))?;
            Some(Run {
                repo: repo.to_string(),
                id: r.get("id")?.as_u64()?,
                title: text(r, "display_title").unwrap_or_else(|| name.clone()),
                name,
                branch: text(r, "head_branch").unwrap_or_default(),
                url: text(r, "html_url")?,
                state: state_of(
                    &text(r, "status").unwrap_or_default(),
                    &text(r, "conclusion").unwrap_or_default(),
                ),
                created_at: text(r, "created_at")?,
                started_at: text(r, "run_started_at"),
                updated_at: text(r, "updated_at").unwrap_or_default(),
                jobs: Vec::new(),
            })
        })
        .collect())
}

pub fn parse_jobs(json: &str) -> Result<Vec<Job>> {
    let value: Value = serde_json::from_str(json).context("parsing gh output")?;
    let Some(items) = value.get("jobs").and_then(Value::as_array) else {
        return Err(error_message(&value));
    };
    Ok(items
        .iter()
        .filter_map(|j| {
            Some(Job {
                name: text(j, "name")?,
                state: state_of(
                    &text(j, "status").unwrap_or_default(),
                    &text(j, "conclusion").unwrap_or_default(),
                ),
                url: text(j, "html_url")?,
                started_at: text(j, "started_at"),
                completed_at: text(j, "completed_at"),
            })
        })
        .collect())
}

/// Only plain `owner/name` repos reach a `gh api` path.
pub fn valid_repo(repo: &str) -> bool {
    let ok = |s: &str| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && !s.starts_with('-')
            && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    repo.split_once('/').is_some_and(|(owner, name)| ok(owner) && ok(name))
}

pub trait RunSource: Send + Sync {
    fn fetch(&self, repos: &[String]) -> Result<Batch>;
}

pub trait RunApi: Send + Sync {
    fn runs(&self, repo: &str) -> Result<String>;
    fn jobs(&self, repo: &str, run_id: u64) -> Result<String>;
}

pub struct Fetcher<A: RunApi> {
    pub api: A,
    window: chrono::Duration,
    cache: Mutex<HashMap<u64, Vec<Job>>>,
}

fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc))
}

impl<A: RunApi> Fetcher<A> {
    pub fn new(api: A, window: chrono::Duration) -> Fetcher<A> {
        Fetcher { api, window, cache: Mutex::new(HashMap::new()) }
    }

    /// A finished run stays for the window after it finished (its last update); an active one always stays.
    fn recent(&self, run: &Run, now: DateTime<Utc>) -> bool {
        let finished = parse_time(&run.updated_at).or_else(|| parse_time(&run.created_at));
        finished.is_some_and(|t| now - t <= self.window)
    }

    fn fetch_repo(&self, repo: &str, now: DateTime<Utc>) -> Result<(Vec<Run>, Vec<String>)> {
        let json = self.api.runs(repo)?;
        let mut runs: Vec<Run> = parse_runs(repo, &json)?
            .into_iter()
            .filter(|r| r.is_active() || self.recent(r, now))
            .collect();
        let mut warnings = Vec::new();
        for run in runs.iter_mut() {
            if !run.is_active() && run.state != RunState::Failed {
                continue;
            }
            if !run.is_active() {
                if let Some(jobs) = self.cache.lock().expect("cache lock").get(&run.id) {
                    run.jobs = jobs.clone();
                    continue;
                }
            }
            match self.api.jobs(repo, run.id).and_then(|j| parse_jobs(&j)) {
                Ok(jobs) => {
                    if !run.is_active() {
                        self.cache.lock().expect("cache lock").insert(run.id, jobs.clone());
                    }
                    run.jobs = jobs;
                }
                Err(e) => warnings.push(format!("{repo} run {}: {e:#}", run.id)),
            }
        }
        Ok((runs, warnings))
    }

    pub fn fetch_at(&self, repos: &[String], now: DateTime<Utc>) -> Result<Batch> {
        let mut warnings = Vec::new();
        let mut valid: Vec<&str> = Vec::new();
        for repo in repos {
            if valid_repo(repo) {
                valid.push(repo);
            } else {
                warnings.push(format!("ignored invalid repo {repo:?}"));
            }
        }
        if valid.is_empty() {
            return Ok(Batch { runs: Vec::new(), warnings });
        }
        let results: Vec<(&str, Result<(Vec<Run>, Vec<String>)>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = valid
                .iter()
                .map(|repo| scope.spawn(move || (*repo, self.fetch_repo(repo, now))))
                .collect();
            handles.into_iter().map(|h| h.join().expect("fetch thread")).collect()
        });
        let mut runs = Vec::new();
        let mut failures = Vec::new();
        for (repo, result) in results {
            match result {
                Ok((found, more)) => {
                    runs.extend(found);
                    warnings.extend(more);
                }
                Err(e) => failures.push(format!("{repo}: {e:#}")),
            }
        }
        if runs.is_empty() && failures.len() == valid.len() {
            bail!("{}", failures.join("; "));
        }
        warnings.extend(failures);
        runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(Batch { runs, warnings })
    }
}

impl<A: RunApi> RunSource for Fetcher<A> {
    fn fetch(&self, repos: &[String]) -> Result<Batch> {
        self.fetch_at(repos, Utc::now())
    }
}

fn gh_output(args: &[&str]) -> Result<String> {
    let mut cmd = Command::new("gh");
    cmd.args(args);
    let Some(out) = run_with_timeout(cmd, GH_TIMEOUT).context("running gh")? else {
        bail!("gh timed out after {}s", GH_TIMEOUT.as_secs());
    };
    if !out.status.success() {
        bail!("gh failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Talks to GitHub through `gh`. Your login is looked up once, at run time, and kept in memory.
#[derive(Default)]
pub struct GhApi {
    login: OnceLock<Result<String, String>>,
}

impl GhApi {
    fn login(&self) -> Result<String> {
        self.login
            .get_or_init(|| {
                let login = gh_output(&["api", "user", "--jq", ".login"]).map_err(|e| format!("{e:#}"))?;
                let login = login.trim().to_string();
                if login.is_empty() || !login.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                    return Err("could not read your GitHub login from gh".to_string());
                }
                Ok(login)
            })
            .clone()
            .map_err(|e| anyhow!(e))
    }
}

impl RunApi for GhApi {
    fn runs(&self, repo: &str) -> Result<String> {
        let login = self.login()?;
        let path = format!("repos/{repo}/actions/runs?event=workflow_dispatch&actor={login}&per_page=10");
        gh_output(&["api", &path])
    }

    fn jobs(&self, repo: &str, run_id: u64) -> Result<String> {
        gh_output(&["api", &format!("repos/{repo}/actions/runs/{run_id}/jobs?per_page=100")])
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RunCadence {
    pub busy: Duration,
    pub idle: Duration,
}

impl Default for RunCadence {
    fn default() -> Self {
        RunCadence { busy: Duration::from_secs(10), idle: Duration::from_secs(30) }
    }
}

impl RunCadence {
    /// Poll quickly while one of your runs is queued, running or waiting, slowly otherwise.
    pub fn next(&self, result: &Result<Batch, String>) -> Duration {
        match result {
            Ok(batch) if batch.runs.iter().any(Run::is_active) => self.busy,
            _ => self.idle,
        }
    }
}

pub enum RunMsg {
    Started,
    Result(Result<Batch, String>),
}

/// Fetch in a background thread, always for the repos currently in the shared list.
/// Send on the returned sender to refresh right away (for example when the list changes).
pub fn spawn(
    source: Arc<dyn RunSource>,
    repos: Arc<Mutex<Vec<String>>>,
    cadence: RunCadence,
) -> (Receiver<RunMsg>, Sender<()>) {
    let (tx, rx) = mpsc::channel();
    let (wake_tx, wake_rx) = mpsc::channel();
    std::thread::spawn(move || loop {
        if tx.send(RunMsg::Started).is_err() {
            break;
        }
        let list = repos.lock().expect("repo list lock").clone();
        let result = if list.is_empty() {
            Ok(Batch::default())
        } else {
            source.fetch(&list).map_err(|e| format!("{e:#}"))
        };
        let wait = cadence.next(&result);
        if tx.send(RunMsg::Result(result)).is_err() {
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

    const RUNS: &str = r#"{"total_count":4,"workflow_runs":[
      {"id":101,"name":"Deploy nonprod","display_title":"Deploy to nonprod","path":".github/workflows/nonprod.yml",
       "head_branch":"feat/x","event":"workflow_dispatch","status":"in_progress","conclusion":null,
       "html_url":"https://github.com/acme/widgets/actions/runs/101","created_at":"2026-10-08T03:00:00Z",
       "run_started_at":"2026-10-08T03:00:05Z","updated_at":"2026-10-08T03:02:00Z"},
      {"id":100,"name":"Deploy nonprod","display_title":"Deploy to nonprod","path":".github/workflows/nonprod.yml",
       "head_branch":"main","event":"workflow_dispatch","status":"completed","conclusion":"success",
       "html_url":"https://github.com/acme/widgets/actions/runs/100","created_at":"2026-10-08T01:00:00Z",
       "run_started_at":"2026-10-08T01:00:02Z","updated_at":"2026-10-08T01:05:00Z"},
      {"id":98,"name":"Deploy nonprod","display_title":"Failed deploy","path":".github/workflows/nonprod.yml",
       "head_branch":"main","event":"workflow_dispatch","status":"completed","conclusion":"failure",
       "html_url":"https://github.com/acme/widgets/actions/runs/98","created_at":"2026-10-08T02:00:00Z",
       "run_started_at":"2026-10-08T02:00:02Z","updated_at":"2026-10-08T02:03:00Z"},
      {"id":90,"name":"Old deploy","display_title":"Old","path":".github/workflows/old.yml",
       "head_branch":"main","event":"workflow_dispatch","status":"completed","conclusion":"failure",
       "html_url":"https://github.com/acme/widgets/actions/runs/90","created_at":"2026-10-01T01:00:00Z",
       "run_started_at":null,"updated_at":"2026-10-01T01:05:00Z"}
    ]}"#;

    const JOBS: &str = r#"{"total_count":5,"jobs":[
      {"id":1,"name":"build","status":"completed","conclusion":"success","started_at":"2026-10-08T03:00:10Z","completed_at":"2026-10-08T03:01:10Z","html_url":"https://github.com/acme/widgets/actions/runs/101/job/1","workflow_name":"Deploy nonprod"},
      {"id":2,"name":"deploy","status":"in_progress","conclusion":null,"started_at":"2026-10-08T03:01:20Z","completed_at":null,"html_url":"https://github.com/acme/widgets/actions/runs/101/job/2","workflow_name":"Deploy nonprod"},
      {"id":3,"name":"smoke","status":"queued","conclusion":null,"started_at":null,"completed_at":null,"html_url":"https://github.com/acme/widgets/actions/runs/101/job/3","workflow_name":"Deploy nonprod"},
      {"id":4,"name":"approve","status":"waiting","conclusion":null,"started_at":null,"completed_at":null,"html_url":"https://github.com/acme/widgets/actions/runs/101/job/4","workflow_name":"Deploy nonprod"},
      {"id":5,"name":"lint","status":"completed","conclusion":"failure","started_at":"2026-10-08T03:00:10Z","completed_at":"2026-10-08T03:00:40Z","html_url":"https://github.com/acme/widgets/actions/runs/101/job/5","workflow_name":"Deploy nonprod"}
    ]}"#;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 8, 4, 0, 0).unwrap()
    }

    #[test]
    fn parse_runs_reads_workflow_title_branch_link_and_state() {
        let runs = parse_runs("acme/widgets", RUNS).unwrap();
        assert_eq!(runs.len(), 4);
        let r = &runs[0];
        assert_eq!((r.repo.as_str(), r.id, r.name.as_str()), ("acme/widgets", 101, "Deploy nonprod"));
        assert_eq!((r.title.as_str(), r.branch.as_str()), ("Deploy to nonprod", "feat/x"));
        assert_eq!(r.url, "https://github.com/acme/widgets/actions/runs/101");
        assert_eq!(r.state, RunState::Running);
        assert_eq!(runs[1].state, RunState::Success);
        assert_eq!(runs[2].state, RunState::Failed);
        assert!(r.is_active() && !runs[1].is_active());
        assert!(r.jobs.is_empty());
    }

    #[test]
    fn states_map_from_github_status_and_conclusion() {
        let s = |status: &str, conclusion: &str| state_of(status, conclusion);
        assert_eq!(s("queued", ""), RunState::Queued);
        assert_eq!(s("requested", ""), RunState::Queued);
        assert_eq!(s("pending", ""), RunState::Queued);
        assert_eq!(s("in_progress", ""), RunState::Running);
        assert_eq!(s("waiting", ""), RunState::Waiting);
        assert_eq!(s("completed", "success"), RunState::Success);
        assert_eq!(s("completed", "failure"), RunState::Failed);
        assert_eq!(s("completed", "timed_out"), RunState::Failed);
        assert_eq!(s("completed", "cancelled"), RunState::Cancelled);
        assert_eq!(s("completed", "skipped"), RunState::Skipped);
        assert_eq!(s("completed", "action_required"), RunState::Waiting);
    }

    #[test]
    fn parse_jobs_reads_every_stage_with_its_link() {
        let jobs = parse_jobs(JOBS).unwrap();
        let states: Vec<(&str, RunState)> = jobs.iter().map(|j| (j.name.as_str(), j.state)).collect();
        assert_eq!(
            states,
            vec![
                ("build", RunState::Success),
                ("deploy", RunState::Running),
                ("smoke", RunState::Queued),
                ("approve", RunState::Waiting),
                ("lint", RunState::Failed)
            ]
        );
        assert_eq!(jobs[1].url, "https://github.com/acme/widgets/actions/runs/101/job/2");
        assert_eq!(jobs[1].started_at.as_deref(), Some("2026-10-08T03:01:20Z"));
    }

    #[test]
    fn the_sheet_order_of_stages_is_running_waiting_queued_failed_then_the_rest() {
        let mut run = parse_runs("acme/widgets", RUNS).unwrap().remove(0);
        run.jobs = parse_jobs(JOBS).unwrap();
        let names: Vec<&str> = run.ordered_jobs().iter().map(|j| j.name.as_str()).collect();
        assert_eq!(names, vec!["deploy", "approve", "smoke", "lint", "build"]);
        let open: Vec<&str> = run.open_jobs().iter().map(|j| j.name.as_str()).collect();
        assert_eq!(open, vec!["deploy", "approve", "smoke", "lint"]);
    }

    #[test]
    fn parse_reports_api_errors() {
        let e = parse_runs("acme/widgets", r#"{"message":"Not Found"}"#).unwrap_err();
        assert!(e.to_string().contains("Not Found"), "{e}");
        assert!(parse_jobs("nope").is_err());
    }

    #[test]
    fn only_owner_slash_name_repos_are_accepted() {
        assert!(valid_repo("acme/widgets"));
        assert!(valid_repo("a-b/c_d.e"));
        for bad in ["", "acme", "acme/", "/x", "a/b/c", "a b/c", "a/b?x=1", "../x", "a/..%2f", "-f/x"] {
            assert!(!valid_repo(bad), "{bad:?} must be rejected");
        }
    }

    struct FakeApi {
        runs: Vec<(&'static str, Result<&'static str, &'static str>)>,
        job_calls: std::sync::Mutex<Vec<u64>>,
    }

    impl FakeApi {
        fn new(runs: Vec<(&'static str, Result<&'static str, &'static str>)>) -> FakeApi {
            FakeApi { runs, job_calls: std::sync::Mutex::new(Vec::new()) }
        }
    }

    impl RunApi for FakeApi {
        fn runs(&self, repo: &str) -> Result<String> {
            match self.runs.iter().find(|(r, _)| *r == repo) {
                Some((_, Ok(json))) => Ok((*json).to_string()),
                Some((_, Err(e))) => bail!("{e}"),
                None => bail!("unknown repo {repo}"),
            }
        }

        fn jobs(&self, _repo: &str, run_id: u64) -> Result<String> {
            self.job_calls.lock().unwrap().push(run_id);
            Ok(JOBS.to_string())
        }
    }

    #[test]
    fn the_fetcher_keeps_active_and_recent_runs_and_loads_stages_only_where_useful() {
        let fetcher = Fetcher::new(FakeApi::new(vec![("acme/widgets", Ok(RUNS))]), chrono::Duration::hours(24));
        let batch = fetcher.fetch_at(&["acme/widgets".to_string()], now()).unwrap();
        let ids: Vec<u64> = batch.runs.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![101, 98, 100], "newest first; the old completed run is dropped");
        assert!(batch.warnings.is_empty());
        let mut calls = fetcher.api.job_calls.lock().unwrap().clone();
        calls.sort();
        assert_eq!(calls, vec![98, 101], "stages for the active run and the failed one, not the success");
        assert_eq!(batch.runs[0].jobs.len(), 5);
        assert!(batch.runs[2].jobs.is_empty());
    }

    #[test]
    fn finished_runs_drop_off_three_hours_after_they_finished_but_active_runs_stay() {
        let fetcher = Fetcher::new(FakeApi::new(vec![("acme/widgets", Ok(RUNS))]), chrono::Duration::hours(3));
        let repos = vec!["acme/widgets".to_string()];
        let at = |h: u32, m: u32| Utc.with_ymd_and_hms(2026, 10, 8, h, m, 0).unwrap();
        let ids = |now| fetcher.fetch_at(&repos, now).unwrap().runs.iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(ids(at(4, 0)), vec![101, 98, 100], "98 finished 02:03 and 100 finished 01:05, both under 3h ago");
        assert_eq!(ids(at(4, 3)), vec![101, 98, 100], "100 was created 01:00 but finished 01:05: the 3 hours count from the finish");
        assert_eq!(ids(at(4, 30)), vec![101, 98], "100 finished 01:05, now over 3h ago");
        assert_eq!(ids(at(5, 30)), vec![101], "98 finished 02:03, now over 3h ago; the active run never drops off");
    }

    #[test]
    fn stages_of_finished_runs_are_cached_but_active_runs_are_refetched() {
        let fetcher = Fetcher::new(FakeApi::new(vec![("acme/widgets", Ok(RUNS))]), chrono::Duration::hours(24));
        let repos = vec!["acme/widgets".to_string()];
        fetcher.fetch_at(&repos, now()).unwrap();
        fetcher.fetch_at(&repos, now()).unwrap();
        let mut calls = fetcher.api.job_calls.lock().unwrap().clone();
        calls.sort();
        assert_eq!(calls, vec![98, 101, 101], "the finished failed run (98) is fetched once; the active one every time");
    }

    #[test]
    fn a_failing_repo_is_a_warning_and_the_others_still_load() {
        let api = FakeApi::new(vec![("acme/widgets", Ok(RUNS)), ("acme/api", Err("HTTP 403"))]);
        let fetcher = Fetcher::new(api, chrono::Duration::hours(24));
        let repos = vec!["acme/api".to_string(), "acme/widgets".to_string()];
        let batch = fetcher.fetch_at(&repos, now()).unwrap();
        assert_eq!(batch.runs.len(), 3);
        assert!(batch.warnings.iter().any(|w| w.contains("acme/api") && w.contains("HTTP 403")), "{:?}", batch.warnings);
    }

    #[test]
    fn every_repo_failing_is_an_error_and_invalid_repos_are_skipped() {
        let fetcher = Fetcher::new(FakeApi::new(vec![("acme/api", Err("HTTP 403"))]), chrono::Duration::hours(24));
        let err = fetcher.fetch_at(&["acme/api".to_string()], now()).unwrap_err();
        assert!(err.to_string().contains("HTTP 403"), "{err}");
        let batch = fetcher.fetch_at(&["../etc".to_string()], now()).unwrap();
        assert!(batch.runs.is_empty());
        assert!(batch.warnings.iter().any(|w| w.contains("../etc")), "{:?}", batch.warnings);
        assert!(fetcher.fetch_at(&[], now()).unwrap().runs.is_empty());
    }

    #[test]
    fn the_poll_interval_is_short_only_while_a_run_is_active() {
        let cadence = RunCadence { busy: Duration::from_secs(10), idle: Duration::from_secs(60) };
        let runs = parse_runs("acme/widgets", RUNS).unwrap();
        let batch = |runs: Vec<Run>| Ok(Batch { runs, warnings: vec![] });
        assert_eq!(cadence.next(&batch(runs.clone())), Duration::from_secs(10));
        assert_eq!(cadence.next(&batch(runs[1..].to_vec())), Duration::from_secs(60));
        assert_eq!(cadence.next(&Err("boom".into())), Duration::from_secs(60));
    }

    #[test]
    fn by_default_an_idle_poll_is_every_thirty_seconds_and_a_busy_one_every_ten() {
        let c = RunCadence::default();
        assert_eq!((c.busy, c.idle), (Duration::from_secs(10), Duration::from_secs(30)));
    }

    struct Recording {
        seen: std::sync::Mutex<Vec<Vec<String>>>,
        calls: AtomicUsize,
    }

    impl RunSource for Recording {
        fn fetch(&self, repos: &[String]) -> Result<Batch> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push(repos.to_vec());
            Ok(Batch { runs: Vec::new(), warnings: Vec::new() })
        }
    }

    fn next_batch(rx: &Receiver<RunMsg>) -> Result<Batch, String> {
        loop {
            match rx.recv_timeout(Duration::from_secs(3)).expect("a result in time") {
                RunMsg::Started => continue,
                RunMsg::Result(r) => return r,
            }
        }
    }

    #[test]
    fn the_poller_follows_the_shared_repo_list_and_wakes_on_demand() {
        let source = Arc::new(Recording { seen: Default::default(), calls: AtomicUsize::new(0) });
        let repos = Arc::new(Mutex::new(Vec::<String>::new()));
        let cadence = RunCadence { busy: Duration::from_secs(30), idle: Duration::from_secs(30) };
        let (rx, wake) = spawn(source.clone(), repos.clone(), cadence);
        assert!(next_batch(&rx).unwrap().runs.is_empty());
        assert_eq!(source.calls.load(Ordering::SeqCst), 0, "no repos: nothing is fetched");
        *repos.lock().unwrap() = vec!["acme/widgets".to_string()];
        wake.send(()).unwrap();
        assert!(next_batch(&rx).is_ok());
        assert_eq!(*source.seen.lock().unwrap(), vec![vec!["acme/widgets".to_string()]]);
    }
}
