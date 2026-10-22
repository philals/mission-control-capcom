//! `capcom web`: the boards and your open PRs as a page on localhost, read-only, polling GitHub by
//! itself (and only while a browser is looking). It listens on 127.0.0.1 only, answers GET only,
//! and refuses a `Host` that is not itself, so another website cannot read it through your browser.
use crate::agents::{self, AgentInfo, Status as AgentStatus};
use crate::attention::Attention;
use crate::model::{Board, Status, Task};
use crate::prs::{self, CheckState, CopilotState, MergeState, PrMsg, PrSource, PullRequest, Review, Waiting};
use crate::notes::{self, Note};
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

const INDEX: &str = include_str!("web/index.html");
const SCRIPT: &str = include_str!("web/app.js");
const STYLE: &str = include_str!("web/style.css");
const FAVICON: &str = include_str!("web/favicon.svg");

/// A browser that fetched the state this recently counts as looking.
const VIEWED_FOR: i64 = 20;
/// With no page looking, GitHub is asked this rarely (and at once when a page comes back).
const AWAY_WAIT: Duration = Duration::from_secs(30 * 60);
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

/// Every link the page offers: what `/open` is willing to open.
fn shown_urls(value: &Value, out: &mut std::collections::HashSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                if key == "deepLink" && v.as_str().is_some_and(|u| u.starts_with("claude://code/continue?session=local_")) {
                    out.insert(v.as_str().unwrap_or_default().to_string());
                } else if (key == "url" || key == "jiraUrl") && v.as_str().is_some_and(|u| u.starts_with("https://")) {
                    let url = v.as_str().unwrap_or_default();
                    out.insert(url.to_string());
                    if map.contains_key("number") {
                        out.insert(format!("{url}/files"));
                    }
                } else {
                    shown_urls(v, out);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| shown_urls(v, out)),
        _ => {}
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

fn same_url(a: &str, b: &str) -> bool {
    a.trim_end_matches('/').eq_ignore_ascii_case(b.trim_end_matches('/'))
}

fn check_state(state: CheckState) -> &'static str {
    match state {
        CheckState::Queued => "queued",
        CheckState::Running => "running",
        CheckState::Passed => "passed",
        CheckState::Failed => "failed",
        CheckState::Cancelled => "cancelled",
        CheckState::Skipped => "skipped",
    }
}

/// A step on a PR's way to a merge: ready, CI, Copilot, its conversations, approval, the merge.
fn rail(pr: &PullRequest) -> Vec<Value> {
    let c = pr.counts();
    let failed = pr.real_failures().len();
    let step = |state: &str, text: String| json!({ "state": state, "text": text });
    let ci = if failed > 0 {
        step("wrong", format!("CI {failed} failed"))
    } else if pr.is_busy() {
        step("going", "CI running".into())
    } else if c.passed > 0 {
        step("done", "CI".into())
    } else {
        step("pending", "CI no checks".into())
    };
    let (copilot, threads) = match pr.feedback.copilot {
        CopilotState::None => (step("pending", "copilot".into()), step("pending", "threads".into())),
        CopilotState::Requested => (step("going", "copilot".into()), step("pending", "threads".into())),
        CopilotState::Reviewed => (
            step("done", "copilot".into()),
            match pr.copilot_open().len() {
                0 => step("done", "threads".into()),
                n => step("wrong", format!("threads {n}")),
            },
        ),
    };
    let approval = match pr.review {
        Review::Approved => step("done", "approved".into()),
        Review::ChangesRequested => step("wrong", "changes requested".into()),
        Review::Required | Review::None => step("pending", "approved".into()),
    };
    let merge = match pr.feedback.merge {
        MergeState::Conflicting => step("wrong", "conflicts".into()),
        MergeState::Behind => step("going", "behind".into()),
        MergeState::Clean => step("done", "merge".into()),
        MergeState::Unknown => step("pending", "merge".into()),
    };
    let ready = if pr.is_draft { step("pending", "draft".into()) } else { step("done", "ready".into()) };
    let mut steps = vec![ready, ci, copilot, threads, approval];
    if !pr.automerge_labels().is_empty() {
        steps.push(if pr.automerge_on() { step("going", "automerge on".into()) } else { step("pending", "automerge".into()) });
    }
    steps.push(merge);
    steps
}

