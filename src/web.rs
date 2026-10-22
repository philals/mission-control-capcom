//! `capcom web`: the boards and your open PRs as a page on localhost, read-only, polling GitHub by
//! itself (and only while a browser is looking). It listens on 127.0.0.1 only, answers GET only,
//! and refuses a `Host` that is not itself, so another website cannot read it through your browser.
use crate::agents::{self, AgentInfo, Status as AgentStatus};
use crate::attention::Attention;
use crate::model::{Board, Status, Task};
use crate::prs::{self, CheckState, CopilotState, MergeState, PrMsg, PrSource, PullRequest, Review, Waiting};
use crate::notes::{self, Note};
use crate::session::PrSession;
use crate::store;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod actions;
mod autopilot;
mod engine;
mod runs_poll;
mod states;
#[cfg(test)]
mod tests_more;
mod view;
pub use engine::Tools;
use engine::Engine;
use view::*;

const INDEX: &str = include_str!("web/index.html");
const SCRIPT: &str = concat!(include_str!("web/js/core.js"), "\n", include_str!("web/js/panels.js"), "\n", include_str!("web/js/extras.js"));
const STYLE: &str = include_str!("web/style.css");
const FAVICON: &str = include_str!("web/favicon.svg");

/// A browser that fetched the state this recently counts as looking.
const VIEWED_FOR: i64 = 20;
/// With no page looking, GitHub is asked this rarely (and at once when a page comes back).
const AWAY_WAIT: Duration = Duration::from_secs(30 * 60);
/// ... or this rarely while desktop notifications are on.
const NOTIFY_AWAY: Duration = Duration::from_secs(2 * 60);
/// While a page is looking, the Claude Code sessions are looked at this often.
pub const AGENTS_EVERY: Duration = Duration::from_secs(3);
/// Notes of agents that ended stay listed this many days, at most this many of them.
const EARLIER_DAYS: i64 = 7;
const EARLIER_MAX: usize = 12;
const MAX_CONNECTIONS: usize = 16;
const MAX_REQUEST: usize = 8192;
const MAX_BODY: usize = 4096;
/// A launch can take a couple of seconds to show, so the same link within this long is one click.
const SAME_CLICK: Duration = Duration::from_secs(4);
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

#[derive(Default)]
struct PrData {
    items: Vec<PullRequest>,
    error: Option<String>,
    updated: Option<DateTime<Utc>>,
    loading: bool,
}

/// What Herdr, the transcripts and the notes say about the agents.
#[derive(Default)]
struct Live {
    agents: Vec<AgentInfo>,
    error: Option<String>,
    notes: Vec<Note>,
}

/// Where the page learns which agents are running.
pub trait AgentSource: Send + Sync {
    fn agents(&self) -> Result<Vec<AgentInfo>, String>;
}

/// Opens a link in the browser you actually use.
pub trait Opener: Send + Sync {
    fn open(&self, url: &str) -> Result<(), String>;
}

/// The system's default browser (`xdg-open`), or the command in `CAPCOM_BROWSER`. It is started through
/// `systemd-run --user` when that is there, so the browser does not inherit this service's sandbox.
pub struct SystemBrowser {
    pub command: Option<String>,
}

/// How the browser is started under `systemd-run`. `KillMode=process` matters: `xdg-open` returns at
/// once, and by default systemd then kills what it started (the browser's own launcher, which is
/// still handing the link to the running browser) as the unit ends.
fn launch_args(program: &str, url: &str) -> Vec<String> {
    ["--user", "--collect", "--quiet", "-p", "KillMode=process", "--", program, url].iter().map(|s| s.to_string()).collect()
}

