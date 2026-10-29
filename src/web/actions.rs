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
        let Ok(request) = serde_json::from_str::<Value>(body) else {
            return plain("400 Bad Request", "need json");
        };
        let Some(action) = request["action"].as_str() else {
            return plain("400 Bad Request", "need an action");
        };
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
            "story.new" => self.new_story(&text("key")),
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
        let link = new_session_link(&format!("/story-break-down {key}"), &folder);
        let Some(opener) = lock(&self.opener).clone() else {
            return answer(false, "no way to open the desktop app from here");
        };
        match opener.open(&link) {
            Ok(()) => answer(true, format!("opened a session for {key}: press Enter to start the break-down")),
            Err(e) => answer(false, format!("could not open the desktop app: {e}")),
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
