//! Tests of what the page can change, and of what the service does by itself.
use super::engine::lock;
use super::*;
use crate::fixstate::Store;
use crate::model::{PrState, TaskType};
use crate::runs::{Job, Run, RunState};
use std::sync::atomic::AtomicUsize;

struct Rig {
    _dir: tempfile::TempDir,
    server: Arc<Server>,
    calls: Arc<Mutex<Vec<String>>>,
    notices: Arc<Mutex<Vec<String>>>,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("stories");
    crate::ops::init_story(&root, "PROJ-1", "First story", None).unwrap();
    store::update(&root, "PROJ-1", |b| {
        let story = root.join("PROJ-1");
        crate::ops::add_task(&story, b, "Do it", TaskType::Pr, vec![], vec![]).map(|_| ())
    })
    .unwrap();
    let server = Server::new(root, 7878);
    server.load_settings(Some(dir.path().join("tui.json")));
    server.set_fix_store(Some(Store::new(dir.path().join("cache"))));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let notices = Arc::new(Mutex::new(Vec::new()));
    let log = |c: &Arc<Mutex<Vec<String>>>| c.clone();
    let (c1, c2, c3, c4, c5, c6) = (log(&calls), log(&calls), log(&calls), log(&calls), log(&calls), notices.clone());
    let mut tools = Tools::default();
    tools.ready = Arc::new(move |url| {
        c1.lock().unwrap().push(format!("ready {url}"));
        Ok(())
    });
    tools.draft = Arc::new(move |url| {
        c2.lock().unwrap().push(format!("draft {url}"));
        Ok(())
    });
    tools.copilot = Arc::new(move |url| {
        c3.lock().unwrap().push(format!("copilot {url}"));
        Ok(())
    });
    tools.label = Arc::new(move |repo, n, label, on| {
        c4.lock().unwrap().push(format!("label {repo}#{n} {label} {on}"));
        Ok(())
    });
    tools.update_branch = Arc::new(move |repo, n, head| {
        c5.lock().unwrap().push(format!("update {repo}#{n} {head}"));
        Ok(())
    });
    tools.notify = Arc::new(move |title, body| c6.lock().unwrap().push(format!("{title} | {body}")));
    server.set_tools(tools);
    Rig { _dir: dir, server, calls, notices }
}

fn pr(n: u64) -> PullRequest {
    let mut pr: PullRequest = serde_json::from_value(json!({
        "repo": "acme/widgets", "number": n, "title": format!("PR {n}"), "url": format!("https://github.com/acme/widgets/pull/{n}"),
        "is_draft": true, "labels": [], "review": "Required", "comments": 0, "updated_at": "2026-10-01T00:00:00Z", "created_at": format!("2026-10-01T00:{n:02}:00Z"),
        "checks": [],
        "feedback": { "mine": true, "head": format!("sha{n}"), "branch": format!("feature/{n}"), "copilot": "None", "copilot_reviewed_at": null, "threads": [] }
    }))
    .unwrap();
    pr.feedback.mine = true;
    pr
}

fn with_prs(rig: &Rig, items: Vec<PullRequest>) {
    lock(&rig.server.prs).items = items;
}

const HEADERS: &str = "X-Capcom: do\r\nOrigin: http://127.0.0.1:7878\r\nSec-Fetch-Site: same-origin\r\n";

fn act(rig: &Rig, body: Value) -> (String, Value) {
    let text = body.to_string();
    let request = format!("POST /api/do HTTP/1.1\r\nHost: 127.0.0.1:7878\r\n{HEADERS}Content-Length: {}\r\n\r\n{text}", text.len());
    let out = String::from_utf8_lossy(&rig.server.answer(&request)).to_string();
    let status = out.lines().next().unwrap_or_default().to_string();
    let json = out.split("\r\n\r\n").nth(1).and_then(|b| serde_json::from_str(b).ok()).unwrap_or(Value::Null);
    (status, json)
}

fn state(rig: &Rig) -> Value {
    rig.server.state_json(Utc::now())
}

fn calls(rig: &Rig) -> Vec<String> {
    rig.calls.lock().unwrap().clone()
}

