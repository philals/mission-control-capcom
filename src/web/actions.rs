//! `POST /api/do`: the changes the page can make. Each one is checked against what the service itself
//! knows (the PR must be in the list, the story must exist, the transition must be allowed), and the
//! ones that change something that is hard to undo need `"confirm": true`, which the page only sends
//! after you click the confirm button.
use super::engine::lock;
use super::view::{new_session_link, shown_urls};
use super::*;
use crate::finish::Msg;
use crate::model::StoryStatus;

/// Actions that need an explicit confirmation from the page.
const CONFIRMED: [&str; 8] = ["pr.ready", "pr.draft", "pr.automerge", "story.done", "story.reopen", "task.finish", "story.sync", "story.delete"];

fn answer(ok: bool, message: impl Into<String>) -> Value {
    json!({ "ok": ok, "message": message.into() })
}

fn jira_key(text: &str) -> Option<String> {
    let upper = text.trim().to_uppercase();
    let (project, number) = upper.rsplit_once('-')?;
    let valid = !project.is_empty()
        && project.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && project.chars().all(|c| c.is_ascii_alphanumeric())
        && !number.is_empty()
        && number.len() <= 9
        && number.chars().all(|c| c.is_ascii_digit());
    valid.then_some(upper)
}

impl Server {
    /// The request body of `/api/do`, as the page sends it.
    pub(super) fn do_request(&self, headers: &[(String, String)], body: &str) -> Reply {
        let h = |name: &str| headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        let plain = |status: &'static str, text: &'static str| reply(status, "text/plain; charset=utf-8", text);
        if h("x-capcom") != Some("do") || h("origin").is_some_and(|o| !self.own_origin(o)) || h("sec-fetch-site").is_some_and(|s| s != "same-origin") {
            return plain("403 Forbidden", "not from the page");
        }
        let Ok(mut request) = serde_json::from_str::<Value>(body) else {
            return plain("400 Bad Request", "need json");
        };
        let Some(action) = request["action"].as_str().map(str::to_string) else {
            return plain("400 Bad Request", "need an action");
        };
        // the app the page is open in: the page's `X-Capcom-Host` header, or a `host` field
        if let (Some(host), true) = (h("x-capcom-host"), request.get("host").and_then(Value::as_str).is_none_or(str::is_empty)) {
            request["host"] = json!(host.to_ascii_lowercase());
        }
        let action = action.as_str();
        if CONFIRMED.contains(&action) && request["confirm"] != json!(true) {
            return plain("400 Bad Request", "this one needs a confirmation");
        }
        let result = self.perform(action, &request);
        reply("200 OK", "application/json", serde_json::to_vec(&result).unwrap_or_default())
    }

    fn live_pr(&self, url: &str) -> Option<PullRequest> {
        lock(&self.prs).items.iter().find(|p| same_url(&p.url, url)).cloned()
    }

    fn perform(&self, action: &str, r: &Value) -> Value {
        let text = |key: &str| r[key].as_str().unwrap_or_default().to_string();
        match action {
            "refresh" => {
                self.eng.check_now.store(true, std::sync::atomic::Ordering::SeqCst);
                self.wake_feeds();
                answer(true, "refreshing")
            }
            "pr.ready" | "pr.draft" => self.set_draft(&text("url"), action == "pr.draft"),
            "pr.automerge" => self.set_automerge(&text("url"), r["on"].as_bool().unwrap_or(false)),
            "pr.autofix" => self.set_tick(&text("url"), r["on"].as_bool().unwrap_or(false), true),
            "pr.autoreview" => self.set_tick(&text("url"), r["on"].as_bool().unwrap_or(false), false),
            "pr.follow" => self.follow(&text("text")),
            "pr.unfollow" => self.unfollow_pr(&text("url")),
            "run.unwatch" => self.unwatch_run(&text("repo"), r["id"].as_u64().unwrap_or(0)),
            "settings" => self.set_setting(&text("key"), r["value"].as_bool()),
            "story.done" => self.finish_story(&text("key")),
            "story.reopen" => self.reopen_story(&text("key")),
            "story.delete" => self.delete_story(&text("key")),
            "story.sync" => self.sync_story(&text("key")),
            "task.finish" => self.finish_task(&text("key"), &text("id")),
            "task.folder" => self.set_task_folder(&text("key"), &text("id"), &text("folder")),
            "story.new" => self.new_story(&text("key")),
            "session.start" => self.start_session(&text("key"), &text("task"), &text("kind"), &text("host")),
            "agent.open" => self.open_agent(&text("pane")),
            _ => answer(false, "unknown action"),
        }
    }

