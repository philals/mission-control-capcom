//! Manual (workflow_dispatch) GitHub Actions runs that you started, across several repos.
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use crate::cache::{self, Cache};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunState {
    Queued,
    Running,
    Waiting,
    Success,
    Failed,
    Cancelled,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub name: String,
    pub state: RunState,
    pub url: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Added by pasting its link, rather than found as one of your manual runs.
    #[serde(default)]
    pub watched: bool,
}

/// A run you asked to follow: it stays in the panel while it runs and for a while after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watch {
    pub repo: String,
    pub id: u64,
    pub added_at: String,
}

/// `(owner/name, run id)` from a link to a GitHub Actions run or one of its jobs, such as
/// `https://github.com/acme/web/actions/runs/123` or `…/runs/123/job/456`.
pub fn parse_run_url(text: &str) -> Option<(String, u64)> {
    let rest = text.trim().strip_prefix("https://github.com/")?;
    let mut parts = rest.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    if (parts.next()?, parts.next()?) != ("actions", "runs") {
        return None;
    }
    let id: String = parts.next()?.chars().take_while(char::is_ascii_digit).collect();
    let repo = format!("{owner}/{name}");
    valid_repo(&repo).then_some(())?;
    Some((repo, id.parse().ok()?))
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

/// One run, as returned by `repos/{repo}/actions/runs/{id}`.
pub fn parse_run(repo: &str, json: &str) -> Result<Run> {
    let value: Value = serde_json::from_str(json).context("parsing gh output")?;
    parse_one(repo, &value).ok_or_else(|| error_message(&value))
}

pub fn parse_runs(repo: &str, json: &str) -> Result<Vec<Run>> {
    let value: Value = serde_json::from_str(json).context("parsing gh output")?;
    let Some(items) = value.get("workflow_runs").and_then(Value::as_array) else {
        return Err(error_message(&value));
    };
    Ok(items.iter().filter_map(|r| parse_one(repo, r)).collect())
}

fn parse_one(repo: &str, r: &Value) -> Option<Run> {
    let name = text(r, "name").or_else(|| text(r, "path"))?;
    Some(Run {
        repo: repo.to_string(),
        id: r.get("id")?.as_u64()?,
        title: text(r, "display_title").unwrap_or_else(|| name.clone()),
        name,
        branch: text(r, "head_branch").unwrap_or_default(),
        url: text(r, "html_url")?,
        state: state_of(&text(r, "status").unwrap_or_default(), &text(r, "conclusion").unwrap_or_default()),
        created_at: text(r, "created_at")?,
        started_at: text(r, "run_started_at"),
        updated_at: text(r, "updated_at").unwrap_or_default(),
        jobs: Vec::new(),
        watched: false,
    })
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

    /// The runs you asked to watch, whatever their age or who started them.
    fn fetch_watched(&self, _watched: &[(String, u64)], _force: bool) -> Result<Batch> {
        Ok(Batch::default())
    }

    /// A manual refresh: skip any "recent enough" shortcut.
    fn fetch_forced(&self, repos: &[String]) -> Result<Batch> {
        self.fetch(repos)
    }
}

/// Shares one fetch between every open copy that watches the same repos, through the on-disk cache.
pub struct SharedSource {
    pub inner: Arc<dyn RunSource>,
    pub cache: Cache,
    pub cadence: RunCadence,
}

impl SharedSource {
    fn get(&self, repos: &[String], force: bool) -> Result<Batch> {
        let mut sorted: Vec<&str> = repos.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        let name = format!("runs-{}", cache::key_of(&sorted));
        let cadence = self.cadence;
        self.cache.shared(
            &name,
            Utc::now,
            move |batch: &Batch| cadence.interval(batch),
            force,
            || if force { self.inner.fetch_forced(repos) } else { self.inner.fetch(repos) },
        )
    }
}

impl RunSource for SharedSource {
    fn fetch(&self, repos: &[String]) -> Result<Batch> {
        self.get(repos, false)
    }

    fn fetch_watched(&self, watched: &[(String, u64)], force: bool) -> Result<Batch> {
        let mut sorted: Vec<String> = watched.iter().map(|(r, id)| format!("{r}#{id}")).collect();
        sorted.sort_unstable();
        let name = format!("watch-{}", cache::key_of(&sorted.iter().map(String::as_str).collect::<Vec<_>>()));
        let cadence = self.cadence;
        self.cache.shared(&name, Utc::now, move |batch: &Batch| cadence.interval(batch), force, || {
            self.inner.fetch_watched(watched, force)
        })
    }

    fn fetch_forced(&self, repos: &[String]) -> Result<Batch> {
        self.get(repos, true)
    }
}

pub trait RunApi: Send + Sync {
    fn runs(&self, repo: &str) -> Result<String>;
    fn run(&self, repo: &str, run_id: u64) -> Result<String>;
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

    /// Stages of a run: loaded while it is active or failed, and kept once it has finished.
    fn load_jobs(&self, run: &mut Run, warnings: &mut Vec<String>) {
        if !run.is_active() && run.state != RunState::Failed {
            return;
        }
        if !run.is_active() {
            if let Some(jobs) = self.cache.lock().expect("cache lock").get(&run.id) {
                run.jobs = jobs.clone();
                return;
            }
        }
        match self.api.jobs(&run.repo, run.id).and_then(|j| parse_jobs(&j)) {
            Ok(jobs) => {
                if !run.is_active() {
                    self.cache.lock().expect("cache lock").insert(run.id, jobs.clone());
                }
                run.jobs = jobs;
            }
            Err(e) => warnings.push(format!("{} run {}: {e:#}", run.repo, run.id)),
        }
    }

    pub fn fetch_watched_runs(&self, watched: &[(String, u64)]) -> Result<Batch> {
        let mut batch = Batch::default();
        let mut failures = Vec::new();
        for (repo, id) in watched.iter().filter(|(r, _)| valid_repo(r)) {
            match self.api.run(repo, *id).and_then(|j| parse_run(repo, &j)) {
                Ok(mut run) => {
                    run.watched = true;
                    self.load_jobs(&mut run, &mut batch.warnings);
                    batch.runs.push(run);
                }
                Err(e) => failures.push(format!("{repo} run {id}: {e:#}")),
            }
        }
        if batch.runs.is_empty() && !failures.is_empty() {
            bail!("{}", failures.join("; "));
        }
        batch.warnings.extend(failures);
        Ok(batch)
    }

    fn fetch_repo(&self, repo: &str, now: DateTime<Utc>) -> Result<(Vec<Run>, Vec<String>)> {
        let json = self.api.runs(repo)?;
        let mut runs: Vec<Run> = parse_runs(repo, &json)?
            .into_iter()
            .filter(|r| r.is_active() || self.recent(r, now))
            .collect();
        let mut warnings = Vec::new();
        for run in runs.iter_mut() {
            self.load_jobs(run, &mut warnings);
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

    fn fetch_watched(&self, watched: &[(String, u64)], _force: bool) -> Result<Batch> {
        self.fetch_watched_runs(watched)
    }
}

/// Talks to GitHub from inside capcom (see `capcom::github`). Your login is looked up once, at run
/// time, and kept in memory.
#[derive(Default)]
pub struct GhApi {
    login: OnceLock<Result<String, String>>,
}

impl GhApi {
    fn login(&self) -> Result<String> {
        self.login
            .get_or_init(|| {
                let user = capcom::github::rest("user").map_err(|e| format!("{e:#}"))?;
                let login = serde_json::from_str::<Value>(&user)
                    .ok()
                    .and_then(|v| v.get("login").and_then(Value::as_str).map(str::to_string))
                    .unwrap_or_default();
                if login.is_empty() || !login.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                    return Err("could not read your GitHub login".to_string());
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
        capcom::github::rest(&format!(
            "repos/{repo}/actions/runs?event=workflow_dispatch&actor={login}&per_page=10&exclude_pull_requests=true"
        ))
    }

    fn run(&self, repo: &str, run_id: u64) -> Result<String> {
        capcom::github::rest(&format!("repos/{repo}/actions/runs/{run_id}"))
    }

    fn jobs(&self, repo: &str, run_id: u64) -> Result<String> {
        capcom::github::rest(&format!("repos/{repo}/actions/runs/{run_id}/jobs?per_page=100"))
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
            Ok(batch) => self.interval(batch),
            _ => self.idle,
        }
    }

    pub fn interval(&self, batch: &Batch) -> Duration {
        if batch.runs.iter().any(Run::is_active) {
            self.busy
        } else {
            self.idle
        }
    }
}

/// Your manual runs and the runs you watch in one list. A run that is both counts as watched, and
/// when only one of the two lookups fails the other still shows, with the failure as a warning.
fn merge(manual: Result<Batch, String>, watching: Result<Batch, String>) -> Result<Batch, String> {
    match (manual, watching) {
        (Ok(mut batch), Ok(extra)) => {
            for run in extra.runs {
                match batch.runs.iter_mut().find(|r| r.repo == run.repo && r.id == run.id) {
                    Some(existing) => existing.watched = true,
                    None => batch.runs.push(run),
                }
            }
            batch.warnings.extend(extra.warnings);
            batch.runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            Ok(batch)
        }
        (Ok(mut batch), Err(e)) | (Err(e), Ok(mut batch)) => {
            batch.warnings.push(e);
            Ok(batch)
        }
        (Err(a), Err(b)) => Err(format!("{a}; {b}")),
    }
}

pub enum RunMsg {
    Started,
    Result(Result<Batch, String>),
}

/// Fetch in a background thread, always for the repos currently in the shared list.
/// Send on the returned sender to refresh right away (for example when the list changes).
#[cfg(test)]
pub fn spawn(
    source: Arc<dyn RunSource>,
    repos: Arc<Mutex<Vec<String>>>,
    watched: Arc<Mutex<Vec<(String, u64)>>>,
    cadence: RunCadence,
) -> (Receiver<RunMsg>, Sender<()>) {
    spawn_with(source, repos, watched, cadence, crate::attention::Attention::new())
}

/// Like `spawn`, and polls more slowly while `attention` says nobody is looking.
pub fn spawn_with(
    source: Arc<dyn RunSource>,
    repos: Arc<Mutex<Vec<String>>>,
    watched: Arc<Mutex<Vec<(String, u64)>>>,
    cadence: RunCadence,
    attention: Arc<crate::attention::Attention>,
) -> (Receiver<RunMsg>, Sender<()>) {
    let (tx, rx) = mpsc::channel();
    let (wake_tx, wake_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut forced = false;
        loop {
            if tx.send(RunMsg::Started).is_err() {
                break;
            }
            let list = repos.lock().expect("repo list lock").clone();
            let followed = watched.lock().expect("watch list lock").clone();
            let manual = if list.is_empty() {
                Ok(Batch::default())
            } else if forced {
                source.fetch_forced(&list).map_err(|e| format!("{e:#}"))
            } else {
                source.fetch(&list).map_err(|e| format!("{e:#}"))
            };
            let watching = if followed.is_empty() {
                Ok(Batch::default())
            } else {
                source.fetch_watched(&followed, forced).map_err(|e| format!("{e:#}"))
            };
            let result = merge(manual, watching);
            let wait = attention.wait(cadence.next(&result));
            if tx.send(RunMsg::Result(result)).is_err() {
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
    fn run_links_give_the_repo_and_run_id_whether_they_point_at_the_run_or_one_of_its_jobs() {
        let want = Some(("linz/web".to_string(), 37880156807));
        assert_eq!(parse_run_url("https://github.com/linz/web/actions/runs/37880156807"), want);
        assert_eq!(parse_run_url("  https://github.com/linz/web/actions/runs/37880156807/job/99?pr=1\n"), want);
        assert_eq!(parse_run_url("https://github.com/linz/web/actions/runs/37880156807#summary"), want);
        for bad in ["", "PROJ-123", "https://github.com/linz/web/pull/12", "https://github.com/linz/web/actions/runs/", "https://github.com/linz/web/actions/runs/abc", "https://example.com/linz/web/actions/runs/1", "https://github.com/../web/actions/runs/1"] {
            assert_eq!(parse_run_url(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_watched_run_is_fetched_by_id_flagged_and_given_stages_while_active() {
        let fetcher = Fetcher::new(FakeApi::new(vec![]), chrono::Duration::minutes(30));
        let batch = fetcher.fetch_watched_runs(&[("acme/web".into(), 777), ("acme/web".into(), 778), ("acme/web".into(), 999)]).unwrap();
        assert_eq!(batch.runs.iter().map(|r| (r.id, r.watched, r.state)).collect::<Vec<_>>(), vec![(777, true, RunState::Running), (778, true, RunState::Success)]);
        assert!(!batch.runs[0].jobs.is_empty() && batch.runs[1].jobs.is_empty(), "stages only for the active run");
        assert!(batch.warnings.iter().any(|w| w.contains("999") && w.contains("Not Found")), "{:?}", batch.warnings);
        let nothing = fetcher.fetch_watched_runs(&[("acme/web".into(), 999)]);
        assert!(nothing.unwrap_err().to_string().contains("Not Found"), "a watch list that finds nothing is an error");
    }

    #[test]
    fn manual_and_watched_runs_merge_with_a_run_in_both_counted_as_watched_and_one_failing_source_only_warns() {
        let run = |id: u64, at: &str, watched: bool| Run { id, created_at: at.into(), watched, ..parse_runs("acme/widgets", RUNS).unwrap().remove(0) };
        let manual = Batch { runs: vec![run(1, "2026-10-01T01:00:00Z", false), run(2, "2026-10-01T03:00:00Z", false)], warnings: vec![] };
        let watching = Batch { runs: vec![run(2, "2026-10-01T03:00:00Z", true), run(3, "2026-10-01T02:00:00Z", true)], warnings: vec!["w".into()] };
        let merged = merge(Ok(manual.clone()), Ok(watching)).unwrap();
        assert_eq!(merged.runs.iter().map(|r| (r.id, r.watched)).collect::<Vec<_>>(), vec![(2, true), (3, true), (1, false)]);
        let one = merge(Ok(manual), Err("HTTP 404".into())).unwrap();
        assert_eq!((one.runs.len(), one.warnings), (2, vec!["HTTP 404".to_string()]));
        assert!(merge(Err("a".into()), Err("b".into())).is_err());
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

        fn run(&self, repo: &str, run_id: u64) -> Result<String> {
            let (status, conclusion) = match run_id {
                777 => ("in_progress", "null".to_string()),
                778 => ("completed", "\"success\"".to_string()),
                _ => bail!("Not Found"),
            };
            Ok(format!(
                r#"{{"id":{run_id},"name":"Build","display_title":"Build web","head_branch":"main","html_url":"https://github.com/{repo}/actions/runs/{run_id}","status":"{status}","conclusion":{conclusion},"created_at":"2026-10-01T00:00:00Z","updated_at":"2026-10-01T00:05:00Z"}}"#
            ))
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

    struct CountingRuns {
        calls: AtomicUsize,
    }

    impl RunSource for CountingRuns {
        fn fetch(&self, repos: &[String]) -> Result<Batch> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut batch = Batch::default();
            batch.runs = parse_runs(&repos[0], RUNS)?;
            Ok(batch)
        }
    }

    #[test]
    fn copies_watching_the_same_repos_share_one_fetch_whatever_the_order() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = || Cache::open(dir.path().to_path_buf()).unwrap();
        let (a, b, c) = (
            Arc::new(CountingRuns { calls: AtomicUsize::new(0) }),
            Arc::new(CountingRuns { calls: AtomicUsize::new(0) }),
            Arc::new(CountingRuns { calls: AtomicUsize::new(0) }),
        );
        let cadence = RunCadence::default();
        let first = SharedSource { inner: a.clone(), cache: cache(), cadence };
        let second = SharedSource { inner: b.clone(), cache: cache(), cadence };
        let third = SharedSource { inner: c.clone(), cache: cache(), cadence };
        let repos = |names: &[&str]| names.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let got = first.fetch(&repos(&["acme/api", "acme/widgets"])).unwrap();
        let again = second.fetch(&repos(&["acme/widgets", "acme/api"])).unwrap();
        assert_eq!(got.runs, again.runs);
        third.fetch(&repos(&["acme/api"])).unwrap();
        assert_eq!(a.calls.load(Ordering::SeqCst), 1);
        assert_eq!(b.calls.load(Ordering::SeqCst), 0, "same repos in another order: no second fetch");
        assert_eq!(c.calls.load(Ordering::SeqCst), 1, "a different repo list is its own entry");
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
        let (rx, wake) = spawn(source.clone(), repos.clone(), Arc::new(Mutex::new(Vec::new())), cadence);
        assert!(next_batch(&rx).unwrap().runs.is_empty());
        assert_eq!(source.calls.load(Ordering::SeqCst), 0, "no repos: nothing is fetched");
        *repos.lock().unwrap() = vec!["acme/widgets".to_string()];
        wake.send(()).unwrap();
        assert!(next_batch(&rx).is_ok());
        assert_eq!(*source.seen.lock().unwrap(), vec![vec!["acme/widgets".to_string()]]);
    }
}
