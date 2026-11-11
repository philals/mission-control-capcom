//! The apps capcom-tui can start and reach agents in besides Herdr (the Herdr app, Orca and the Claude desktop app),
//! and the router that starts new agents in the app capcom-tui runs in and reaches each running agent
//! in the app it runs in.
use crate::agents::AgentInfo;
use crate::herdr::{Herdr, Launch, Outcome, Resume};
use capcom::herdr_app::{HerdrApp, Start};
use capcom::host::Host;
use capcom::orca::Orca;
use capcom::starting::Starting;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub type Opener = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
pub type Live = Box<dyn Fn() -> Vec<AgentInfo> + Send + Sync>;

pub struct DesktopHost {
    pub open: Opener,
    /// The live Claude Code sessions (`capcom::claude::live`).
    pub live: Live,
    pub workdir: PathBuf,
}

pub struct OrcaHost {
    pub orca: Orca,
    /// Live sessions started in Orca terminals, by Orca pane key.
    pub sessions: Box<dyn Fn() -> HashMap<String, String> + Send + Sync>,
    pub workdir: PathBuf,
}

pub struct HerdrAppHost {
    pub app: HerdrApp,
    /// The `capcom::claude::live` sessions.
    pub live: Live,
    pub workdir: PathBuf,
}

pub struct Router {
    pub home: Host,
    pub hosts: Vec<(Host, Arc<dyn Herdr>)>,
    pub starting: Option<Starting>,
    pub now: fn() -> u64,
}

/// The folder a launch starts in: the task's folder when it exists, else the work folder.
fn start_folder(l: &Launch, workdir: &Path) -> PathBuf {
    l.folder.as_deref().map(PathBuf::from).filter(|f| f.is_dir()).unwrap_or_else(|| workdir.to_path_buf())
}

fn holds(a: &AgentInfo, session: &str) -> bool {
    a.session.as_deref() == Some(session)
}

fn no_session() -> String {
    "no session is recorded for this task yet".into()
}

fn gone(cwd: &str) -> String {
    format!("its session started in {cwd}, which no longer exists")
}

impl DesktopHost {
    fn running(&self) -> Vec<AgentInfo> {
        (self.live)().into_iter().filter(|a| capcom::claude::entrypoint_of(&a.pane) == "claude-desktop").collect()
    }
}

impl Herdr for DesktopHost {
    fn launch(&self, l: &Launch) -> Result<Outcome, String> {
        (self.open)(&capcom::claude::new_session_link(&l.prompt, &start_folder(l, &self.workdir)))?;
        Ok(Outcome::Started)
    }

    fn resume(&self, r: &Resume) -> Result<Outcome, String> {
        if let Some((id, _)) = &r.session {
            if let Some(a) = self.running().into_iter().find(|a| holds(a, id)) {
                // the desktop app cannot be typed into from outside: a new session in the same folder gets the prompt
                if let Some(prompt) = &r.prompt {
                    (self.open)(&capcom::claude::new_session_link(prompt, Path::new(&a.cwd)))?;
                    return Ok(Outcome::Prompted);
                }
                let link = capcom::claude::desktop_link(&a.pane).ok_or("the desktop app gave this session no id to open it by")?;
                (self.open)(&link)?;
                return Ok(Outcome::Focused);
            }
        }
        match (&r.session, &r.fresh_repo, &r.prompt) {
            (Some((id, cwd)), _, prompt) => {
                if !Path::new(cwd).is_dir() {
                    return Err(gone(cwd));
                }
                let link = match prompt {
                    Some(p) => capcom::claude::new_session_link(p, Path::new(cwd)),
                    None => capcom::claude::import_link(id).ok_or("that session id does not look right")?,
                };
                (self.open)(&link)?;
                Ok(Outcome::Started)
            }
            (None, Some(repo), Some(prompt)) => {
                let dir = self.workdir.join(repo);
                if !dir.is_dir() {
                    return Err(format!("no session is recorded for this PR and {} does not exist to start a new one in", dir.display()));
                }
                (self.open)(&capcom::claude::new_session_link(prompt, &dir))?;
                Ok(Outcome::Started)
            }
            _ => Err(no_session()),
        }
    }

    fn agents(&self) -> Result<Vec<AgentInfo>, String> {
        Ok(self.running())
    }

    fn focus(&self, pane: &str) -> Result<(), String> {
        (self.open)(&capcom::claude::desktop_link(pane).ok_or("the desktop app gave this session no id to open it by")?)
    }
}

impl Herdr for OrcaHost {
    /// A task starts in its own folder, never quietly in another repo: one with no folder, or a folder
    /// that has gone, is refused. Story-wide work (review, plan) starts in the work folder.
    fn launch(&self, l: &Launch) -> Result<Outcome, String> {
        if let Some(task) = &l.task {
            capcom::orca::check_task_folder(&l.workspace, task, l.folder.as_deref())?;
        }
        self.orca.launch(&start_folder(l, &self.workdir).to_string_lossy(), &l.prompt, true)?;
        Ok(Outcome::Started)
    }