#[test]
fn a_change_needs_the_page_headers_and_the_confirmed_ones_need_a_confirmation() {
    let r = rig();
    with_prs(&r, vec![pr(1)]);
    let url = "https://github.com/acme/widgets/pull/1";
    let body = json!({ "action": "pr.ready", "url": url, "confirm": true }).to_string();
    for headers in ["", "X-Capcom: open\r\n", "X-Capcom: do\r\nOrigin: https://evil.example\r\n", "X-Capcom: do\r\nSec-Fetch-Site: cross-site\r\n"] {
        let request = format!("POST /api/do HTTP/1.1\r\nHost: 127.0.0.1:7878\r\n{headers}Content-Length: {}\r\n\r\n{body}", body.len());
        let out = String::from_utf8_lossy(&r.server.answer(&request)).to_string();
        assert!(out.starts_with("HTTP/1.1 403"), "{headers:?}: {out}");
    }
    assert!(calls(&r).is_empty());
    let (status, _) = act(&r, json!({ "action": "pr.ready", "url": url }));
    assert!(status.contains("400"), "asked for a confirmation: {status}");
    assert!(calls(&r).is_empty(), "nothing was done without it");
    let (status, answer) = act(&r, json!({ "action": "pr.ready", "url": url, "confirm": true }));
    assert!(status.contains("200") && answer["ok"] == true, "{answer}");
    assert_eq!(calls(&r), vec![format!("ready {url}")]);
    let get = String::from_utf8_lossy(&r.server.answer("GET /api/do HTTP/1.1\r\nHost: 127.0.0.1:7878\r\n\r\n")).to_string();
    assert!(get.starts_with("HTTP/1.1 404"), "a GET does nothing: {get}");
}

#[test]
fn a_pr_goes_between_draft_and_ready_only_when_it_is_in_the_list_and_not_already_there() {
    let r = rig();
    let mut ready = pr(2);
    ready.is_draft = false;
    with_prs(&r, vec![pr(1), ready]);
    let confirm = |action: &str, n: u64| act(&r, json!({ "action": action, "url": format!("https://github.com/acme/widgets/pull/{n}"), "confirm": true })).1;
    assert_eq!(confirm("pr.ready", 1)["ok"], true);
    assert_eq!(confirm("pr.ready", 2)["ok"], false, "already ready");
    assert_eq!(confirm("pr.draft", 2)["ok"], true);
    assert_eq!(confirm("pr.draft", 1)["ok"], false, "already a draft");
    assert_eq!(confirm("pr.ready", 99)["ok"], false, "not in the list");
    assert_eq!(calls(&r).len(), 2);
}

#[test]
fn ticks_are_kept_in_the_settings_file_the_tui_reads_and_start_afresh_when_ticked_again() {
    let r = rig();
    with_prs(&r, vec![pr(1)]);
    let url = "https://github.com/acme/widgets/pull/1";
    act(&r, json!({ "action": "settings", "key": "autofix", "value": false }));
    assert_eq!(state(&r)["prs"][0]["autofix"], false);
    assert_eq!(act(&r, json!({ "action": "pr.autofix", "url": url, "on": true })).1["ok"], true);
    assert_eq!(state(&r)["prs"][0]["autofix"], true);
    let saved = crate::settings::load(&r._dir.path().join("tui.json"));
    assert_eq!(saved.autofix_prs, vec![(url.to_string(), true)], "the TUI reads the same file");
    act(&r, json!({ "action": "pr.autofix", "url": url, "on": false }));
    assert_eq!(state(&r)["prs"][0]["autofix"], false);
    assert_eq!(crate::settings::load(&r._dir.path().join("tui.json")).autofix_prs.len(), 1, "one entry per PR");
    // the default applies to a PR with no tick of its own, except one followed by link
    act(&r, json!({ "action": "settings", "key": "autofix", "value": true }));
    let mut followed = pr(2);
    followed.feedback.pasted = true;
    with_prs(&r, vec![pr(3), followed]);
    let prs = state(&r)["prs"].clone();
    let by = |n: u64| prs.as_array().unwrap().iter().find(|p| p["number"] == n).unwrap().clone();
    assert_eq!(by(3)["autofix"], true, "the default");
    assert_eq!(by(2)["autofix"], false, "a followed PR starts off whatever the default");
    let not_mine = {
        let mut p = pr(4);
        p.feedback.mine = false;
        p
    };
    with_prs(&r, vec![not_mine]);
    assert_eq!(act(&r, json!({ "action": "pr.autofix", "url": "https://github.com/acme/widgets/pull/4", "on": true })).1["ok"], false, "someone else's PR is left alone");
}

