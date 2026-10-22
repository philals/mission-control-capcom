//! What the web service keeps besides the PR list: settings shared with the TUI, runs, the followed
//! PRs and runs, learned check durations, what the autopilot has done, and the tools it uses.
use super::*;
use crate::durations::Durations;
use crate::fixstate;
use crate::prs::Pasted;
use crate::refresh::PrLookup;
use crate::runs::Run;
use crate::settings::{self, Settings};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;

#[derive(Default)]
pub(super) struct RunData {
    pub items: Vec<Run>,
    pub error: Option<String>,
    pub updated: Option<DateTime<Utc>>,
    pub loading: bool,
}

/// Something capcom did or noticed by itself, for the page to list.
#[derive(Clone)]
pub(super) struct Event {
    pub at: DateTime<Utc>,
    pub text: String,
}

const KEEP_EVENTS: usize = 30;

/// What the autopilot has already done, so it never does the same thing twice.
#[derive(Default)]
pub(super) struct AutoState {
    /// PRs Copilot was asked about this run.
    pub copilot_tried: HashSet<String>,
    /// The state of each PR that was last found wanting.
    pub fix_seen: HashMap<String, String>,
    pub behind_asked: HashMap<String, std::time::Instant>,
    /// PRs that were green and waiting for a reviewer at the last look (None until the first look).
    pub awaiting: Option<HashSet<String>>,
    /// When Copilot last reviewed each PR, as of the last look (None until the first look).
    pub copilot_seen: Option<HashMap<String, Option<String>>>,
    /// Runs that were active at the last look, to tell when one finishes (None until the first look).
    pub active_runs: Option<HashSet<(String, u64)>>,
}

type Done = Result<(), String>;

/// The things the service does through GitHub and the desktop. The defaults refuse; `main` installs the real ones.
#[derive(Clone)]
pub struct Tools {
    pub ready: Arc<dyn Fn(&str) -> Done + Send + Sync>,
    pub draft: Arc<dyn Fn(&str) -> Done + Send + Sync>,
    pub copilot: Arc<dyn Fn(&str) -> Done + Send + Sync>,
    /// repo, number, label, on
    pub label: Arc<dyn Fn(&str, u64, &str, bool) -> Done + Send + Sync>,
    /// repo, number, the head it saw
    pub update_branch: Arc<dyn Fn(&str, u64, &str) -> Done + Send + Sync>,
    /// title, text
    pub notify: Arc<dyn Fn(&str, &str) + Send + Sync>,
    pub lookup: Arc<dyn PrLookup + Send + Sync>,
}

struct Refuse;

impl PrLookup for Refuse {
    fn state(&self, _url: &str) -> anyhow::Result<crate::model::PrState> {
        anyhow::bail!("not set up to look at GitHub")
    }
}

impl Default for Tools {
    fn default() -> Tools {
        let no = |what: &'static str| -> Done { Err(format!("{what} is not set up here")) };
        Tools {
            ready: Arc::new(move |_| no("marking ready")),
            draft: Arc::new(move |_| no("marking as draft")),
            copilot: Arc::new(move |_| no("asking Copilot")),
            label: Arc::new(move |_, _, _, _| no("labelling")),
            update_branch: Arc::new(move |_, _, _| no("updating a branch")),
            notify: Arc::new(|_, _| {}),
            lookup: Arc::new(Refuse),
        }
    }
}

impl Tools {
    /// The real ones: `gh` and the GitHub API, and `notify-send` for desktop notifications.
    pub fn real() -> Tools {
        use crate::actions::*;
        Tools {
            ready: Arc::new(|url| gh_mark_ready(url)),
            draft: Arc::new(|url| gh_mark_draft(url)),
            copilot: Arc::new(|url| gh_request_copilot(url)),
            label: Arc::new(|repo, number, label, on| gh_set_label(repo, number, label, on)),
            update_branch: Arc::new(|repo, number, head| gh_update_branch(repo, number, head)),
            notify: Arc::new(|title, body| {
                let (title, body) = (title.to_string(), body.to_string());
                std::thread::spawn(move || {
                    let _ = Command::new("notify-send").args(["--app-name=CAPCOM", &title, &body]).status();
                });
            }),
            lookup: Arc::new(crate::refresh::GhLookup),
        }
    }
}

pub(super) struct Engine {
    pub settings: Mutex<Settings>,
    pub settings_path: Mutex<Option<PathBuf>>,
    pub runs: Mutex<RunData>,
    pub run_repos: Arc<Mutex<Vec<String>>>,
    pub run_watched: Arc<Mutex<Vec<(String, u64)>>>,
    pub run_wake: Mutex<Option<Sender<()>>>,
    pub extra_repos: Mutex<Vec<String>>,
    pub pasted: Arc<Mutex<Pasted>>,
    pub durations: Mutex<Durations>,
    pub durations_path: Mutex<Option<PathBuf>>,
    /// The PRs and the Claude Code session that made each.
    pub sessions: Mutex<Vec<(String, PrSession)>>,
    pub workdir: Mutex<PathBuf>,
    pub tools: Mutex<Tools>,
    pub auto: Mutex<AutoState>,
    pub fix_store: Mutex<Option<fixstate::Store>>,
    pub events: Mutex<Vec<Event>>,
    pub runs_on: Mutex<bool>,
    /// The state GitHub gave each recorded PR at the last look.
    pub known: Mutex<HashMap<String, crate::model::PrState>>,
    /// Look at the recorded PRs now.
    pub check_now: std::sync::atomic::AtomicBool,
}