    fn set_draft(&self, url: &str, to_draft: bool) -> Value {
        let Some(pr) = self.live_pr(url) else {
            return answer(false, "that pull request is not in the list");
        };
        if pr.is_draft == to_draft {
            return answer(false, if to_draft { "it is a draft already" } else { "it is ready for review already" });
        }
        let tools = self.tools();
        let name = format!("{}#{}", pr.repo, pr.number);
        let result = if to_draft { (tools.draft)(&pr.url) } else { (tools.ready)(&pr.url) };
        self.wake_feeds();
        match (result, to_draft) {
            (Ok(()), false) => {
                self.event(format!("{name} is ready for review"));
                answer(true, format!("{name} is ready for review"))
            }
            (Ok(()), true) => {
                self.event(format!("{name} is a draft again"));
                answer(true, format!("{name} is a draft again"))
            }
            (Err(e), false) => answer(false, format!("could not mark {name} ready: {e}")),
            (Err(e), true) => answer(false, format!("could not turn {name} back into a draft: {e}")),
        }
    }

    fn set_automerge(&self, url: &str, on: bool) -> Value {
        let Some(pr) = self.live_pr(url).filter(|p| p.managed() && !p.automerge_labels().is_empty()) else {
            return answer(false, "automerge needs your own or a followed PR in a repo whose merge bot is set up (a .kodiak.toml)");
        };
        let name = format!("{}#{}", pr.repo, pr.number);
        let (set, repo, number) = (tools_label(self), pr.repo.clone(), pr.number);
        let result = if on {
            let first = pr.automerge_labels()[0].clone();
            set(&repo, number, &first, true).map(|()| format!("automerge is on for {name} (added \"{first}\")"))
        } else {
            let present: Vec<String> = pr.automerge_labels().iter().filter(|l| pr.has_label(l)).cloned().collect();
            match present.iter().find_map(|l| set(&repo, number, l, false).err()) {
                None => Ok(format!("automerge is off for {name}")),
                Some(e) => Err(e),
            }
        };
        self.wake_feeds();
        match result {
            Ok(message) => {
                self.event(message.clone());
                answer(true, message)
            }
            Err(e) => answer(false, format!("could not change automerge for {name}: {e}")),
        }
    }

    fn set_tick(&self, url: &str, on: bool, fix: bool) -> Value {
        let Some(pr) = self.live_pr(url).filter(PullRequest::managed) else {
            return answer(false, "you did not write this PR and are not following it, so capcom leaves it alone");
        };
        let url = pr.url.clone();
        self.change_settings(|s| {
            let list = if fix { &mut s.autofix_prs } else { &mut s.autocopilot_prs };
            list.retain(|(u, _)| !same_url(u, &url));
            list.push((url.clone(), on));
        });
        if on {
            if let Some(store) = lock(&self.eng.fix_store).as_ref() {
                if fix {
                    store.clear(&url);
                } else {
                    store.forget_copilot(&url);
                }
            }
            let mut auto = lock(&self.eng.auto);
            if fix {
                auto.fix_seen.remove(&url);
            } else {
                auto.copilot_tried.remove(&url);
            }
        }
        // look at the PR right away with the new tick
        let items = lock(&self.prs).items.clone();
        if let Some(me) = self.me.upgrade() {
            me.run_autopilot(&items);
        }
        answer(true, format!("{} {} for {}#{}", if fix { "auto-fix" } else { "auto-review" }, if on { "on" } else { "off" }, pr.repo, pr.number))
    }