    fn resume(&self, r: &Resume) -> Result<Outcome, String> {
        if let Some((id, _)) = &r.session {
            let agents = self.orca.agents(&(self.sessions)())?;
            if let Some(a) = agents.iter().find(|a| holds(a, id)) {
                let handle = capcom::orca::handle_of(&a.pane).unwrap_or_default();
                if let Some(prompt) = &r.prompt {
                    if a.status == crate::agents::Status::Working {
                        return Ok(Outcome::Busy);
                    }
                    self.orca.send(handle, prompt)?;
                    if r.focus {
                        self.orca.switch(handle)?;
                    }
                    return Ok(Outcome::Prompted);
                }
                self.orca.switch(handle)?;
                return Ok(Outcome::Focused);
            }
        }
        match (&r.session, &r.fresh_repo, &r.prompt) {
            (Some((id, cwd)), _, prompt) => {
                if !Path::new(cwd).is_dir() {
                    return Err(gone(cwd));
                }
                let checkout = self.orca.checkout_of(cwd)?;
                self.orca.resume(&checkout, cwd, id, prompt.as_deref(), r.focus)?;
                Ok(Outcome::Started)
            }
            (None, Some(repo), Some(prompt)) => {
                let dir = self.workdir.join(repo);
                if !dir.is_dir() {
                    return Err(format!("no session is recorded for this PR and {} does not exist to start a new one in", dir.display()));
                }
                self.orca.launch(&dir.to_string_lossy(), prompt, r.focus)?;
                Ok(Outcome::Started)
            }
            _ => Err(no_session()),
        }
    }

    fn agents(&self) -> Result<Vec<AgentInfo>, String> {
        self.orca.agents(&(self.sessions)())
    }

    fn focus(&self, pane: &str) -> Result<(), String> {
        self.orca.switch(capcom::orca::handle_of(pane).ok_or("not an Orca agent")?)
    }

    fn close(&self, pane: &str) -> Result<(), String> {
        self.orca.close(capcom::orca::handle_of(pane).ok_or("not an Orca agent")?)
    }
}

impl HerdrAppHost {
    fn agents_now(&self) -> Result<Vec<AgentInfo>, String> {
        self.app.agents(&capcom::herdr_app::sessions_by_pid(&(self.live)()))
    }

    fn start(&self, l: &Resume, folder: &Path, argv: Vec<String>, prompt: Option<&str>) -> Result<Outcome, String> {
        let folder = folder.to_string_lossy();
        self.app.start(&Start { space: &l.workspace, label: &l.label, tab: &l.tab, folder: &folder, argv, prompt, focus: l.focus })?;
        Ok(Outcome::Started)
    }
}

impl Herdr for HerdrAppHost {
    /// The app cannot name an agent, so one already running is found by its session (the router does
    /// that before it gets here), not by name.
    fn launch(&self, l: &Launch) -> Result<Outcome, String> {
        let folder = start_folder(l, &self.workdir);
        let start = Start {
            space: &l.workspace,
            label: &l.label,
            tab: &l.tab,
            folder: &folder.to_string_lossy(),
            argv: vec!["claude".into()],
            prompt: Some(&l.prompt),
            focus: false,
        };
        self.app.start(&start)?;
        Ok(Outcome::Started)
    }

    fn resume(&self, r: &Resume) -> Result<Outcome, String> {
        if let Some((id, _)) = &r.session {
            if let Some(a) = self.agents_now()?.iter().find(|a| holds(a, id)) {
                let terminal = capcom::herdr_app::terminal_of(&a.pane).unwrap_or_default();
                if let Some(prompt) = &r.prompt {
                    if a.status == crate::agents::Status::Working {
                        return Ok(Outcome::Busy);
                    }
                    self.app.send(terminal, prompt)?;
                    if r.focus {
                        self.app.focus(terminal)?;
                    }
                    return Ok(Outcome::Prompted);
                }
                self.app.focus(terminal)?;
                return Ok(Outcome::Focused);
            }
        }
        match (&r.session, &r.fresh_repo, &r.prompt) {
            (Some((id, cwd)), _, prompt) => {
                if !Path::new(cwd).is_dir() {
                    return Err(gone(cwd));
                }
                self.start(r, Path::new(cwd), vec!["claude".into(), "--resume".into(), id.clone()], prompt.as_deref())
            }
            (None, Some(repo), Some(prompt)) => {
                let dir = self.workdir.join(repo);
                if !dir.is_dir() {
                    return Err(format!("no session is recorded for this PR and {} does not exist to start a new one in", dir.display()));
                }
                self.start(r, &dir, vec!["claude".into()], Some(prompt))
            }
            _ => Err(no_session()),
        }
    }

    fn agents(&self) -> Result<Vec<AgentInfo>, String> {
        self.agents_now()
    }

    fn focus(&self, pane: &str) -> Result<(), String> {
        self.app.focus(capcom::herdr_app::terminal_of(pane).ok_or("not a Herdr app agent")?)
    }

    fn close(&self, pane: &str) -> Result<(), String> {
        self.app.close(capcom::herdr_app::terminal_of(pane).ok_or("not a Herdr app agent")?)
    }
}

impl Router {
    fn host(&self, host: Host) -> Option<&Arc<dyn Herdr>> {
        self.hosts.iter().find(|(h, _)| *h == host).map(|(_, x)| x)
    }