impl Opener for SystemBrowser {
    fn open(&self, url: &str) -> Result<(), String> {
        let program = self.command.clone().unwrap_or_else(|| if cfg!(target_os = "macos") { "open".into() } else { "xdg-open".into() });
        let through_systemd = Command::new("systemd-run").args(launch_args(&program, url)).output();
        if through_systemd.is_ok_and(|o| o.status.success()) {
            return Ok(());
        }
        let mut child = Command::new(&program).arg(url).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().map_err(|e| format!("could not run {program}: {e}"))?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

/// The Claude Code sessions on this machine, the terminal program's and the desktop app's.
pub struct ClaudeSessions {
    pub config: PathBuf,
}

impl AgentSource for ClaudeSessions {
    fn agents(&self) -> Result<Vec<AgentInfo>, String> {
        Ok(crate::claude::live(&self.config))
    }
}

pub struct Server {
    root: PathBuf,
    eng: Engine,
    me: std::sync::Weak<Server>,
    prs: Mutex<PrData>,
    live: Mutex<Live>,
    opener: Mutex<Option<Arc<dyn Opener>>>,
    /// The last link opened and when: the same link again within a few seconds is the same click, not a new launch.
    last_open: Mutex<Option<(String, std::time::Instant)>>,
    viewed: AtomicI64,
    attention: Arc<Attention>,
    wake: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    connections: AtomicUsize,
    port: u16,
}

struct Reply {
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

fn reply(status: &'static str, content_type: &'static str, body: impl Into<Vec<u8>>) -> Reply {
    Reply { status, content_type, body: body.into() }
}

impl Server {
    pub fn new(root: PathBuf, port: u16) -> Arc<Server> {
        Arc::new_cyclic(|me| Server {
            root,
            eng: Engine::default(),
            me: me.clone(),
            prs: Mutex::new(PrData { loading: true, ..PrData::default() }),
            live: Mutex::new(Live::default()),
            opener: Mutex::new(None),
            last_open: Mutex::new(None),
            viewed: AtomicI64::new(0),
            attention: Attention::waiting_at_least(AWAY_WAIT),
            wake: Mutex::new(None),
            connections: AtomicUsize::new(0),
            port,
        })
    }

    /// Start polling GitHub in the background through `source`.
    pub fn poll(self: &Arc<Self>, source: Arc<dyn PrSource>) {
        let (rx, wake) = prs::spawn_with(source, prs::Cadence::default(), self.attention.clone());
        *self.wake.lock().unwrap_or_else(|e| e.into_inner()) = Some(wake);
        let me = self.clone();
        std::thread::spawn(move || {
            while let Ok(msg) = rx.recv() {
                let mut data = me.prs.lock().unwrap_or_else(|e| e.into_inner());
                match msg {
                    PrMsg::Started => data.loading = true,
                    PrMsg::Result(result) => {
                        data.loading = false;
                        match result {
                            Ok(items) => {
                                drop(data);
                                me.on_prs(&items);
                                data = me.prs.lock().unwrap_or_else(|e| e.into_inner());
                                data.items = items;
                                data.error = None;
                                data.updated = Some(Utc::now());
                            }
                            Err(e) => data.error = Some(e),
                        }
                    }
                }
            }
        });
        let me = self.clone();
        std::thread::spawn(move || loop {
            me.attention.set_terminal(me.looking());
            std::thread::sleep(Duration::from_secs(2));
        });
    }

    /// Let a click on a link open it in your own browser through `opener`.
    pub fn set_opener(&self, opener: Arc<dyn Opener>) {
        *self.opener.lock().unwrap_or_else(|e| e.into_inner()) = Some(opener);
    }

    fn own_origin(&self, origin: &str) -> bool {
        origin == format!("http://127.0.0.1:{}", self.port) || origin == format!("http://localhost:{}", self.port)
    }

    /// `POST /open` with `{"url": "…"}`: open a link the page shows in the system browser. Refused
    /// unless the request comes from the page itself (a header a website cannot add without a
    /// preflight this server never grants, and its own origin) and names a link already on the page.
    fn open(&self, headers: &[(String, String)], body: &str) -> Reply {
        let h = |name: &str| headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        let plain = |status: &'static str, text: &'static str| reply(status, "text/plain; charset=utf-8", text);
        if h("x-capcom") != Some("open") || h("origin").is_some_and(|o| !self.own_origin(o)) || h("sec-fetch-site").is_some_and(|s| s != "same-origin") {
            return plain("403 Forbidden", "not from the page");
        }
        let Some(url) = serde_json::from_str::<Value>(body).ok().and_then(|v| v["url"].as_str().map(str::to_string)) else {
            return plain("400 Bad Request", "need a url");
        };
        let shown = {
            let mut urls = std::collections::HashSet::new();
            shown_urls(&self.state_json(Utc::now()), &mut urls);
            urls
        };
        if url.len() > 2000 || url.chars().any(|c| c.is_whitespace() || c.is_control()) || !shown.contains(&url) {
            return plain("403 Forbidden", "not a link on the page");
        }
        let Some(opener) = self.opener.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
            return plain("501 Not Implemented", "no browser to open it in");
        };
        {
            let mut last = self.last_open.lock().unwrap_or_else(|e| e.into_inner());
            if last.as_ref().is_some_and(|(u, at)| *u == url && at.elapsed() < SAME_CLICK) {
                return plain("204 No Content", "");
            }
            *last = Some((url.clone(), std::time::Instant::now()));
        }
        match opener.open(&url) {
            Ok(()) => plain("204 No Content", ""),
            Err(_) => plain("502 Bad Gateway", "the browser could not be started"),
        }
    }

    fn looking(&self) -> bool {
        Utc::now().timestamp() - self.viewed.load(Ordering::Relaxed) < VIEWED_FOR
    }

    /// Keep the agents and their notes up to date, but only while a page is looking (and once at the start).
    pub fn watch_agents(self: &Arc<Self>, source: Arc<dyn AgentSource>, notes_dir: Option<PathBuf>, every: Duration, tick: Duration) {
        let me = self.clone();
        std::thread::spawn(move || {
            let mut asked: Option<std::time::Instant> = None;
            let mut notes_seen = None;
            loop {
                if (asked.is_none() || me.looking()) && asked.is_none_or(|t| t.elapsed() >= every) {
                    asked = Some(std::time::Instant::now());
                    let result = source.agents();
                    let mut live = me.live.lock().unwrap_or_else(|e| e.into_inner());
                    match result {
                        Ok(list) => {
                            live.agents = list;
                            live.error = None;
                        }
                        Err(e) => {
                            live.agents.clear();
                            live.error = Some(e);
                        }
                    }
                    if let Some(dir) = &notes_dir {
                        let stamp = notes::stamp(dir);
                        if notes_seen.as_ref() != Some(&stamp) {
                            notes_seen = Some(stamp);
                            live.notes = notes::read_all(dir, Utc::now());
                        }
                    }
                }
                std::thread::sleep(tick);
            }
        });
    }

    fn looked(&self) {
        let before = self.viewed.swap(Utc::now().timestamp(), Ordering::Relaxed);
        if Utc::now().timestamp() - before >= VIEWED_FOR {
            // somebody is back: fetch now instead of after the slow wait
            if let Some(wake) = self.wake.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                let _ = wake.send(());
            }
        }
    }

    fn host_ok(&self, host: &str) -> bool {
        let host = host.trim();
        host == format!("127.0.0.1:{}", self.port) || host == format!("localhost:{}", self.port)
    }

    fn route(&self, path: &str, query: &str) -> Reply {
        match path {
            "/" | "/index.html" => reply("200 OK", "text/html; charset=utf-8", INDEX),
            "/app.js" => reply("200 OK", "text/javascript; charset=utf-8", SCRIPT),
            "/style.css" => reply("200 OK", "text/css; charset=utf-8", STYLE),
            "/favicon.svg" | "/favicon.ico" => reply("200 OK", "image/svg+xml", FAVICON),
            "/api/state" => {
                self.looked();
                let value = self.state_json(Utc::now());
                reply("200 OK", "application/json", serde_json::to_vec(&value).unwrap_or_default())
            }
            "/api/task" => self.task_detail(query),
            _ => reply("404 Not Found", "text/plain; charset=utf-8", "not found"),
        }
    }

    /// What a task's own file says, for its sheet: `/api/task?story=PROJ-1&id=T2`.
    fn task_detail(&self, query: &str) -> Reply {
        let param = |name: &str| query.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == name).map(|(_, v)| v.to_string())).unwrap_or_default();
        let (key, id) = (param("story"), param("id"));
        let safe = |t: &str| !t.is_empty() && t.len() <= 64 && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !safe(&key) || !safe(&id) {
            return reply("400 Bad Request", "text/plain; charset=utf-8", "need story and id");
        }
        let Ok(board) = store::load(&self.root, &key) else {
            return reply("404 Not Found", "text/plain; charset=utf-8", "no such story");
        };
        let Some(task) = board.task(&id) else {
            return reply("404 Not Found", "text/plain; charset=utf-8", "no such task");
        };
        let file = std::path::Path::new(&task.file);
        let doc = if file.is_absolute() || file.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            None
        } else {
            std::fs::read_to_string(self.root.join(&key).join(file)).ok()
        };
        let summary = match &doc {
            Some(doc) => crate::taskdoc::summary(doc, task.status),
            None => crate::taskdoc::Summary { label: "Description", body: "The task file could not be read.".to_string() },
        };
        let worktrees = doc.as_deref().is_some_and(crate::finish::mentions_worktrees);
        let body = json!({
            "id": task.id,
            "title": task.title,
            "status": task.status.as_str(),
            "label": summary.label,
            "body": summary.body,
            "file": task.file,
            "agent": task.agent.as_ref().map(|a| json!({ "pane": a.pane, "skill": a.skill, "startedAt": a.started_at })),
            "sessions": task.sessions.len(),
            "worktrees": worktrees,
        });
        reply("200 OK", "application/json", serde_json::to_vec(&body).unwrap_or_default())
    }

    /// The answer to one request, as the bytes to send back.
    pub fn answer(&self, request: &str) -> Vec<u8> {
        let (head, body) = request.split_once("\r\n\r\n").unwrap_or((request, ""));
        let mut lines = head.split("\r\n");
        let first = lines.next().unwrap_or_default();
        let mut parts = first.split(' ');
        let (method, target) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
        let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
        let host = headers.iter().find(|(k, _)| k == "host").map(|(_, v)| v.as_str()).unwrap_or_default();
        let path = target.split(['?', '#']).next().unwrap_or("/");
        let query = target.split('#').next().and_then(|t| t.split_once('?')).map_or("", |(_, q)| q);
        let result = if !self.host_ok(host) {
            reply("403 Forbidden", "text/plain; charset=utf-8", "wrong host")
        } else if method == "POST" && path == "/open" {
            self.open(&headers, body)
        } else if method == "POST" && path == "/api/do" {
            self.do_request(&headers, body)
        } else if method != "GET" && method != "HEAD" {
            reply("405 Method Not Allowed", "text/plain; charset=utf-8", "read-only")
        } else {
            self.route(path, query)
        };
        let mut out = format!(
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: {CSP}\r\nConnection: close\r\n\r\n",
            result.status,
            result.content_type,
            result.body.len()
        )
        .into_bytes();
        if method != "HEAD" {
            out.extend(result.body);
        }
        out
    }

    fn handle(&self, mut stream: TcpStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        let end_of_head = |buf: &[u8]| buf.windows(4).position(|w| w == b"\r\n\r\n");
        while end_of_head(&buf).is_none() && buf.len() < MAX_REQUEST {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        let Some(end) = end_of_head(&buf) else {
            return;
        };
        let head = String::from_utf8_lossy(&buf[..end]).to_lowercase();
        let length: usize = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        if length > MAX_BODY {
            return;
        }
        while buf.len() < end + 4 + length {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        let _ = stream.write_all(&self.answer(&String::from_utf8_lossy(&buf)));
    }

    /// Accept connections for ever, one short-lived thread each (at most a few at once).
    pub fn serve(self: &Arc<Self>, listener: TcpListener) {
        for stream in listener.incoming().flatten() {
            if self.connections.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                self.connections.fetch_sub(1, Ordering::SeqCst);
                continue;
            }
            let me = self.clone();
            std::thread::spawn(move || {
                me.handle(stream);
                me.connections.fetch_sub(1, Ordering::SeqCst);
            });
        }
    }
}