    fn follow(&self, text: &str) -> Value {
        if let Some((owner, name, number)) = prs::pr_ref(text) {
            let url = format!("https://github.com/{owner}/{name}/pull/{number}");
            let known = self.settings().pasted_prs.iter().any(|u| same_url(u, &url)) || lock(&self.prs).items.iter().any(|p| p.feedback.mine && same_url(&p.url, &url));
            if known {
                return answer(false, format!("{owner}/{name}#{number} is already in the list"));
            }
            self.change_settings(|s| s.pasted_prs.push(url.clone()));
            return answer(true, format!("following {owner}/{name}#{number}: auto-fix and auto-review start off for it"));
        }
        if let Some((repo, id)) = crate::runs::parse_run_url(text) {
            if self.settings().watched.iter().any(|w| w.repo.eq_ignore_ascii_case(&repo) && w.id == id) {
                return answer(false, format!("already watching {repo} run {id}"));
            }
            let added_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            self.change_settings(|s| s.watched.push(crate::runs::Watch { repo: repo.clone(), id, added_at }));
            return answer(true, format!("watching {repo} run {id}: you are told when it finishes"));
        }
        answer(false, "that is not a link to a pull request or a workflow run on GitHub")
    }

    fn unfollow_pr(&self, url: &str) -> Value {
        if !self.settings().pasted_prs.iter().any(|u| same_url(u, url)) {
            return answer(false, "only a PR you followed by link can be taken out of the list");
        }
        self.change_settings(|s| {
            s.pasted_prs.retain(|u| !same_url(u, url));
            s.autofix_prs.retain(|(u, _)| !same_url(u, url));
            s.autocopilot_prs.retain(|(u, _)| !same_url(u, url));
        });
        answer(true, "no longer followed")
    }

    fn unwatch_run(&self, repo: &str, id: u64) -> Value {
        if !self.settings().watched.iter().any(|w| w.repo.eq_ignore_ascii_case(repo) && w.id == id) {
            return answer(false, "that run is not one you are watching");
        }
        self.change_settings(|s| s.watched.retain(|w| !(w.repo.eq_ignore_ascii_case(repo) && w.id == id)));
        answer(true, format!("no longer watching {repo} run {id}"))
    }