#[test]
fn settings_are_changed_one_by_one_and_unknown_ones_are_refused() {
    let r = rig();
    r.server.attention.set_terminal(false);
    assert_eq!(r.server.attention.wait(Duration::from_secs(5)), AWAY_WAIT);
    act(&r, json!({ "action": "settings", "key": "notify", "value": true }));
    assert_eq!(r.server.attention.wait(Duration::from_secs(5)), NOTIFY_AWAY, "notifications on: it cannot wait half an hour");
    act(&r, json!({ "action": "settings", "key": "notify", "value": false }));
    assert_eq!(r.server.attention.wait(Duration::from_secs(5)), AWAY_WAIT);
    for key in ["autofix", "autoreview", "auto_sync", "notify", "slow_when_away"] {
        assert_eq!(act(&r, json!({ "action": "settings", "key": key, "value": true })).1["ok"], true, "{key}");
    }
    let s = state(&r)["settings"].clone();
    assert_eq!(s, json!({ "autofix": true, "autoreview": true, "autoSync": true, "notify": true, "slowWhenAway": true }));
    assert_eq!(act(&r, json!({ "action": "settings", "key": "slow_when_away", "value": false })).1["ok"], true);
    assert_eq!(act(&r, json!({ "action": "settings", "key": "rm_rf", "value": true })).1["ok"], false);
    assert_eq!(act(&r, json!({ "action": "settings", "key": "notify" })).1["ok"], false, "needs true or false");
    assert!(!r.server.attention.slow_when_away() || state(&r)["settings"]["slowWhenAway"] == false);
}

#[test]
fn automerge_adds_the_first_label_and_takes_every_one_off() {
    let r = rig();
    let mut p = pr(1);
    p.feedback.kodiak = Some(crate::kodiak::Kodiak { automerge: vec!["automerge 🚀".into(), "automerge".into()], autoupdate: vec![], update_always: false });
    with_prs(&r, vec![p.clone()]);
    let url = "https://github.com/acme/widgets/pull/1";
    assert_eq!(state(&r)["prs"][0]["automerge"]["available"], true);
    assert_eq!(act(&r, json!({ "action": "pr.automerge", "url": url, "on": true, "confirm": true })).1["ok"], true);
    assert_eq!(calls(&r), vec!["label acme/widgets#1 automerge 🚀 true".to_string()]);
    p.labels = vec!["automerge 🚀".into(), "automerge".into()];
    with_prs(&r, vec![p]);
    act(&r, json!({ "action": "pr.automerge", "url": url, "on": false, "confirm": true }));
    assert_eq!(calls(&r)[1..], ["label acme/widgets#1 automerge 🚀 false".to_string(), "label acme/widgets#1 automerge false".to_string()]);
    with_prs(&r, vec![pr(2)]);
    assert_eq!(act(&r, json!({ "action": "pr.automerge", "url": "https://github.com/acme/widgets/pull/2", "on": true, "confirm": true })).1["ok"], false, "no merge bot in that repo");
}

#[test]
fn a_followed_pr_or_run_is_added_once_remembered_and_can_be_let_go() {
    let r = rig();
    let follow = |t: &str| act(&r, json!({ "action": "pr.follow", "text": t })).1;
    assert_eq!(follow("https://github.com/acme/api/pull/12/files")["ok"], true);
    assert_eq!(follow("https://github.com/acme/api/pull/12")["ok"], false, "already followed");
    assert_eq!(follow("https://github.com/acme/web/actions/runs/123/job/456")["ok"], true);
    assert_eq!(follow("https://github.com/acme/web/actions/runs/123")["ok"], false, "already watched");
    assert_eq!(follow("https://example.com/not-github")["ok"], false);
    let saved = crate::settings::load(&r._dir.path().join("tui.json"));
    assert_eq!(saved.pasted_prs, vec!["https://github.com/acme/api/pull/12".to_string()]);
    assert_eq!(saved.watched.len(), 1);
    assert_eq!(*lock(&r.server.eng.run_watched), vec![("acme/web".to_string(), 123)], "the poller is told");
    assert_eq!(lock(&r.server.eng.pasted).urls, saved.pasted_prs);
    assert_eq!(act(&r, json!({ "action": "pr.unfollow", "url": "https://github.com/acme/api/pull/12" })).1["ok"], true);
    assert_eq!(act(&r, json!({ "action": "pr.unfollow", "url": "https://github.com/acme/api/pull/12" })).1["ok"], false);
    assert_eq!(act(&r, json!({ "action": "run.unwatch", "repo": "acme/web", "id": 123 })).1["ok"], true);
    assert!(crate::settings::load(&r._dir.path().join("tui.json")).watched.is_empty());
}