impl Default for Engine {
    fn default() -> Engine {
        Engine {
            settings: Mutex::new(Settings::default()),
            settings_path: Mutex::new(None),
            runs: Mutex::new(RunData::default()),
            run_repos: Arc::new(Mutex::new(Vec::new())),
            run_watched: Arc::new(Mutex::new(Vec::new())),
            run_wake: Mutex::new(None),
            extra_repos: Mutex::new(Vec::new()),
            pasted: Arc::new(Mutex::new(Pasted::default())),
            durations: Mutex::new(Durations::default()),
            durations_path: Mutex::new(None),
            sessions: Mutex::new(Vec::new()),
            workdir: Mutex::new(std::env::current_dir().unwrap_or_default()),
            tools: Mutex::new(Tools::default()),
            auto: Mutex::new(AutoState::default()),
            fix_store: Mutex::new(None),
            events: Mutex::new(Vec::new()),
            runs_on: Mutex::new(false),
            known: Mutex::new(HashMap::new()),
            check_now: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

pub(super) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Server {
    pub(super) fn tools(&self) -> Tools {
        lock(&self.eng.tools).clone()
    }

    /// Install the tools the service uses to act (see [`Tools::real`]).
    pub fn set_tools(&self, tools: Tools) {
        *lock(&self.eng.tools) = tools;
    }

    /// The folder new Claude Code sessions start in.
    pub fn set_workdir(&self, dir: PathBuf) {
        *lock(&self.eng.workdir) = dir;
    }

    pub fn set_extra_repos(&self, repos: Vec<String>) {
        *lock(&self.eng.extra_repos) = repos;
    }

    /// Keep the settings in this file, the same one the TUI keeps its own in, and follow what they say now.
    pub fn load_settings(&self, path: Option<PathBuf>) {
        if let Some(p) = &path {
            let loaded = settings::load(p);
            self.apply_settings(&loaded);
            *lock(&self.eng.settings) = loaded;
        }
        *lock(&self.eng.settings_path) = path;
    }

    /// Where the learned check durations are kept (shared with the TUI).
    pub fn load_durations(&self, path: Option<PathBuf>) {
        if let Some(p) = &path {
            *lock(&self.eng.durations) = Durations::load(p);
        }
        *lock(&self.eng.durations_path) = path;
    }

    pub fn set_fix_store(&self, store: Option<fixstate::Store>) {
        *lock(&self.eng.fix_store) = store;
    }

    /// A PR source that adds the PRs followed by link to what `inner` lists.
    pub fn with_followed(&self, gh: Arc<prs::GhSource>, inner: Arc<dyn PrSource>) -> Arc<dyn PrSource> {
        Arc::new(prs::WithPasted { inner, gh, pasted: self.eng.pasted.clone() })
    }

    /// Keep the index of which Claude Code session made which PR up to date while a page looks.
    pub fn watch_sessions(self: &Arc<Self>, claude_config: PathBuf, index_file: Option<PathBuf>, every: Duration, tick: Duration) {
        let me = self.clone();
        std::thread::spawn(move || {
            let mut index = index_file.as_deref().map(crate::session::Index::load).unwrap_or_default();
            let mut read: Option<std::time::Instant> = None;
            loop {
                if (read.is_none() || me.looking()) && read.is_none_or(|t| t.elapsed() >= every) {
                    read = Some(std::time::Instant::now());
                    if index.refresh(&claude_config) {
                        if let Some(file) = &index_file {
                            let _ = index.save(file);
                        }
                    }
                    *lock(&me.eng.sessions) = index.sessions();
                }
                std::thread::sleep(tick);
            }
        });
    }

    pub(super) fn settings(&self) -> Settings {
        lock(&self.eng.settings).clone()
    }

    /// What the settings say that the pollers need: the followed PRs and runs.
    fn apply_settings(&self, s: &Settings) {
        lock(&self.eng.pasted).urls = s.pasted_prs.clone();
        *lock(&self.eng.run_watched) = s.watched.iter().map(|w| (w.repo.clone(), w.id)).collect();
        self.attention.set_slow_when_away(s.slow_when_away);
    }

    /// Change the settings: read the file again first (the TUI may have changed it), change, save.
    pub(super) fn change_settings(&self, change: impl FnOnce(&mut Settings)) -> Settings {
        let path = lock(&self.eng.settings_path).clone();
        let mut current = path.as_deref().map_or_else(|| self.settings(), settings::load);
        change(&mut current);
        if let Some(p) = &path {
            let _ = settings::save(p, &current);
        }
        self.apply_settings(&current);
        *lock(&self.eng.settings) = current.clone();
        self.wake_feeds();
        current
    }

    /// Have the PR and run pollers look now.
    pub(super) fn wake_feeds(&self) {
        if let Some(wake) = lock(&self.wake).as_ref() {
            let _ = wake.send(());
        }
        if let Some(wake) = lock(&self.eng.run_wake).as_ref() {
            let _ = wake.send(());
        }
    }

    /// Record something the service did or saw, for the page to list.
    pub(super) fn event(&self, text: impl Into<String>) {
        let mut events = lock(&self.eng.events);
        events.push(Event { at: Utc::now(), text: text.into() });
        let extra = events.len().saturating_sub(KEEP_EVENTS);
        events.drain(..extra);
    }

    pub(super) fn notify_if_wanted(&self, title: &str, body: &str) {
        self.event(format!("{title}: {body}"));
        if self.settings().notify {
            (self.tools().notify)(title, body);
        }
    }
}