    fn set_setting(&self, key: &str, value: Option<bool>) -> Value {
        let Some(on) = value else {
            return answer(false, "need true or false");
        };
        let known = ["autofix", "autoreview", "auto_sync", "notify", "slow_when_away"];
        if !known.contains(&key) {
            return answer(false, "unknown setting");
        }
        self.change_settings(|s| match key {
            "autofix" => s.autofix = on,
            "autoreview" => s.autocopilot = on,
            "auto_sync" => s.auto_sync = on,
            "notify" => s.notify = on,
            _ => s.slow_when_away = on,
        });
        if key == "auto_sync" && on {
            self.eng.check_now.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        answer(true, format!("{key} {}", if on { "on" } else { "off" }))
    }

    fn finish_story(&self, key: &str) -> Value {
        let skipped = std::cell::Cell::new(false);
        let result = store::update(&self.root, key, |b| {
            if b.story.status == StoryStatus::InProgress {
                crate::rules::story_transition(b, StoryStatus::InReview)?;
                skipped.set(true);
            }
            crate::rules::story_transition(b, StoryStatus::Done)
        });
        match result {
            Ok(()) => answer(true, format!("{key} is done{}", if skipped.get() { " (the review was skipped)" } else { "" })),
            Err(e) => answer(false, format!("could not mark {key} done: {e:#}")),
        }
    }

    fn reopen_story(&self, key: &str) -> Value {
        match store::update(&self.root, key, |b| crate::rules::story_transition(b, StoryStatus::InProgress)) {
            Ok(()) => answer(true, format!("{key} is back in DOING")),
            Err(e) => answer(false, format!("could not reopen {key}: {e:#}")),
        }
    }

    fn delete_story(&self, key: &str) -> Value {
        match store::delete(&self.root, key) {
            Ok(()) => answer(true, format!("{key} is deleted, with its board and task files")),
            Err(e) => answer(false, format!("could not delete {key}: {e:#}")),
        }
    }

    fn lookup(&self) -> Arc<dyn crate::refresh::PrLookup + Send + Sync> {
        self.tools().lookup
    }

    fn messages(&self, key: &str, msgs: Vec<Msg>, cleanup_for: &str) -> Value {
        let mut notes = Vec::new();
        let mut cleanup = false;
        for m in msgs {
            match m {
                Msg::Notice(t) => notes.push(t),
                Msg::Cleanup(_) => cleanup = true,
            }
        }
        let folder = lock(&self.eng.workdir).clone();
        let mut out = answer(true, notes.join(" · "));
        if cleanup {
            let prompt = format!("Clean up the git worktrees that task {cleanup_for} of story {key} created: list them, check each is merged or abandoned, and remove the ones that are.");
            out["cleanup"] = json!({ "link": new_session_link(&prompt, &folder) });
        }
        out
    }

    fn finish_task(&self, key: &str, id: &str) -> Value {
        let msgs = crate::finish::finish(&self.root, key, id, self.lookup().as_ref());
        self.messages(key, msgs, id)
    }

    fn set_task_folder(&self, key: &str, id: &str, folder: &str) -> Value {
        if store::load(&self.root, key).is_err() {
            return answer(false, format!("no story {key}"));
        }
        match store::update(&self.root, key, |b| crate::ops::set_folder(b, id, Some(folder))) {
            Ok(()) if folder.trim().is_empty() => answer(true, format!("{id}'s folder is cleared")),
            Ok(()) => answer(true, format!("{id}'s folder is set")),
            Err(e) => answer(false, format!("could not set the folder: {e:#}")),
        }
    }

    fn sync_story(&self, key: &str) -> Value {
        let msgs = crate::finish::sync_story(&self.root, key, self.lookup().as_ref());
        self.messages(key, msgs, "")
    }

    /// Open a new Claude Code session that starts breaking a story down.
    fn new_story(&self, text: &str) -> Value {
        let Some(key) = jira_key(text) else {
            return answer(false, "no Jira key found in that text");
        };
        let folder = lock(&self.eng.workdir).clone();
        let link = new_session_link(&format!("/story-plan {key}"), &folder);
        let Some(opener) = lock(&self.opener).clone() else {
            return answer(false, "no way to open the desktop app from here");
        };
        match opener.open(&link) {
            Ok(()) => answer(true, format!("opened a session for {key}: press Enter to start the plan")),
            Err(e) => answer(false, format!("could not open the desktop app: {e}")),
        }
    }
}

impl Server {
    /// Start a skill's session in the app the page is open in: Orca for `host: "orca"`, else the desktop
    /// app (returned as a link for the page to open, as before). A task whose session is already running
    /// anywhere, or that was started moments ago, is not started again.
    fn start_session(&self, key: &str, task: &str, kind: &str, host: &str) -> Value {
        use crate::host::Host;
        let Ok(board) = store::load(&self.root, key) else {
            return answer(false, format!("no story {key}"));
        };
        let skill = match kind {
            "plan" => "story-plan",
            "implement" => "story-implement-task",
            "review" => "story-review",
            "breakdown" => "story-plan",
            _ => return answer(false, "unknown kind of session"),
        };
        let workdir = lock(&self.eng.workdir).clone();
        let (prompt, folder, task, sessions) = if task.is_empty() {
            (format!("/{skill} {key}"), workdir, None, Vec::new())
        } else {
            let Some(t) = board.task(task) else {
                return answer(false, format!("no task {task} in {key}"));
            };
            if host == "orca" {
                if let Err(e) = crate::orca::check_task_folder(key, task, t.folder.as_deref()) {
                    return answer(false, e);
                }
            }
            let folder = t.folder.as_deref().map(std::path::PathBuf::from).filter(|f| f.is_dir()).unwrap_or(workdir);
            (format!("/{skill} {key} {task}"), folder, Some(task), t.sessions.iter().filter(|s| s.skill == skill).map(|s| s.id.clone()).collect::<Vec<_>>())
        };
        let host = match host {
            "orca" => Host::Orca,
            "" | "desktop" => Host::Desktop,
            "herdr" | "herdr-app" => return answer(false, "the page can't start sessions in Herdr: use capcom-tui in a Herdr pane"),
            other => return answer(false, format!("unknown app {other:?}: use orca or desktop")),
        };
        let marks = lock(&self.starting).clone();
        let now = Utc::now().timestamp().max(0) as u64;
        if let (Some(t), Some(m)) = (task, &marks) {
            if let Some(h) = m.live(key, t, now) {
                return answer(false, format!("{key} {t} is already starting in {}: give it a moment", h.name()));
            }
        }
        let running = lock(&self.live).agents.iter().find(|a| a.session.as_ref().is_some_and(|s| sessions.contains(s))).map(|a| a.pane.clone());
        if let Some(pane) = running {
            let mut out = self.open_agent(&pane);
            out["message"] = json!(format!("{key} {} is already running: {}", task.unwrap_or_default(), out["message"].as_str().unwrap_or("opening it")));
            return out;
        }
        if let (Some(t), Some(m)) = (task, &marks) {
            if !m.mark(key, t, host, now) {
                let h = m.live(key, t, now).map_or("another app", |h| h.name());
                return answer(false, format!("{key} {t} is already starting in {h}: give it a moment"));
            }
        }
        let clear = || {
            if let (Some(t), Some(m)) = (task, &marks) {
                m.clear(key, t);
            }
        };
        match host {
            Host::Orca => {
                let Some(orca) = lock(&self.orca).clone() else {
                    clear();
                    return answer(false, "Orca: its command-line tool was not found (open Orca, or register its CLI)");
                };
                match orca.launch(&folder.to_string_lossy(), &prompt, true) {
                    Ok(_) => answer(true, format!("started {prompt} in Orca")),
                    Err(e) => {
                        clear();
                        answer(false, e)
                    }
                }
            }
            _ => json!({ "ok": true, "link": new_session_link(&prompt, &folder) }),
        }
    }