#[test]
fn a_story_is_marked_done_or_reopened_through_the_same_rules_as_the_tui() {
    let r = rig();
    let key = "PROJ-1";
    let (_, answer) = act(&r, json!({ "action": "story.done", "key": key, "confirm": true }));
    assert_eq!(answer["ok"], false, "{answer}");
    assert!(answer["message"].as_str().unwrap().contains("unfinished"), "{answer}");
    store::update(&r.server.root, key, |b| {
        b.tasks[0].status = crate::model::Status::Done;
        Ok(())
    })
    .unwrap();
    let (_, answer) = act(&r, json!({ "action": "story.done", "key": key, "confirm": true }));
    assert_eq!(answer["ok"], true, "{answer}");
    assert!(answer["message"].as_str().unwrap().contains("review was skipped"));
    assert_eq!(state(&r)["stories"][0]["status"], "done");
    assert_eq!(act(&r, json!({ "action": "story.reopen", "key": key, "confirm": true })).1["ok"], true);
    assert_eq!(state(&r)["stories"][0]["status"], "in_progress");
    assert_eq!(act(&r, json!({ "action": "story.reopen", "key": key, "confirm": true })).1["ok"], false, "already open");
}

#[test]
fn finishing_a_task_checks_its_prs_and_offers_a_cleanup_session_when_it_mentions_a_worktree() {
    struct Merged;
    impl crate::refresh::PrLookup for Merged {
        fn state(&self, _url: &str) -> anyhow::Result<PrState> {
            Ok(PrState::Merged)
        }
    }
    let r = rig();
    let mut tools = r.server.tools();
    tools.lookup = Arc::new(Merged);
    r.server.set_tools(tools);
    store::update(&r.server.root, "PROJ-1", |b| {
        crate::ops::add_pr(b, "T1", "widgets", "https://github.com/acme/widgets/pull/9", PrState::Draft)?;
        b.tasks[0].status = crate::model::Status::Implementing;
        Ok(())
    })
    .unwrap();
    let file = r.server.root.join("PROJ-1").join("tasks");
    let task_file = std::fs::read_dir(&file).unwrap().next().unwrap().unwrap().path();
    let mut doc = std::fs::read_to_string(&task_file).unwrap();
    doc.push_str("\n## Progress\n- worktree at /tmp/work/proj-1-t1\n");
    std::fs::write(&task_file, doc).unwrap();
    let (_, answer) = act(&r, json!({ "action": "task.finish", "key": "PROJ-1", "id": "T1", "confirm": true }));
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(state(&r)["stories"][0]["tasks"][0]["status"], "done");
    let link = answer["cleanup"]["link"].as_str().expect("a cleanup session is offered");
    assert!(link.starts_with("claude://code/new?prompt="), "{link}");
    let detail = String::from_utf8_lossy(&r.server.answer("GET /api/task?story=PROJ-1&id=T1 HTTP/1.1\r\nHost: 127.0.0.1:7878\r\n\r\n")).to_string();
    let body: Value = serde_json::from_str(detail.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["worktrees"], true);
    assert_eq!(body["id"], "T1");
    let bad = String::from_utf8_lossy(&r.server.answer("GET /api/task?story=..%2Fx&id=T1 HTTP/1.1\r\nHost: 127.0.0.1:7878\r\n\r\n")).to_string();
    assert!(bad.starts_with("HTTP/1.1 400"), "{bad}");
}

