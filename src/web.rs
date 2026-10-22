//! `capcom web`: the boards and your open PRs as a page on localhost, read-only, polling GitHub by
//! itself (and only while a browser is looking). It listens on 127.0.0.1 only, answers GET only,
//! and refuses a `Host` that is not itself, so another website cannot read it through your browser.
use crate::attention::Attention;
use crate::model::{Board, Status, Task};
use crate::prs::{self, CheckState, CopilotState, MergeState, PrMsg, PrSource, PullRequest, Review, Waiting};
use crate::store;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
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
const MAX_CONNECTIONS: usize = 16;
const MAX_REQUEST: usize = 8192;
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

#[derive(Default)]
struct PrData {
    items: Vec<PullRequest>,
    error: Option<String>,
    updated: Option<DateTime<Utc>>,
    loading: bool,
}

pub struct Server {
    root: PathBuf,
    prs: Mutex<PrData>,
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
fn state(root: &std::path::Path, data: &PrData, now: DateTime<Utc>) -> Value {
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
            let looking = Utc::now().timestamp() - me.viewed.load(Ordering::Relaxed) < VIEWED_FOR;
            me.attention.set_terminal(looking);
            std::thread::sleep(Duration::from_secs(2));
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
                    state(&self.root, &data, Utc::now())
                };
                reply("200 OK", "application/json", serde_json::to_vec(&value).unwrap_or_default())
            }
            _ => reply("404 Not Found", "text/plain; charset=utf-8", "not found"),
        }
    }

    /// The answer to one request, as the bytes to send back.
    pub fn answer(&self, request: &str) -> Vec<u8> {
        let mut lines = request.split("\r\n");
        let first = lines.next().unwrap_or_default();
        let mut parts = first.split(' ');
        let (method, target) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
        let host = lines.filter_map(|l| l.split_once(':')).find(|(k, _)| k.eq_ignore_ascii_case("host")).map(|(_, v)| v.to_string()).unwrap_or_default();
        let result = if method != "GET" && method != "HEAD" {
            reply("405 Method Not Allowed", "text/plain; charset=utf-8", "read-only")
        } else if !self.host_ok(&host) {
            reply("403 Forbidden", "text/plain; charset=utf-8", "wrong host")
        } else {
            self.route(target.split(['?', '#']).next().unwrap_or("/"))
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
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < MAX_REQUEST {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        if !buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return;
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
        let s = Arc::new(Server { root: s.root.clone(), prs: Mutex::new(PrData::default()), viewed: AtomicI64::new(0), attention: Attention::new(), wake: Mutex::new(None), connections: AtomicUsize::new(0), port });
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