    /// Go to a running agent in the app it runs in: switch to it in Orca, or a link the page opens for
    /// the desktop app. Herdr and plain terminals cannot be reached from the page.
    fn open_agent(&self, pane: &str) -> Value {
        use crate::host::{of_pane, Host};
        match of_pane(pane) {
            Host::Orca => {
                let (Some(orca), Some(handle)) = (lock(&self.orca).clone(), crate::orca::handle_of(pane)) else {
                    return answer(false, "Orca: its command-line tool was not found");
                };
                match orca.switch(handle) {
                    Ok(()) => answer(true, "switched to it in Orca"),
                    Err(e) => answer(false, e),
                }
            }
            Host::Desktop => match crate::claude::desktop_link(pane) {
                Some(link) => json!({ "ok": true, "link": link }),
                None => answer(false, "the desktop app gave this session no id to open it by"),
            },
            Host::Plain => answer(false, "it's running in a terminal: switch to that terminal to reach it"),
            h => answer(false, format!("it's running in {}; switch to {} to reach it", h.name(), h.name())),
        }
    }
}

fn tools_label(server: &Server) -> Arc<dyn Fn(&str, u64, &str, bool) -> Result<(), String> + Send + Sync> {
    server.tools().label
}

/// Whether `url` is one the page shows (for the other module that opens links).
#[allow(dead_code)]
pub(super) fn is_shown(state: &Value, url: &str) -> bool {
    let mut urls = std::collections::HashSet::new();
    shown_urls(state, &mut urls);
    urls.contains(url)
}