#[test]
fn a_new_story_opens_a_desktop_session_with_the_break_down_typed_in() {
    struct Recorder(Mutex<Vec<String>>);
    impl Opener for Recorder {
        fn open(&self, url: &str) -> Result<(), String> {
            self.0.lock().unwrap().push(url.to_string());
            Ok(())
        }
    }
    let r = rig();
    let opened = Arc::new(Recorder(Mutex::new(Vec::new())));
    r.server.set_opener(opened.clone());
    r.server.set_workdir("/w/code".into());
    assert_eq!(act(&r, json!({ "action": "story.new", "key": "see PROJ-123 for it" })).1["ok"], false, "only a key, not a sentence");
    assert_eq!(act(&r, json!({ "action": "story.new", "key": "proj-123" })).1["ok"], true);
    assert_eq!(*opened.0.lock().unwrap(), vec!["claude://code/new?prompt=%2Fstory-break-down%20PROJ-123&folder=%2Fw%2Fcode".to_string()]);
}

#[test]
fn the_state_offers_the_skills_as_links_and_those_links_are_openable() {
    let r = rig();
    r.server.set_workdir("/w/code".into());
    let s = state(&r);
    let story = &s["stories"][0];
    assert_eq!(story["breakdown"], "claude://code/new?prompt=%2Fstory-break-down%20PROJ-1&folder=%2Fw%2Fcode");
    assert_eq!(story["tasks"][0]["plan"], "claude://code/new?prompt=%2Fstory-plan-task%20PROJ-1%20T1&folder=%2Fw%2Fcode");
    assert_eq!(story["tasks"][0]["implement"], "claude://code/new?prompt=%2Fstory-implement-task%20PROJ-1%20T1&folder=%2Fw%2Fcode");
    let mut shown = std::collections::HashSet::new();
    super::view::shown_urls(&s, &mut shown);
    assert!(shown.contains(story["review"].as_str().unwrap()));
    assert!(shown.contains(story["tasks"][0]["plan"].as_str().unwrap()));
}

#[test]
fn a_pr_that_needs_fixing_carries_a_link_to_a_session_with_the_fix_typed_in() {
    let r = rig();
    r.server.set_workdir("/w/code".into());
    let mut p = pr(1);
    p.is_draft = false;
    p.checks = serde_json::from_value(json!([{ "name": "build", "workflow": null, "state": "Failed", "started_at": null, "completed_at": "2026-10-01T00:00:00Z", "url": null }])).unwrap();
    with_prs(&r, vec![p]);
    let fix = state(&r)["prs"][0]["fix"].clone();
    assert_eq!(fix["why"], "1 failing check");
    assert_eq!(fix["prompt"], "/pr-address https://github.com/acme/widgets/pull/1");
    assert!(fix["link"].as_str().unwrap().starts_with("claude://code/new?prompt=%2Fpr-address%20https%3A%2F%2Fgithub.com"), "{fix}");    assert_eq!(fix["open"], fix["link"], "no session made it: a new one");
}

fn behind() -> PullRequest {
    let mut p = pr(1);
    p.is_draft = false;
    p.feedback.merge = crate::prs::MergeState::Behind;
    p.checks = serde_json::from_value(json!([{ "name": "build", "workflow": null, "state": "Passed", "started_at": null, "completed_at": "2026-10-01T00:00:00Z", "url": null }])).unwrap();
    p
}