fn pr_view(pr: &PullRequest, boards: &[Board]) -> Value {
    let tag = boards.iter().find_map(|b| {
        b.tasks.iter().find(|t| t.prs.iter().any(|p| same_url(&p.url, &pr.url))).map(|t| json!({ "story": b.story.key, "task": t.id }))
    });
    let c = pr.counts();
    let waiting = match pr.waiting_for() {
        Some(Waiting::Reviewer) => "reviewer",
        Some(Waiting::Merge) => "merge",
        Some(Waiting::Nothing) => "green",
        None => "",
    };
    json!({
        "repo": pr.repo,
        "number": pr.number,
        "title": pr.title,
        "url": pr.url,
        "branch": pr.feedback.branch,
        "draft": pr.is_draft,
        "mine": pr.feedback.mine,
        "pasted": pr.feedback.pasted,
        "labels": pr.labels,
        "review": match pr.review { Review::Approved => "approved", Review::ChangesRequested => "changes", Review::Required => "required", Review::None => "none" },
        "comments": pr.comments,
        "createdAt": pr.created_at,
        "updatedAt": pr.updated_at,
        "tag": tag,
        "urgency": pr.urgency(),
        "red": pr.is_red(),
        "waiting": waiting,
        "blocker": pr.merge_blocker(),
        "rail": rail(pr),
        "counts": { "passed": c.passed, "failed": c.failed, "running": c.running, "queued": c.queued, "skipped": c.skipped, "cancelled": c.cancelled },
        "checks": pr.ordered_checks().iter().map(|k| json!({
            "label": k.label(),
            "state": check_state(k.state),
            "startedAt": k.started_at,
            "completedAt": k.completed_at,
            "url": k.url,
            "external": k.external,
        })).collect::<Vec<_>>(),
    })
}

fn task_view(board: &Board, task: &Task) -> Value {
    let done = |id: &str| board.task(id).is_some_and(|t| t.status == Status::Done);
    let waiting_on: Vec<&String> = task.depends_on.iter().filter(|d| !done(d)).collect();
    json!({
        "id": task.id,
        "title": task.title,
        "type": task.kind.as_str(),
        "status": task.status.as_str(),
        "blocked": task.blocked.as_ref().map(|b| b.reason.clone()),
        "dependsOn": task.depends_on,
        "waitingOn": waiting_on,
        "repos": task.repos,
        "prs": task.prs.iter().map(|p| json!({ "repo": p.repo, "url": p.url, "state": p.state.as_str() })).collect::<Vec<_>>(),
        "working": task.agent.is_some(),
    })
}

/// Everything the page draws: the boards, and your open PRs, newest first.
fn state(root: &std::path::Path, data: &PrData, live: &Live, now: DateTime<Utc>) -> Value {
    let mut boards = Vec::new();
    let mut broken = Vec::new();
    for key in store::list_keys(root).unwrap_or_default() {
        match store::load(root, &key) {
            Ok(board) => boards.push(board),
            Err(e) => broken.push(json!({ "key": key, "error": format!("{e:#}") })),
        }
    }
    let stories: Vec<Value> = boards
        .iter()
        .map(|b| {
            json!({
                "key": b.story.key,
                "title": b.story.title,
                "status": b.story.status.as_str(),
                "jiraUrl": b.story.jira_url,
                "tasks": b.tasks.iter().map(|t| task_view(b, t)).collect::<Vec<_>>(),
            })
        })
        .collect();
    let mut order: Vec<&PullRequest> = data.items.iter().collect();
    order.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    let red = data.items.iter().filter(|p| p.is_red()).count();
    let waiting = data.items.iter().filter(|p| p.waiting_for() == Some(Waiting::Reviewer)).count();
    json!({
        "now": now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "stories": stories,
        "broken": broken,
        "prs": order.iter().map(|p| pr_view(p, &boards)).collect::<Vec<_>>(),
        "prError": data.error,
        "prUpdated": data.updated.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        "loading": data.loading,
        "red": red,
        "awaitingReview": waiting,
        "agents": agents_view(live, &boards, now),
    })
}