/// Bind to localhost only. Port 0 picks a free one.
pub fn bind(port: u16) -> Result<(TcpListener, u16)> {
    let listener = TcpListener::bind(("127.0.0.1", port)).with_context(|| format!("port {port} on 127.0.0.1 is in use"))?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> (tempfile::TempDir, Arc<Server>) {
        let root = tempfile::tempdir().unwrap();
        crate::ops::init_story(root.path(), "PROJ-1", "First story", None).unwrap();
        store::update(root.path(), "PROJ-1", |b| {
            let dir = root.path().join("PROJ-1");
            crate::ops::add_task(&dir, b, "Do it", crate::model::TaskType::Pr, vec![], vec![]).map(|_| ())
        })
        .unwrap();
        let server = Server::new(root.path().to_path_buf(), 7878);
        (root, server)
    }

    fn get(server: &Server, path: &str, host: &str) -> String {
        String::from_utf8_lossy(&server.answer(&format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n"))).to_string()
    }

    #[test]
    fn it_serves_the_page_and_the_state_to_its_own_host_only() {
        let (_root, s) = server();
        assert!(get(&s, "/", "127.0.0.1:7878").starts_with("HTTP/1.1 200 OK"));
        assert!(get(&s, "/", "localhost:7878").starts_with("HTTP/1.1 200 OK"));
        let page = get(&s, "/", "127.0.0.1:7878");
        assert!(page.contains("Content-Security-Policy: default-src 'none'; script-src 'self'"), "{page}");
        assert!(get(&s, "/", "evil.example:7878").starts_with("HTTP/1.1 403"), "a rebound name is refused");
        assert!(get(&s, "/api/state", "evil.example").starts_with("HTTP/1.1 403"));
        assert!(get(&s, "/nope", "127.0.0.1:7878").starts_with("HTTP/1.1 404"));
        assert!(get(&s, "/app.js?x=1", "127.0.0.1:7878").starts_with("HTTP/1.1 200 OK"), "a query string is ignored");
    }

    #[test]
    fn with_nobody_looking_github_is_asked_only_every_half_hour() {
        let (_root, s) = server();
        let busy = Duration::from_secs(5);
        s.attention.set_terminal(true);
        assert_eq!(s.attention.wait(busy), busy, "a page is open: normal pace");
        s.attention.set_terminal(false);
        assert_eq!(s.attention.wait(busy), AWAY_WAIT);
        assert!(AWAY_WAIT >= Duration::from_secs(1800));
    }

    struct Fake {
        calls: AtomicUsize,
        agents: Vec<AgentInfo>,
    }

    impl AgentSource for Fake {
        fn agents(&self) -> Result<Vec<AgentInfo>, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.agents.clone())
        }
    }

    fn agent(pane: &str, name: Option<&str>, status: AgentStatus, session: Option<&str>) -> AgentInfo {
        AgentInfo { pane: pane.into(), name: name.map(str::to_string), status, topic: format!("topic of {pane}"), workspace: "scratch".into(), cwd: String::new(), session: session.map(str::to_string) }
    }

    fn note(session: &str, task: &str, needs_you: bool, at: DateTime<Utc>) -> Note {
        Note { session: session.into(), task: task.into(), progress: "p".into(), waiting: String::new(), needs_you, next: "do it".into(), cwd: "/w".into(), updated_at: at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true) }
    }

    fn agents_state(s: &Server) -> Value {
        let page = get(s, "/api/state", "127.0.0.1:7878");
        serde_json::from_str::<Value>(page.split("\r\n\r\n").nth(1).unwrap()).unwrap()["agents"].clone()
    }

    #[test]
    fn the_state_groups_agents_under_their_story_with_notes_and_lists_the_ended_ones() {
        let (_root, s) = server();
        let now = Utc::now();
        *s.live.lock().unwrap() = Live {
            agents: vec![
                agent("w1:p1", Some("proj-1-t1-impl"), AgentStatus::Idle, Some("s-ask")),
                agent("w2:p1", None, AgentStatus::Working, Some("s-busy")),
                agent("w3:p1", None, AgentStatus::Blocked, None),
            ],
            notes: vec![
                note("s-ask", "find the loop", true, now),
                note("s-busy", "fix lint", true, now),
                note("s-gone", "spike aurora", true, now - chrono::Duration::days(2)),
                note("s-old", "ancient", false, now - chrono::Duration::days(20)),
            ],
            ..Live::default()
        };
        let agents = agents_state(&s);
        assert_eq!(agents["groups"][0]["label"], "PROJ-1 First story", "the story's agent comes first");
        assert_eq!(agents["groups"][0]["entries"][0]["label"], "T1 impl");
        assert_eq!(agents["groups"][0]["entries"][0]["status"], "needs you", "idle with a note that needs you");
        assert_eq!(agents["groups"][0]["entries"][0]["note"], 0);
        let scratch = agents["groups"].as_array().unwrap().iter().find(|g| g["label"] == "scratch").unwrap();
        let busy = scratch["entries"].as_array().unwrap().iter().find(|e| e["pane"] == "w2:p1").unwrap();
        assert_eq!(busy["state"], "working", "a note left from before the agent carried on is not an ask");
        assert_eq!(agents["earlier"], json!([2]), "only the ended agent within the week");
        assert_eq!(agents["needsYou"], 3, "the idle ask, the blocked agent and the ended one");
        assert_eq!(agents["working"], 1);
        assert_eq!(agents["notes"][0]["next"], "do it");
        assert_eq!(agents["groups"][0]["entries"][0]["deepLink"], Value::Null, "no desktop id: nothing to open");
    }

    #[test]
    fn the_agents_are_asked_about_only_while_a_page_looks_and_once_at_the_start() {
        let (_root, s) = server();
        let fake = Arc::new(Fake { calls: AtomicUsize::new(0), agents: vec![agent("w1:p1", None, AgentStatus::Idle, None)] });
        s.watch_agents(fake.clone(), None, Duration::from_millis(40), Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1, "once, then nothing while nobody looks");
        assert_eq!(s.live.lock().unwrap().agents.len(), 1);
        s.looked();
        std::thread::sleep(Duration::from_millis(300));
        assert!(fake.calls.load(Ordering::SeqCst) >= 3, "a page is looking: asked again and again");
    }

    #[test]
    fn a_herdr_that_cannot_be_reached_is_said_and_the_stale_agents_are_dropped() {
        struct Down;
        impl AgentSource for Down {
            fn agents(&self) -> Result<Vec<AgentInfo>, String> {
                Err("could not run herdr".into())
            }
        }
        let (_root, s) = server();
        s.live.lock().unwrap().agents = vec![agent("w1:p1", None, AgentStatus::Idle, None)];
        s.watch_agents(Arc::new(Down), None, Duration::from_millis(40), Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(200));
        let agents = agents_state(&s);
        assert_eq!(agents["error"], "could not run herdr");
        assert_eq!(agents["count"], 0);
    }

    struct Recorder(Mutex<Vec<String>>);

    impl Opener for Recorder {
        fn open(&self, url: &str) -> Result<(), String> {
            self.0.lock().unwrap().push(url.to_string());
            Ok(())
        }
    }

    fn post(s: &Server, headers: &str, body: &str) -> String {
        let request = format!("POST /open HTTP/1.1\r\nHost: 127.0.0.1:7878\r\n{headers}Content-Length: {}\r\n\r\n{body}", body.len());
        String::from_utf8_lossy(&s.answer(&request)).to_string()
    }

    const PAGE: &str = "X-Capcom: open\r\nOrigin: http://127.0.0.1:7878\r\nSec-Fetch-Site: same-origin\r\n";

    #[test]
    fn a_click_opens_only_a_link_the_page_shows_and_only_when_the_page_asks() {
        let (_root, s) = server();
        store::update(&s.root, "PROJ-1", |b| crate::ops::add_pr(b, "T1", "widgets", "https://github.com/acme/widgets/pull/12", crate::model::PrState::Draft)).unwrap();
        let opened = Arc::new(Recorder(Mutex::new(Vec::new())));
        let link = r#"{"url":"https://github.com/acme/widgets/pull/12"}"#;
        assert!(post(&s, PAGE, link).starts_with("HTTP/1.1 501"), "nothing to open it with yet");
        s.set_opener(opened.clone());
        assert!(post(&s, PAGE, link).starts_with("HTTP/1.1 204"));
        assert_eq!(*opened.0.lock().unwrap(), vec!["https://github.com/acme/widgets/pull/12".to_string()]);
        assert!(post(&s, "Origin: http://127.0.0.1:7878\r\n", link).starts_with("HTTP/1.1 403"), "no header from the page: refused (a website cannot add it without a preflight)");
        assert!(post(&s, "X-Capcom: open\r\nOrigin: https://evil.example\r\n", link).starts_with("HTTP/1.1 403"), "another origin");
        assert!(post(&s, "X-Capcom: open\r\nSec-Fetch-Site: cross-site\r\n", link).starts_with("HTTP/1.1 403"), "a cross-site request");
        assert!(post(&s, PAGE, r#"{"url":"https://evil.example/"}"#).starts_with("HTTP/1.1 403"), "a link the page does not show");
        assert!(post(&s, PAGE, r#"{"url":"file:///etc/passwd"}"#).starts_with("HTTP/1.1 403"));
        assert!(post(&s, PAGE, r#"{"url":"https://github.com/acme/widgets/pull/12 --new-window"}"#).starts_with("HTTP/1.1 403"), "no extra words");
        assert!(post(&s, PAGE, "not json").starts_with("HTTP/1.1 400"));
        assert_eq!(opened.0.lock().unwrap().len(), 1, "only the one good click opened anything");
        let get = String::from_utf8_lossy(&s.answer("GET /open HTTP/1.1\r\nHost: 127.0.0.1:7878\r\n\r\n")).to_string();
        assert!(!get.starts_with("HTTP/1.1 204") && !get.starts_with("HTTP/1.1 200"), "a GET never opens anything: {get}");
        let rebound = String::from_utf8_lossy(&s.answer(&format!("POST /open HTTP/1.1\r\nHost: evil.example\r\n{PAGE}Content-Length: {}\r\n\r\n{link}", link.len()))).to_string();
        assert!(rebound.starts_with("HTTP/1.1 403"), "a rebound name is refused first");
    }

    #[test]
    fn a_desktop_session_opens_in_the_desktop_app_but_only_one_that_is_listed() {
        let (_root, s) = server();
        let opened = Arc::new(Recorder(Mutex::new(Vec::new())));
        s.set_opener(opened.clone());
        s.live.lock().unwrap().agents = vec![agent("claude-desktop:7:local_abc-123", None, AgentStatus::Idle, Some("s-1")), agent("cli:8", None, AgentStatus::Idle, Some("s-2"))];
        let state = agents_state(&s);
        let entries: Vec<&Value> = state["groups"].as_array().unwrap().iter().flat_map(|g| g["entries"].as_array().unwrap()).collect();
        let desktop = entries.iter().find(|e| e["source"] == "desktop").unwrap();
        let cli = entries.iter().find(|e| e["source"] == "cli").unwrap();
        assert_eq!(desktop["deepLink"], "claude://code/continue?session=local_abc-123");
        assert_eq!(cli["deepLink"], Value::Null, "a terminal session has no link");
        assert!(post(&s, PAGE, r#"{"url":"claude://code/continue?session=local_abc-123"}"#).starts_with("HTTP/1.1 204"));
        assert!(post(&s, PAGE, r#"{"url":"claude://code/continue?session=local_other"}"#).starts_with("HTTP/1.1 403"), "not a session on the list");
        assert!(post(&s, PAGE, r#"{"url":"claude://code/new"}"#).starts_with("HTTP/1.1 403"));
        assert_eq!(*opened.0.lock().unwrap(), vec!["claude://code/continue?session=local_abc-123".to_string()]);
    }

    #[test]
    fn the_pr_diff_and_the_jira_link_are_among_the_links_it_will_open() {
        let (_root, s) = server();
        store::update(&s.root, "PROJ-1", |b| {
            b.story.jira_url = Some("https://jira.example/browse/PROJ-1".into());
            Ok(())
        })
        .unwrap();
        let mut pr: PullRequest = serde_json::from_value(json!({
            "repo": "acme/widgets", "number": 12, "title": "t", "url": "https://github.com/acme/widgets/pull/12",
            "is_draft": false, "labels": [], "review": "Required", "comments": 0, "updated_at": "x", "created_at": "x",
            "checks": [{ "name": "build", "workflow": null, "state": "Failed", "started_at": null, "completed_at": null, "url": "https://github.com/acme/widgets/actions/runs/1/job/2" }],
            "feedback": { "mine": true, "head": "a", "branch": "b", "copilot": "None", "copilot_reviewed_at": null, "threads": [] }
        }))
        .unwrap();
        pr.feedback.mine = true;
        *s.prs.lock().unwrap() = PrData { items: vec![pr], ..PrData::default() };
        let mut urls = std::collections::HashSet::new();
        shown_urls(&s.state_json(Utc::now()), &mut urls);
        for want in ["https://github.com/acme/widgets/pull/12", "https://github.com/acme/widgets/pull/12/files", "https://github.com/acme/widgets/actions/runs/1/job/2", "https://jira.example/browse/PROJ-1"] {
            assert!(urls.contains(want), "{want} missing from {urls:?}");
        }
    }

    #[test]
    fn the_same_link_again_within_a_few_seconds_is_one_click_and_launches_once() {
        let (_root, s) = server();
        store::update(&s.root, "PROJ-1", |b| crate::ops::add_pr(b, "T1", "widgets", "https://github.com/acme/widgets/pull/12", crate::model::PrState::Draft)).unwrap();
        let opened = Arc::new(Recorder(Mutex::new(Vec::new())));
        s.set_opener(opened.clone());
        let link = r#"{"url":"https://github.com/acme/widgets/pull/12"}"#;
        for _ in 0..3 {
            assert!(post(&s, PAGE, link).starts_with("HTTP/1.1 204"), "every click is answered the same");
        }
        assert_eq!(opened.0.lock().unwrap().len(), 1, "but only the first started anything");
        *s.last_open.lock().unwrap() = Some(("https://github.com/acme/widgets/pull/12".into(), std::time::Instant::now() - SAME_CLICK - Duration::from_millis(10)));
        post(&s, PAGE, link);
        assert_eq!(opened.0.lock().unwrap().len(), 2, "a click after the wait starts it again");
    }

    #[test]
    fn the_browser_is_started_so_that_systemd_does_not_cut_its_launcher_off() {
        let args = launch_args("xdg-open", "https://github.com/");
        assert!(args.windows(2).any(|w| w == ["-p", "KillMode=process"]), "{args:?}");
        assert_eq!(args[args.len() - 2..], ["xdg-open", "https://github.com/"], "the link is the last, single argument");
    }

    #[test]
    fn only_get_is_answered() {
        let (_root, s) = server();
        for method in ["POST", "PUT", "DELETE"] {
            let out = String::from_utf8_lossy(&s.answer(&format!("{method} /api/state HTTP/1.1\r\nHost: 127.0.0.1:7878\r\n\r\n"))).to_string();
            assert!(out.starts_with("HTTP/1.1 405"), "{method}: {out}");
        }
        let head = String::from_utf8_lossy(&s.answer("HEAD / HTTP/1.1\r\nHost: 127.0.0.1:7878\r\n\r\n")).to_string();
        assert!(head.starts_with("HTTP/1.1 200 OK") && head.ends_with("\r\n\r\n"), "no body for HEAD");
    }

    #[test]
    fn the_state_has_the_boards_and_the_prs_newest_first_with_their_task() {
        let (_root, s) = server();
        store::update(&s.root, "PROJ-1", |b| crate::ops::add_pr(b, "T1", "widgets", "https://github.com/acme/widgets/pull/12", crate::model::PrState::Draft)).unwrap();
        let mk = |n: u64, created: &str| {
            let mut pr: PullRequest = serde_json::from_value(json!({
                "repo": "acme/widgets", "number": n, "title": format!("PR {n}"), "url": format!("https://github.com/acme/widgets/pull/{n}"),
                "is_draft": false, "labels": [], "review": "Required", "comments": 0, "updated_at": created, "created_at": created, "checks": [],
                "feedback": { "mine": true, "head": "abc", "branch": format!("feature/{n}"), "copilot": "None", "copilot_reviewed_at": null, "threads": [] }
            }))
            .unwrap();
            pr.feedback.mine = true;
            pr
        };
        *s.prs.lock().unwrap() = PrData { items: vec![mk(11, "2026-10-01T00:00:00Z"), mk(12, "2026-10-02T00:00:00Z")], updated: Some(Utc::now()), ..PrData::default() };
        let page = get(&s, "/api/state", "127.0.0.1:7878");
        let body: Value = serde_json::from_str(page.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["stories"][0]["key"], "PROJ-1");
        assert_eq!(body["stories"][0]["tasks"][0]["id"], "T1");
        assert_eq!(body["prs"][0]["number"], 12, "newest first");
        assert_eq!(body["prs"][0]["tag"], json!({ "story": "PROJ-1", "task": "T1" }));
        assert_eq!(body["prs"][0]["branch"], "feature/12");
        assert_eq!(body["prs"][1]["tag"], Value::Null);
        assert!(body["prs"][0]["rail"].as_array().unwrap().len() >= 6);
    }

    #[test]
    fn a_looked_at_page_wakes_the_poll_only_after_being_away() {
        let (_root, s) = server();
        let (tx, rx) = std::sync::mpsc::channel();
        *s.wake.lock().unwrap() = Some(tx);
        s.looked();
        assert!(rx.try_recv().is_ok(), "first look after being away wakes the poll");
        s.looked();
        assert!(rx.try_recv().is_err(), "the next one, a moment later, does not");
    }

    #[test]
    fn it_answers_over_a_real_socket_and_refuses_a_slow_or_huge_request() {
        let (_root, s) = server();
        let (listener, port) = bind(0).unwrap();
        let s = Server::new(s.root.clone(), port);
        let server = s.clone();
        std::thread::spawn(move || server.serve(listener));
        let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
        conn.write_all(format!("GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes()).unwrap();
        let mut out = String::new();
        conn.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 200 OK") && out.contains("\"stories\""), "{out}");
        let mut huge = TcpStream::connect(("127.0.0.1", port)).unwrap();
        huge.write_all(&vec![b'a'; MAX_REQUEST + 100]).unwrap();
        let mut nothing = String::new();
        let _ = huge.read_to_string(&mut nothing);
        assert!(nothing.is_empty(), "a request with no end is dropped");
    }
}