fn settle(_r: &Rig) {
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_branch_that_is_only_behind_is_updated_through_github_once_with_no_agent() {
    let r = rig();
    act(&r, json!({ "action": "settings", "key": "autofix", "value": true }));
    act(&r, json!({ "action": "settings", "key": "autoreview", "value": false }));
    let items = vec![behind()];
    with_prs(&r, items.clone());
    r.server.on_prs(&items);
    settle(&r);
    assert_eq!(calls(&r), vec!["update acme/widgets#1 sha1".to_string()]);
    r.server.on_prs(&items);
    settle(&r);
    assert_eq!(calls(&r).len(), 1, "the same look does not update it again");
    assert!(state(&r)["events"][0]["text"].as_str().unwrap().contains("brought acme/widgets#1 up to date"));
}

#[test]
fn a_pr_the_autopilot_cannot_fix_alone_is_noted_and_notified_but_nothing_is_sent() {
    let r = rig();
    act(&r, json!({ "action": "settings", "key": "autofix", "value": true }));
    act(&r, json!({ "action": "settings", "key": "notify", "value": true }));
    act(&r, json!({ "action": "settings", "key": "autoreview", "value": false }));
    let mut p = pr(1);
    p.is_draft = false;
    p.checks = serde_json::from_value(json!([{ "name": "build", "workflow": null, "state": "Failed", "started_at": null, "completed_at": "2026-10-01T00:00:00Z", "url": null }])).unwrap();
    r.server.on_prs(&[p.clone()]);
    r.server.on_prs(&[p]);
    settle(&r);
    assert!(calls(&r).is_empty(), "no update, no copilot, nothing");
    let notes = r.notices.lock().unwrap().clone();
    assert_eq!(notes.len(), 1, "told once: {notes:?}");
    assert!(notes[0].contains("needs fixing") && notes[0].contains("1 failing check"), "{notes:?}");
}

#[test]
fn auto_review_asks_copilot_once_for_a_new_pr() {
    let r = rig();
    act(&r, json!({ "action": "settings", "key": "autoreview", "value": true }));
    let items = vec![pr(1)];
    r.server.on_prs(&items);
    r.server.on_prs(&items);
    settle(&r);
    assert_eq!(calls(&r), vec!["copilot https://github.com/acme/widgets/pull/1".to_string()]);
}

#[test]
fn a_pr_turning_green_for_a_reviewer_is_announced_once_and_only_when_notifications_are_on() {
    let r = rig();
    let mut waiting = pr(1);
    waiting.is_draft = false;
    waiting.checks = serde_json::from_value(json!([{ "name": "build", "workflow": null, "state": "Passed", "started_at": null, "completed_at": "2026-10-01T00:00:00Z", "url": null }])).unwrap();
    r.server.on_prs(&[pr(1)]);
    r.server.on_prs(&[waiting.clone()]);
    assert!(r.notices.lock().unwrap().is_empty(), "notifications are off");
    assert!(state(&r)["events"][0]["text"].as_str().unwrap().contains("needs a reviewer"), "but it is on the page's list");
    act(&r, json!({ "action": "settings", "key": "notify", "value": true }));
    r.server.on_prs(&[pr(1)]);
    r.server.on_prs(&[waiting.clone()]);
    r.server.on_prs(&[waiting]);
    assert_eq!(r.notices.lock().unwrap().len(), 1, "once per turn to green");
}

fn run(id: u64, state: RunState, watched: bool, updated: &str) -> Run {
    Run { repo: "acme/web".into(), id, name: "Deploy".into(), title: "Deploy to staging".into(), branch: "main".into(), url: format!("https://github.com/acme/web/actions/runs/{id}"), state, created_at: updated.into(), started_at: None, updated_at: updated.into(), jobs: vec![Job { name: "go".into(), state, url: String::new(), started_at: None, completed_at: None }], watched }
}

#[test]
fn a_run_that_finishes_is_announced_and_a_watched_one_leaves_half_an_hour_later() {
    let r = rig();
    act(&r, json!({ "action": "settings", "key": "notify", "value": true }));
    let now = Utc::now();
    let ago = |m: i64| (now - chrono::Duration::minutes(m)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    assert_eq!(r.server.settle_runs(vec![run(1, RunState::Running, false, &ago(1))]).len(), 1);
    assert!(r.notices.lock().unwrap().is_empty());
    let kept = r.server.settle_runs(vec![run(1, RunState::Success, false, &ago(0))]);
    assert_eq!(kept.len(), 1);
    let notes = r.notices.lock().unwrap().clone();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("is GREEN"), "{notes:?}");
    act(&r, json!({ "action": "pr.follow", "text": "https://github.com/acme/web/actions/runs/2" }));
    let fresh = r.server.settle_runs(vec![run(2, RunState::Success, true, &ago(45))]);
    assert_eq!(fresh.len(), 1, "followed just now, so it stays half an hour from now");
    r.server.change_settings(|s| s.watched[0].added_at = ago(45));
    let list = r.server.settle_runs(vec![run(2, RunState::Success, true, &ago(45))]);
    assert!(list.is_empty(), "a watched run that finished 45 minutes ago is dropped");
    assert!(crate::settings::load(&r._dir.path().join("tui.json")).watched.is_empty(), "and no longer watched");
}

#[test]
fn the_runs_are_in_the_state_for_the_page() {
    let r = rig();
    lock(&r.server.eng.runs).items = vec![run(7, RunState::Failed, false, "2026-10-01T00:00:00Z")];
    let runs = state(&r)["runs"].clone();
    assert_eq!(runs["items"][0]["state"], "failed");
    assert_eq!(runs["items"][0]["jobs"][0]["name"], "go");
    assert_eq!(runs["items"][0]["url"], "https://github.com/acme/web/actions/runs/7");
}

#[test]
fn a_tick_for_a_pr_that_is_no_longer_listed_is_forgotten() {
    let r = rig();
    with_prs(&r, vec![pr(1)]);
    act(&r, json!({ "action": "pr.autofix", "url": "https://github.com/acme/widgets/pull/1", "on": true }));
    r.server.on_prs(&[pr(2)]);
    assert!(r.server.settings().autofix_prs.is_empty());
    r.server.on_prs(&[]);
    let _ = AtomicUsize::new(0);
}

struct AlwaysMerged;
impl crate::refresh::PrLookup for AlwaysMerged {
    fn state(&self, _url: &str) -> anyhow::Result<PrState> {
        Ok(PrState::Merged)
    }
}

fn record_a_pr(r: &Rig) {
    store::update(&r.server.root, "PROJ-1", |b| {
        crate::ops::add_pr(b, "T1", "widgets", "https://github.com/acme/widgets/pull/9", PrState::Draft)?;
        b.tasks[0].status = crate::model::Status::Implementing;
        Ok(())
    })
    .unwrap();
    let mut tools = r.server.tools();
    tools.lookup = Arc::new(AlwaysMerged);
    r.server.set_tools(tools);
}

#[test]
fn a_recorded_pr_that_merged_shows_on_its_task_without_touching_the_board_unless_auto_sync_is_on() {
    let r = rig();
    record_a_pr(&r);
    assert_eq!(state(&r)["stories"][0]["tasks"][0]["prs"][0]["state"], "draft", "as recorded, until it has been looked up");
    r.server.check_recorded(None, false);
    let task = state(&r)["stories"][0]["tasks"][0].clone();
    assert_eq!(task["prs"][0]["state"], "merged");
    assert_eq!(task["allMerged"], true);
    let board = store::load(&r.server.root, "PROJ-1").unwrap();
    assert_eq!(board.tasks[0].prs[0].state, PrState::Draft, "auto-sync is off: the board is not written");
    act(&r, json!({ "action": "settings", "key": "auto_sync", "value": true }));
    r.server.check_recorded(None, false);
    let board = store::load(&r.server.root, "PROJ-1").unwrap();
    assert_eq!(board.tasks[0].prs[0].state, PrState::Merged, "auto-sync on: the board follows GitHub");
    assert!(state(&r)["events"][0]["text"].as_str().unwrap().contains("auto-sync PROJ-1"));
}

#[test]
fn an_open_pr_in_the_list_is_not_looked_up_again_and_says_draft_or_ready_for_its_task() {
    let r = rig();
    record_a_pr(&r);
    let mut open = pr(9);
    open.is_draft = false;
    with_prs(&r, vec![open]);
    assert!(r.server.recorded_urls().is_empty(), "the open list vouches for it");
    assert_eq!(state(&r)["stories"][0]["tasks"][0]["prs"][0]["state"], "ready", "the live list wins over the board");
}

#[test]
fn the_plan_usage_the_status_line_wrote_is_in_the_state_and_missing_is_null() {
    let r = rig();
    assert_eq!(state(&r)["usage"], Value::Null, "nothing written yet");
    let file = r._dir.path().join("usage.json");
    r.server.set_usage_file(Some(file.clone()));
    assert_eq!(state(&r)["usage"], Value::Null, "no file");
    std::fs::write(&file, r#"{"at": 1790000000, "rate_limits": {"five_hour": {"used_percentage": 23.5, "resets_at": 4102444800}, "seven_day": {"used_percentage": 41}}}"#).unwrap();
    let usage = state(&r)["usage"].clone();
    assert_eq!(usage["fiveHour"]["pct"], 23.5);
    assert_eq!(usage["fiveHour"]["resetsAt"], "2100-01-01T00:00:00Z");
    assert_eq!(usage["sevenDay"]["pct"], 41.0);
    assert_eq!(usage["sevenDay"]["resetsAt"], Value::Null);
    assert_eq!(usage["at"], "2026-09-21T14:13:20Z");
}