fn agents_view(live: &Live, boards: &[Board], now: DateTime<Utc>) -> Value {
    let stories: Vec<(String, String)> = boards.iter().map(|b| (b.story.key.clone(), agents::short_name(&b.story.title))).collect();
    let parents = agents::parents(&live.agents, &[]);
    let note_of = |a: &AgentInfo| a.session.as_deref().and_then(|s| live.notes.iter().position(|n| n.session == s));
    let needs = |a: &AgentInfo| agents::needs_you(a.status, note_of(a).map(|i| &live.notes[i]));
    let running: std::collections::HashSet<&str> = live.agents.iter().filter_map(|a| a.session.as_deref()).collect();
    let earlier = notes::earlier(&live.notes, &running, now, EARLIER_DAYS, EARLIER_MAX);
    let groups: Vec<Value> = agents::groups(&live.agents, &stories, &parents)
        .iter()
        .map(|g| {
            json!({
                "label": g.label,
                "story": g.story,
                "entries": g.entries.iter().map(|e| {
                    let a = &live.agents[e.index];
                    let needs_you = needs(a);
                    json!({
                        "pane": a.pane,
                        "label": e.label,
                        "depth": e.depth,
                        "status": if needs_you { AgentStatus::Blocked.word() } else { a.status.word() },
                        "icon": if needs_you { AgentStatus::Blocked.icon() } else { a.status.icon() },
                        "state": if needs_you { "needs" } else { match a.status { AgentStatus::Working => "working", AgentStatus::Done => "done", _ => "idle" } },
                        "topic": a.topic,
                        "source": match crate::claude::entrypoint_of(&a.pane) { "claude-desktop" => "desktop", other => other },
                        "deepLink": crate::claude::desktop_link(&a.pane),
                        "note": note_of(a),
                    })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    let needing = live.agents.iter().filter(|a| needs(a)).count() + earlier.iter().filter(|i| live.notes[**i].needs_you).count();
    json!({
        "error": live.error,
        "groups": groups,
        "earlier": earlier,
        "notes": live.notes.iter().map(|n| json!({
            "session": n.session, "task": n.task, "progress": n.progress, "waiting": n.waiting, "needsYou": n.needs_you, "next": n.next, "updatedAt": n.updated_at,
        })).collect::<Vec<_>>(),
        "count": live.agents.len(),
        "working": live.agents.iter().filter(|a| a.status == AgentStatus::Working).count(),
        "needsYou": needing,
    })
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
        Arc::new(Server {
            root,
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
            let data = self.prs.lock().unwrap_or_else(|e| e.into_inner());
            let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            let mut urls = std::collections::HashSet::new();
            shown_urls(&state(&self.root, &data, &live, Utc::now()), &mut urls);
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

    fn route(&self, path: &str) -> Reply {
        match path {
            "/" | "/index.html" => reply("200 OK", "text/html; charset=utf-8", INDEX),
            "/app.js" => reply("200 OK", "text/javascript; charset=utf-8", SCRIPT),
            "/style.css" => reply("200 OK", "text/css; charset=utf-8", STYLE),
            "/favicon.svg" | "/favicon.ico" => reply("200 OK", "image/svg+xml", FAVICON),
            "/api/state" => {
                self.looked();
                let value = {
                    let data = self.prs.lock().unwrap_or_else(|e| e.into_inner());
                    let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
                    state(&self.root, &data, &live, Utc::now())
                };
                reply("200 OK", "application/json", serde_json::to_vec(&value).unwrap_or_default())
            }
            _ => reply("404 Not Found", "text/plain; charset=utf-8", "not found"),
        }
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
        let result = if !self.host_ok(host) {
            reply("403 Forbidden", "text/plain; charset=utf-8", "wrong host")
        } else if method == "POST" && path == "/open" {
            self.open(&headers, body)
        } else if method != "GET" && method != "HEAD" {
            reply("405 Method Not Allowed", "text/plain; charset=utf-8", "read-only")
        } else {
            self.route(path)
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
        shown_urls(&state(&s.root, &s.prs.lock().unwrap(), &s.live.lock().unwrap(), Utc::now()), &mut urls);
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
        let s = Arc::new(Server { root: s.root.clone(), prs: Mutex::new(PrData::default()), live: Mutex::new(Live::default()), opener: Mutex::new(None), last_open: Mutex::new(None), viewed: AtomicI64::new(0), attention: Attention::new(), wake: Mutex::new(None), connections: AtomicUsize::new(0), port });
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