    fn of_pane(&self, pane: &str) -> Result<&Arc<dyn Herdr>, String> {
        let app = capcom::host::of_pane(pane);
        self.host(app).ok_or_else(|| format!("it's running in {}; switch to {} to reach it", app.name(), app.name()))
    }

    fn home(&self) -> Result<&Arc<dyn Herdr>, String> {
        self.host(self.home).ok_or_else(|| match self.home {
            Host::Orca => "Orca: its command-line tool was not found (open Orca, or register its CLI)".to_string(),
            Host::HerdrApp => "Herdr app: it has no window open to start anything in (open it, then try again)".to_string(),
            h => format!("{} cannot be reached", h.name()),
        })
    }

    /// The app running one of these sessions, and the agent.
    fn running(&self, sessions: &[String]) -> Option<(&Arc<dyn Herdr>, AgentInfo)> {
        self.hosts.iter().find_map(|(_, host)| {
            let agents = host.agents().ok()?;
            let a = agents.into_iter().find(|a| a.session.as_ref().is_some_and(|s| sessions.contains(s)))?;
            Some((host, a))
        })
    }
}

impl Herdr for Router {
    fn launch(&self, l: &Launch) -> Result<Outcome, String> {
        let now = (self.now)();
        let mark = l.task.as_deref().zip(self.starting.as_ref());
        let clear = || {
            if let Some((task, marks)) = mark {
                marks.clear(&l.workspace, task);
            }
        };
        // the mark is taken first: looking through every app takes a while, and a second click meanwhile
        // must find it taken
        let taken = match mark {
            Some((task, marks)) if !marks.mark(&l.workspace, task, self.home, now) => marks.live(&l.workspace, task, now),
            _ => None,
        };
        if let Some((host, a)) = self.running(&l.sessions) {
            if taken.is_none() {
                clear();
            }
            host.focus(&a.pane)?;
            return Ok(Outcome::Focused);
        }
        if let Some(h) = taken {
            return Ok(Outcome::AlreadyStarting(h));
        }
        let home = match self.home() {
            Ok(h) => h,
            Err(e) => {
                clear();
                return Err(e);
            }
        };
        let out = home.launch(l);
        if !matches!(out, Ok(Outcome::Started)) {
            if let Some((task, marks)) = mark {
                marks.clear(&l.workspace, task);
            }
        }
        out
    }

    fn resume(&self, r: &Resume) -> Result<Outcome, String> {
        if let Some((id, _)) = &r.session {
            if let Some((host, _)) = self.running(std::slice::from_ref(id)) {
                return host.resume(r);
            }
        }
        self.home()?.resume(r)
    }

    fn agents(&self) -> Result<Vec<AgentInfo>, String> {
        Ok(self.hosts.iter().filter_map(|(_, h)| h.agents().ok()).flatten().collect())
    }

    fn focus(&self, pane: &str) -> Result<(), String> {
        self.of_pane(pane)?.focus(pane)
    }

    fn close(&self, pane: &str) -> Result<(), String> {
        self.of_pane(pane)?.close(pane)
    }
}

/// Open a link with the system's opener (`xdg-open`, or `open` on macOS), without waiting for it.
pub fn open_link(url: &str) -> Result<(), String> {
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let mut child = std::process::Command::new(program)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn unix_now() -> u64 {
    chrono::Utc::now().timestamp().max(0) as u64
}

/// The router for a TUI running in `home`; None in a plain terminal, where nothing can be started.
/// Herdr is reachable from anywhere while it runs (its socket is in a fixed place), so it is included
/// whenever it answers, and the Herdr app likewise while it has a window open; Orca whenever its CLI is
/// found; the desktop app always.
pub fn router(home: Host, workdir: PathBuf) -> Option<Router> {
    if home == Host::Plain {
        return None;
    }
    let config = capcom::session::config_dir();
    let live = move || config.as_deref().map(capcom::claude::live).unwrap_or_default();
    let mut hosts: Vec<(Host, Arc<dyn Herdr>)> = Vec::new();
    if home == Host::Herdr || crate::herdr::answers() {
        hosts.push((Host::Herdr, Arc::new(crate::herdr::Cli::new(workdir.clone()))));
    }
    if home == Host::HerdrApp || capcom::herdr_app::answers() {
        let live = live.clone();
        hosts.push((Host::HerdrApp, Arc::new(HerdrAppHost { app: HerdrApp::new(), live: Box::new(live), workdir: workdir.clone() })));
    }
    if let Some(cli) = capcom::orca::find_cli_here() {
        let live = live.clone();
        hosts.push((
            Host::Orca,
            Arc::new(OrcaHost {
                orca: Orca::new(cli),
                sessions: Box::new(move || capcom::orca::sessions_by_pane_key(&live(), capcom::orca::environ)),
                workdir: workdir.clone(),
            }),
        ));
    }
    hosts.push((Host::Desktop, Arc::new(DesktopHost { open: Arc::new(open_link), live: Box::new(live), workdir })));
    Some(Router { home, hosts, starting: Starting::default(), now: unix_now })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::Status;
    use serde_json::{json, Value};
    use std::sync::Mutex;

    type Calls = Arc<Mutex<Vec<String>>>;

    fn agent(pane: &str, session: &str, status: Status, cwd: &str) -> AgentInfo {
        AgentInfo { pane: pane.into(), name: None, status, topic: String::new(), workspace: String::new(), cwd: cwd.into(), session: Some(session.into()) }
    }

    /// A stand-in for any app: records what it is asked and lists the agents it is given.
    struct Fake {
        tag: &'static str,
        agents: Vec<AgentInfo>,
        calls: Calls,
        fail: bool,
    }

    impl Herdr for Fake {
        fn launch(&self, l: &Launch) -> Result<Outcome, String> {
            self.calls.lock().unwrap().push(format!("{} launch {}", self.tag, l.prompt));
            if self.fail {
                return Err(format!("{} is not running", self.tag));
            }
            Ok(Outcome::Started)
        }
        fn resume(&self, r: &Resume) -> Result<Outcome, String> {
            self.calls.lock().unwrap().push(format!("{} resume {}", self.tag, r.prompt.clone().unwrap_or_default()));
            Ok(Outcome::Focused)
        }
        fn agents(&self) -> Result<Vec<AgentInfo>, String> {
            if self.fail {
                return Err("down".into());
            }
            Ok(self.agents.clone())
        }
        fn focus(&self, pane: &str) -> Result<(), String> {
            self.calls.lock().unwrap().push(format!("{} focus {pane}", self.tag));
            Ok(())
        }
        fn close(&self, pane: &str) -> Result<(), String> {
            self.calls.lock().unwrap().push(format!("{} close {pane}", self.tag));
            Ok(())
        }
    }

    fn fake(tag: &'static str, agents: Vec<AgentInfo>, calls: &Calls, fail: bool) -> Arc<dyn Herdr> {
        Arc::new(Fake { tag, agents, calls: calls.clone(), fail })
    }

    fn now() -> u64 {
        1_000
    }

    fn router_of(home: Host, hosts: Vec<(Host, Arc<dyn Herdr>)>, dir: &tempfile::TempDir) -> Router {
        Router { home, hosts, starting: Some(Starting::new(dir.path().join("starting.json"))), now }
    }

    fn launch(folder: Option<&str>) -> Launch {
        Launch {
            workspace: "K-1".into(),
            label: "K-1 - Thing".into(),
            tab: "T2 implement".into(),
            agent: "k-1-t2-impl".into(),
            prompt: "/story-implement-task K-1 T2".into(),
            folder: folder.map(str::to_string),
            task: Some("T2".into()),
            sessions: vec!["s-1".into()],
        }
    }

    fn resume(session: Option<(&str, &str)>, prompt: Option<&str>, focus: bool) -> Resume {
        Resume {
            workspace: "K-1".into(),
            label: "K-1 - Thing".into(),
            tab: "T2 resume".into(),
            agent: "k-1-t2-resume".into(),
            known_agents: Vec::new(),
            session: session.map(|(a, b)| (a.to_string(), b.to_string())),
            prompt: prompt.map(str::to_string),
            focus,
            fresh_repo: None,
        }
    }

    fn opener() -> (Opener, Calls) {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let log = calls.clone();
        (Arc::new(move |url: &str| {
            log.lock().unwrap().push(url.to_string());
            Ok(())
        }), calls)
    }

    fn desktop(agents: Vec<AgentInfo>, workdir: &std::path::Path) -> (DesktopHost, Calls) {
        let (open, calls) = opener();
        (DesktopHost { open, live: Box::new(move || agents.clone()), workdir: workdir.to_path_buf() }, calls)
    }

    fn orca_runner(calls: &Calls, fail: bool, ps: Value, known: String) -> capcom::orca::Runner {
        let log = calls.clone();
        Box::new(move |args: &[String]| {
            log.lock().unwrap().push(args.join(" "));
            if fail {
                return Err("Orca CLI is unavailable; reopen Orca or register the CLI again.".into());
            }
            Ok(match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
                ["worktree", "list", ..] => json!({"ok": true, "result": {"worktrees": [{"path": "/w/app"}, {"path": known}]}}),
                ["worktree", "ps", ..] => ps.clone(),
                ["terminal", "list", ..] => json!({"ok": true, "result": {"terminals": [{"handle": "term_9", "tabId": "t", "leafId": "1"}]}}),
                ["agent", "launch", ..] => json!({"ok": true, "result": {"outcome": {"handle": "term_9"}}}),
                ["terminal", "create", ..] => json!({"ok": true, "result": {"terminal": {"handle": "term_7"}}}),
                _ => json!({"ok": true, "result": {}}),
            })
        })
    }

    fn orca_host(fail: bool, state: &str, workdir: &std::path::Path) -> (OrcaHost, Calls) {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let ps = json!({"ok": true, "result": {"worktrees": [{"path": "/w/app", "displayName": "main", "agents": [
            {"paneKey": "t:1", "state": state, "agentType": "claude"}]}]}});
        let host = OrcaHost {
            orca: Orca::with_runner(orca_runner(&calls, fail, ps, workdir.to_string_lossy().to_string())),
            sessions: Box::new(|| HashMap::from([("t:1".to_string(), "s-1".to_string())])),
            workdir: workdir.to_path_buf(),
        };
        (host, calls)
    }

    fn app_host(fail: bool, status: &str, workdir: &std::path::Path) -> (HerdrAppHost, Calls) {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let (log, status) = (calls.clone(), status.to_string());
        let run: capcom::herdr_app::Runner = Box::new(move |program, args| {
            log.lock().unwrap().push(format!("{program} {}", args.join(" ")));
            if fail {
                return Err("no window is open".into());
            }
            Ok(match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
                ["ls"] => json!({"spaces": [{"id": "sp1", "name": "K-1 - Thing", "tabs": [{"id": "tb1", "name": "1", "terminals": [{"terminal": "t1", "status": status, "app": "claude-code", "name": null}]}]}]}),
                ["status"] => json!({"space": "sp1", "tab": "tb1"}),
                ["do", "tab.new" | "space.new", ..] => json!({"terminal": "t9"}),
                ["get", "t1"] => json!({"cwd": "/w/app", "title": "Claude Code", "foreground": {"pid": 11}}),
                _ => json!({"ok": true}),
            })
        });
        let live = vec![AgentInfo { pane: "cli:11".into(), name: None, status: Status::Idle, topic: String::new(), workspace: String::new(), cwd: "/w/app".into(), session: Some("s-1".into()) }];
        (HerdrAppHost { app: HerdrApp::with_runner(run), live: Box::new(move || live.clone()), workdir: workdir.to_path_buf() }, calls)
    }

    #[test]
    fn launching_from_the_herdr_app_starts_claude_in_a_tab_of_the_story_space_in_the_task_folder() {
        let work = tempfile::tempdir().unwrap();
        let folder = work.path().join("repo");
        std::fs::create_dir(&folder).unwrap();
        let (h, calls) = app_host(false, "idle", work.path());
        assert_eq!(h.launch(&launch(Some(&folder.to_string_lossy()))), Ok(Outcome::Started));
        let all = calls.lock().unwrap().clone();
        let made = all.iter().find(|c| c.contains("tab.new")).expect("a tab was made");
        assert!(made.contains(&format!("--folder {}", folder.display())) && made.contains("--argv [\"claude\"]") && made.contains("--space sp1"), "{made}");
    }

    #[test]
    fn the_herdr_app_not_answering_is_reported_as_it() {
        let work = tempfile::tempdir().unwrap();
        let (h, _) = app_host(true, "idle", work.path());
        assert_eq!(h.launch(&launch(None)), Err("Herdr app: no window is open".to_string()));
    }

    #[test]
    fn a_herdr_app_agent_with_the_session_is_focused_or_prompted() {
        let work = tempfile::tempdir().unwrap();
        let (h, calls) = app_host(false, "idle", work.path());
        assert_eq!(h.resume(&resume(Some(("s-1", "/w/app")), None, false)), Ok(Outcome::Focused));
        assert_eq!(h.resume(&resume(Some(("s-1", "/w/app")), Some("/pr-address x"), false)), Ok(Outcome::Prompted));
        let all = calls.lock().unwrap().clone();
        assert!(all.contains(&"herdr-app do agent.focus --terminal t1".to_string()), "{all:?}");
        assert!(all.contains(&"hc send t1 /pr-address x".to_string()), "{all:?}");
        assert_eq!(all.iter().filter(|c| c.contains("agent.focus")).count(), 1, "an automatic prompt leaves the screen alone");
    }

    #[test]
    fn a_working_herdr_app_agent_is_not_prompted() {
        let work = tempfile::tempdir().unwrap();
        let (h, calls) = app_host(false, "working", work.path());
        assert_eq!(h.resume(&resume(Some(("s-1", "/w/app")), Some("/pr-address x"), false)), Ok(Outcome::Busy));
        assert!(!calls.lock().unwrap().iter().any(|c| c.starts_with("hc send")));
    }

    #[test]
    fn a_closed_session_is_resumed_in_a_herdr_app_tab_from_its_own_folder() {
        let work = tempfile::tempdir().unwrap();
        let folder = work.path().to_string_lossy().to_string();
        let (h, calls) = app_host(false, "idle", work.path());
        assert_eq!(h.resume(&resume(Some(("s-gone", &folder)), Some("/pr-address x"), true)), Ok(Outcome::Started));
        let all = calls.lock().unwrap().clone();
        let made = all.iter().find(|c| c.contains("tab.new")).expect("a tab was made");
        assert!(made.contains("--argv [\"claude\",\"--resume\",\"s-gone\"]") && made.contains("--task /pr-address x"), "{made}");
        assert!(!all.iter().any(|c| c.contains("tab.select")), "focus was asked for: {all:?}");
        assert_eq!(h.resume(&resume(Some(("s-gone", "/no/such/folder")), None, true)), Err(gone("/no/such/folder")));
        assert_eq!(h.resume(&resume(None, None, true)), Err(no_session()));
    }

    #[test]
    fn focus_and_close_go_to_the_herdr_app_by_terminal() {
        let work = tempfile::tempdir().unwrap();
        let (h, calls) = app_host(false, "idle", work.path());
        h.focus("herdr-app:t1").unwrap();
        h.close("herdr-app:t1").unwrap();
        assert_eq!(h.focus("orca:t1"), Err("not a Herdr app agent".to_string()));
        assert_eq!(*calls.lock().unwrap(), vec!["herdr-app do agent.focus --terminal t1", "herdr-app do terminal.kill --terminal t1"]);
    }

    #[test]
    fn launching_from_the_herdr_app_is_not_started_twice_when_its_session_runs() {
        let work = tempfile::tempdir().unwrap();
        let (h, calls) = app_host(false, "idle", work.path());
        let r = router_of(Host::HerdrApp, vec![(Host::HerdrApp, Arc::new(h))], &work);
        let mut l = launch(None);
        l.sessions = vec!["s-1".into()];
        assert_eq!(r.launch(&l), Ok(Outcome::Focused));
        assert!(!calls.lock().unwrap().iter().any(|c| c.contains("tab.new")));
    }

    #[test]
    fn launching_from_desktop_opens_a_new_session_link_in_the_task_folder() {
        let work = tempfile::tempdir().unwrap();
        let folder = work.path().join("api");
        std::fs::create_dir(&folder).unwrap();
        let (d, calls) = desktop(Vec::new(), work.path());
        assert_eq!(d.launch(&launch(Some(&folder.to_string_lossy()))), Ok(Outcome::Started));
        assert_eq!(calls.lock().unwrap()[0], capcom::claude::new_session_link("/story-implement-task K-1 T2", &folder));
        d.launch(&launch(Some("/gone/folder"))).unwrap();
        assert_eq!(calls.lock().unwrap()[1], capcom::claude::new_session_link("/story-implement-task K-1 T2", work.path()), "a missing folder falls back to the work folder");
    }

    #[test]
    fn a_running_desktop_session_is_opened_by_its_link() {
        let work = tempfile::tempdir().unwrap();
        let (d, calls) = desktop(vec![agent("claude-desktop:7:local_abc", "s-1", Status::Idle, "/w/app")], work.path());
        assert_eq!(d.resume(&resume(Some(("s-1", "/w/app")), None, true)), Ok(Outcome::Focused));
        assert_eq!(calls.lock().unwrap()[0], "claude://code/continue?session=local_abc");
    }

    #[test]
    fn a_closed_session_is_imported_into_the_desktop_app() {
        let work = tempfile::tempdir().unwrap();
        let (d, calls) = desktop(Vec::new(), work.path());
        let cwd = work.path().to_string_lossy().to_string();
        assert_eq!(d.resume(&resume(Some(("6bd808b2-6f76", &cwd)), None, true)), Ok(Outcome::Started));
        assert_eq!(calls.lock().unwrap()[0], "claude://resume?session=6bd808b2-6f76");
    }

    #[test]
    fn launching_from_orca_starts_a_terminal_agent_in_the_folder() {
        let work = tempfile::tempdir().unwrap();
        let w = work.path().to_string_lossy().to_string();
        let (o, calls) = orca_host(false, "idle", work.path());
        assert_eq!(o.launch(&launch(Some(&w))), Ok(Outcome::Started));
        assert_eq!(
            calls.lock().unwrap().last().unwrap(),
            &format!("agent launch --agent claude --surface terminal --worktree path:{w} --prompt /story-implement-task K-1 T2 --focus --json")
        );
    }

    #[test]
    fn an_orca_task_without_a_folder_says_so_and_starts_nothing() {
        let work = tempfile::tempdir().unwrap();
        let (o, calls) = orca_host(false, "idle", work.path());
        let none = o.launch(&launch(None)).unwrap_err();
        assert!(none.contains("T2 has no folder") && none.contains("capcom set-folder K-1 T2"), "{none}");
        let gone = o.launch(&launch(Some("/gone/folder"))).unwrap_err();
        assert!(gone.contains("/gone/folder") && gone.contains("no longer exists"), "{gone}");
        assert!(calls.lock().unwrap().is_empty(), "nothing asked of Orca");
        let mut review = launch(None);
        review.task = None;
        assert_eq!(o.launch(&review), Ok(Outcome::Started), "story-wide work starts in the work folder");
    }

    #[test]
    fn orca_not_running_is_reported_as_orca() {
        let work = tempfile::tempdir().unwrap();
        let (o, _) = orca_host(true, "idle", work.path());
        assert!(o.launch(&launch(Some(&work.path().to_string_lossy()))).unwrap_err().starts_with("Orca: "));
    }

    #[test]
    fn an_orca_agent_with_the_session_is_switched_to_or_prompted() {
        let work = tempfile::tempdir().unwrap();
        let (o, calls) = orca_host(false, "idle", work.path());
        assert_eq!(o.resume(&resume(Some(("s-1", "/w/app")), None, true)), Ok(Outcome::Focused));
        assert!(calls.lock().unwrap().iter().any(|c| c == "terminal switch --terminal term_9 --json"));
        assert_eq!(o.resume(&resume(Some(("s-1", "/w/app")), Some("/pr-address x"), true)), Ok(Outcome::Prompted));
        assert!(calls.lock().unwrap().iter().any(|c| c == "terminal send --terminal term_9 --text /pr-address x --enter --json"));
    }

    #[test]
    fn a_closed_session_is_resumed_in_an_orca_terminal_from_its_own_folder() {
        let work = tempfile::tempdir().unwrap();
        let w = work.path().to_string_lossy().to_string();
        let (o, calls) = orca_host(false, "idle", work.path());
        assert!(o.resume(&resume(Some(("s-2", "/nowhere/at/all")), None, true)).unwrap_err().contains("/nowhere/at/all"));
        assert_eq!(o.resume(&resume(Some(("s-2", &w)), None, true)), Ok(Outcome::Started));
        assert_eq!(calls.lock().unwrap().last().unwrap(), &format!("terminal create --worktree path:{w} --command cd '{w}' && claude --resume 's-2' --focus --json"));
        let elsewhere = tempfile::tempdir().unwrap();
        let unknown = o.resume(&resume(Some(("s-2", &elsewhere.path().to_string_lossy())), None, true)).unwrap_err();
        assert!(unknown.contains("orca repo add"), "{unknown}");
    }

    #[test]
    fn a_working_orca_agent_is_not_prompted() {
        let work = tempfile::tempdir().unwrap();
        let (o, calls) = orca_host(false, "working", work.path());
        assert_eq!(o.resume(&resume(Some(("s-1", "/w/app")), Some("/pr-address x"), false)), Ok(Outcome::Busy));
        assert!(!calls.lock().unwrap().iter().any(|c| c.starts_with("terminal send")));
    }

    #[test]
    fn launching_from_orca_uses_orca_never_the_desktop_link() {
        let dir = tempfile::tempdir().unwrap();
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let r = router_of(Host::Orca, vec![(Host::Orca, fake("orca", vec![], &calls, true)), (Host::Desktop, fake("desktop", vec![], &calls, false))], &dir);
        let err = r.launch(&launch(None)).unwrap_err();
        assert!(err.contains("orca is not running"), "{err}");
        assert!(!calls.lock().unwrap().iter().any(|c| c.starts_with("desktop launch")), "{calls:?}");
        assert_eq!(r.starting.as_ref().unwrap().live("K-1", "T2", now()), None, "a failed start leaves no mark");
    }

    #[test]
    fn an_agent_running_in_another_app_is_focused_not_started_again() {
        let dir = tempfile::tempdir().unwrap();
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let running = vec![agent("claude-desktop:7:local_abc", "s-1", Status::Idle, "/w/app")];
        let r = router_of(Host::Orca, vec![(Host::Orca, fake("orca", vec![], &calls, false)), (Host::Desktop, fake("desktop", running, &calls, false))], &dir);
        assert_eq!(r.launch(&launch(None)), Ok(Outcome::Focused));
        assert_eq!(*calls.lock().unwrap(), vec!["desktop focus claude-desktop:7:local_abc".to_string()]);
    }

    #[test]
    fn a_second_click_while_starting_reports_it_and_starts_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let r = router_of(Host::Desktop, vec![(Host::Desktop, fake("desktop", vec![], &calls, false))], &dir);
        assert_eq!(r.launch(&launch(None)), Ok(Outcome::Started));
        assert_eq!(r.launch(&launch(None)), Ok(Outcome::AlreadyStarting(Host::Desktop)));
        assert_eq!(calls.lock().unwrap().iter().filter(|c| c.contains("launch")).count(), 1);
    }

    /// An app that takes a while to list its agents, like Orca's CLI or Herdr's socket.
    struct Slow(Calls);

    impl Herdr for Slow {
        fn launch(&self, l: &Launch) -> Result<Outcome, String> {
            self.0.lock().unwrap().push(format!("launch {}", l.prompt));
            Ok(Outcome::Started)
        }
        fn resume(&self, _r: &Resume) -> Result<Outcome, String> {
            Ok(Outcome::Focused)
        }
        fn agents(&self) -> Result<Vec<AgentInfo>, String> {
            std::thread::sleep(std::time::Duration::from_millis(150));
            Ok(Vec::new())
        }
    }

    #[test]
    fn two_clicks_at_once_start_one_agent() {
        let dir = tempfile::tempdir().unwrap();
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let r = Arc::new(router_of(Host::Orca, vec![(Host::Orca, Arc::new(Slow(calls.clone())))], &dir));
        let clicks: Vec<_> = (0..2).map(|_| {
            let r = r.clone();
            std::thread::spawn(move || r.launch(&launch(None)))
        }).collect();
        let mut outcomes: Vec<String> = clicks.into_iter().map(|c| format!("{:?}", c.join().unwrap())).collect();
        outcomes.sort();
        assert_eq!(outcomes, ["Ok(AlreadyStarting(Orca))", "Ok(Started)"]);
        assert_eq!(calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_marked_task_whose_agent_is_now_running_is_brought_to_the_front() {
        let dir = tempfile::tempdir().unwrap();
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let running = vec![agent("w1:p2", "s-1", Status::Idle, "/w")];
        let r = router_of(Host::Herdr, vec![(Host::Herdr, fake("herdr", running, &calls, false))], &dir);
        r.starting.as_ref().unwrap().mark("K-1", "T2", Host::Herdr, now());
        assert_eq!(r.launch(&launch(None)), Ok(Outcome::Focused));
        assert_eq!(*calls.lock().unwrap(), vec!["herdr focus w1:p2"]);
    }

    #[test]
    fn story_wide_work_leaves_no_mark() {
        let dir = tempfile::tempdir().unwrap();
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let r = router_of(Host::Desktop, vec![(Host::Desktop, fake("desktop", vec![], &calls, false))], &dir);
        let mut review = launch(None);
        review.task = None;
        review.sessions = Vec::new();
        assert_eq!(r.launch(&review), Ok(Outcome::Started));
        assert_eq!(r.launch(&review), Ok(Outcome::Started));
    }

    #[test]
    fn focus_goes_to_the_agents_own_app() {
        let dir = tempfile::tempdir().unwrap();
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let r = router_of(
            Host::Herdr,
            vec![
                (Host::Herdr, fake("herdr", vec![], &calls, false)),
                (Host::HerdrApp, fake("herdr-app", vec![], &calls, false)),
                (Host::Orca, fake("orca", vec![], &calls, false)),
                (Host::Desktop, fake("desktop", vec![], &calls, false)),
            ],
            &dir,
        );
        r.focus("herdr-app:t8").unwrap();
        r.focus("orca:term_9").unwrap();
        r.focus("claude-desktop:7:local_a").unwrap();
        r.focus("w1:p2").unwrap();
        r.close("orca:term_9").unwrap();
        assert_eq!(*calls.lock().unwrap(), vec!["herdr-app focus herdr-app:t8", "orca focus orca:term_9", "desktop focus claude-desktop:7:local_a", "herdr focus w1:p2", "orca close orca:term_9"]);
    }

    #[test]
    fn a_herdr_agent_seen_from_outside_herdr_names_herdr() {
        let dir = tempfile::tempdir().unwrap();
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let r = router_of(Host::Desktop, vec![(Host::Desktop, fake("desktop", vec![], &calls, false))], &dir);
        let err = r.focus("w1:p2").unwrap_err();
        assert!(err.contains("Herdr"), "{err}");
    }

    #[test]
    fn agents_from_every_app_are_listed_and_a_down_app_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let r = router_of(
            Host::Orca,
            vec![
                (Host::Orca, fake("orca", vec![agent("orca:term_9", "s-1", Status::Working, "/w")], &calls, false)),
                (Host::Herdr, fake("herdr", vec![], &calls, true)),
                (Host::Desktop, fake("desktop", vec![agent("claude-desktop:7:local_a", "s-2", Status::Idle, "/w")], &calls, false)),
            ],
            &dir,
        );
        let panes: Vec<String> = r.agents().unwrap().into_iter().map(|a| a.pane).collect();
        assert_eq!(panes, ["orca:term_9", "claude-desktop:7:local_a"]);
    }

    #[test]
    fn a_resume_goes_to_the_app_running_the_session_else_home() {
        let dir = tempfile::tempdir().unwrap();
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let r = router_of(
            Host::Desktop,
            vec![(Host::Desktop, fake("desktop", vec![], &calls, false)), (Host::Orca, fake("orca", vec![agent("orca:term_9", "s-1", Status::Idle, "/w")], &calls, false))],
            &dir,
        );
        r.resume(&resume(Some(("s-1", "/w")), None, true)).unwrap();
        r.resume(&resume(Some(("s-5", "/w")), None, true)).unwrap();
        assert_eq!(*calls.lock().unwrap(), vec!["orca resume ", "desktop resume "]);
    }

    #[test]
    fn an_automatic_prompt_to_a_desktop_agent_opens_it_with_the_prompt() {
        let work = tempfile::tempdir().unwrap();
        let (d, calls) = desktop(vec![agent("claude-desktop:7:local_abc", "s-1", Status::Idle, "/w/app")], work.path());
        assert_eq!(d.resume(&resume(Some(("s-1", "/w/app")), Some("/pr-address x"), false)), Ok(Outcome::Prompted));
        assert_eq!(calls.lock().unwrap()[0], capcom::claude::new_session_link("/pr-address x", Path::new("/w/app")));
    }

    #[test]
    fn an_automatic_prompt_to_an_orca_agent_is_sent_without_switching() {
        let work = tempfile::tempdir().unwrap();
        let (o, calls) = orca_host(false, "idle", work.path());
        assert_eq!(o.resume(&resume(Some(("s-1", "/w/app")), Some("/pr-address x"), false)), Ok(Outcome::Prompted));
        let calls = calls.lock().unwrap();
        assert!(calls.iter().any(|c| c.starts_with("terminal send --terminal term_9")), "{calls:?}");
        assert!(!calls.iter().any(|c| c.starts_with("terminal switch")), "{calls:?}");
    }

    #[test]
    fn an_automatic_prompt_with_no_agent_starts_one_in_the_home_app() {
        let dir = tempfile::tempdir().unwrap();
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let r = router_of(Host::Orca, vec![(Host::Orca, fake("orca", vec![], &calls, false)), (Host::Desktop, fake("desktop", vec![], &calls, false))], &dir);
        r.resume(&resume(Some(("s-9", "/w")), Some("/pr-address x"), false)).unwrap();
        assert_eq!(*calls.lock().unwrap(), vec!["orca resume /pr-address x"]);
    }

    #[test]
    fn plain_terminals_get_no_router() {
        assert!(router(Host::Plain, PathBuf::from("/w")).is_none());
    }
}
