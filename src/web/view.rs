//! The JSON the page draws, built from everything the server knows.
use super::*;
use crate::durations::{time_left, Durations};
use crate::runs::{Run, RunState};
use crate::settings::Settings;
use super::engine::lock;

pub(super) fn same_url(a: &str, b: &str) -> bool {
    a.trim_end_matches('/').eq_ignore_ascii_case(b.trim_end_matches('/'))
}

pub(super) fn check_state(state: CheckState) -> &'static str {
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
pub(super) fn rail(pr: &PullRequest) -> Vec<Value> {
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

/// What the views need besides the PR itself.
pub(super) struct Ctx<'a> {
    pub boards: &'a [Board],
    pub settings: &'a Settings,
    pub durations: &'a Durations,
    pub now: DateTime<Utc>,
    /// The PRs and the Claude Code session that made each, from the transcripts.
    pub sessions: &'a [(String, PrSession)],
    pub agents: &'a [AgentInfo],
    /// What GitHub last said about each PR recorded on a task, and your open PRs.
    pub known: &'a std::collections::HashMap<String, crate::model::PrState>,
    pub open: &'a [PullRequest],
    /// The folder new Claude Code sessions start in.
    pub workdir: &'a std::path::Path,
}

fn percent(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The link that opens a new Claude Code desktop session in `folder` with `prompt` typed in (not sent).
pub(super) fn new_session_link(prompt: &str, folder: &std::path::Path) -> String {
    format!("claude://code/new?prompt={}&folder={}", percent(prompt), percent(&folder.to_string_lossy()))
}

fn effective(url: &str, per_pr: &[(String, bool)], default: bool, pasted: bool) -> bool {
    per_pr.iter().find(|(u, _)| same_url(u, url)).map_or(default && !pasted, |(_, on)| *on)
}

pub(super) fn autofix_on(settings: &Settings, pr: &PullRequest) -> bool {
    effective(&pr.url, &settings.autofix_prs, settings.autofix, pr.feedback.pasted)
}

pub(super) fn autoreview_on(settings: &Settings, pr: &PullRequest) -> bool {
    effective(&pr.url, &settings.autocopilot_prs, settings.autocopilot, pr.feedback.pasted)
}

/// The folder a fix for this PR starts in: where the session that made it began, else the repo's folder
/// under the work folder when it exists, else the work folder.
fn fix_folder(pr: &PullRequest, cx: &Ctx) -> std::path::PathBuf {
    if let Some((_, s)) = cx.sessions.iter().find(|(u, _)| same_url(u, &pr.url)) {
        if std::path::Path::new(&s.cwd).is_dir() {
            return s.cwd.clone().into();
        }
    }
    let repo = pr.repo.rsplit('/').next().unwrap_or_default();
    let under = cx.workdir.join(repo);
    if !repo.is_empty() && under.is_dir() {
        under
    } else {
        cx.workdir.to_path_buf()
    }
}

/// The link that goes back to the session that made a PR: the desktop app's own link when that session is
/// running there, else the app's import link for a terminal session.
fn agent_link(pr: &PullRequest, cx: &Ctx) -> Value {
    let Some((_, made)) = cx.sessions.iter().find(|(u, _)| same_url(u, &pr.url)) else {
        return Value::Null;
    };
    let live = cx.agents.iter().find(|a| a.session.as_deref() == Some(made.session.as_str()));
    let link = live.and_then(|a| crate::claude::desktop_link(&a.pane)).or_else(|| crate::claude::import_link(&made.session));
    json!({ "session": made.session, "running": live.is_some(), "link": link })
}

pub(super) fn pr_view(pr: &PullRequest, cx: &Ctx) -> Value {
    let boards = cx.boards;
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
    let need = crate::autopilot::fix_needed(pr);
    let agent = agent_link(pr, cx);
    let fix = need.as_ref().map(|n| {
        let prompt = format!("/pr-address {}", pr.url);
        let link = new_session_link(&prompt, &fix_folder(pr, cx));
        // The session that made the PR, when it is open in the desktop app; else a new one with the prompt typed in.
        let open = agent["link"].as_str().filter(|l| agent["running"] == true && l.starts_with("claude://code/continue")).map_or(link.clone(), str::to_string);
        json!({ "why": n.why, "onlyBehind": n.only_behind, "prompt": prompt, "link": link, "open": open })
    });
    let kodiak = pr.feedback.kodiak.as_ref();
    json!({
        "repo": pr.repo,
        "number": pr.number,
        "title": pr.title,
        "url": pr.url,
        "branch": pr.feedback.branch,
        "draft": pr.is_draft,
        "mine": pr.feedback.mine,
        "pasted": pr.feedback.pasted,
        "managed": pr.managed(),
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
        "copilot": match pr.feedback.copilot { CopilotState::None => "none", CopilotState::Requested => "requested", CopilotState::Reviewed => "reviewed" },
        "copilotOpen": pr.copilot_open().len(),
        "copilotReviewedAt": pr.feedback.copilot_reviewed_at,
        "merge": match pr.feedback.merge { MergeState::Clean => "clean", MergeState::Behind => "behind", MergeState::Conflicting => "conflicting", MergeState::Unknown => "unknown" },
        "automerge": { "available": !pr.automerge_labels().is_empty(), "on": pr.automerge_on(), "labels": pr.automerge_labels(), "botUpdates": pr.bot_updates(), "kodiak": kodiak.is_some() },
        "autofix": autofix_on(cx.settings, pr),
        "autoreview": autoreview_on(cx.settings, pr),
        "fix": fix,
        "agent": agent,
        "leftSeconds": time_left(pr, cx.now, cx.durations),
        "counts": { "passed": c.passed, "failed": c.failed, "running": c.running, "queued": c.queued, "skipped": c.skipped, "cancelled": c.cancelled },
        "checks": pr.ordered_checks().iter().map(|k| json!({
            "label": k.label(),
            "state": check_state(k.state),
            "startedAt": k.started_at,
            "completedAt": k.completed_at,
            "url": k.url,
            "external": k.external,
            "usual": if k.external { None } else { cx.durations.estimate(&pr.repo, k) },
        })).collect::<Vec<_>>(),
    })
}

pub(super) fn task_view(board: &Board, task: &Task, cx: &Ctx) -> Value {
    let done = |id: &str| board.task(id).is_some_and(|t| t.status == Status::Done);
    let waiting_on: Vec<&String> = task.depends_on.iter().filter(|d| !done(d)).collect();
    let key = &board.story.key;
    let launch = |skill: &str| new_session_link(&format!("/{skill} {key} {}", task.id), cx.workdir);
    // an open PR says whether it is a draft; otherwise what GitHub last said; otherwise what the board recorded
    let state_of = |p: &crate::model::Pr| {
        cx.open.iter().find(|o| same_url(&o.url, &p.url)).map(|o| if o.is_draft { crate::model::PrState::Draft } else { crate::model::PrState::Ready })
            .or_else(|| cx.known.iter().find(|(u, _)| same_url(u, &p.url)).map(|(_, s)| *s))
            .unwrap_or(p.state)
    };
    let all_merged = !task.prs.is_empty() && task.prs.iter().all(|p| matches!(state_of(p), crate::model::PrState::Merged | crate::model::PrState::Closed));
    json!({
        "id": task.id,
        "title": task.title,
        "type": task.kind.as_str(),
        "status": task.status.as_str(),
        "blocked": task.blocked.as_ref().map(|b| b.reason.clone()),
        "dependsOn": task.depends_on,
        "waitingOn": waiting_on,
        "repos": task.repos,
        "prs": task.prs.iter().map(|p| json!({ "repo": p.repo, "url": p.url, "state": state_of(p).as_str() })).collect::<Vec<_>>(),
        "working": task.agent.is_some(),
        "allMerged": all_merged,
        "jira": task.jira_subtask,
        "plan": launch("story-plan-task"),
        "implement": launch("story-implement-task"),
    })
}

fn run_state(state: RunState) -> &'static str {
    match state {
        RunState::Queued => "queued",
        RunState::Running => "running",
        RunState::Waiting => "waiting",
        RunState::Success => "success",
        RunState::Failed => "failed",
        RunState::Cancelled => "cancelled",
        RunState::Skipped => "skipped",
    }
}

pub(super) fn run_view(run: &Run) -> Value {
    json!({
        "repo": run.repo,
        "id": run.id,
        "name": run.name,
        "title": run.title,
        "branch": run.branch,
        "url": run.url,
        "state": run_state(run.state),
        "active": run.is_active(),
        "watched": run.watched,
        "createdAt": run.created_at,
        "startedAt": run.started_at,
        "updatedAt": run.updated_at,
        "jobs": run.ordered_jobs().iter().map(|j| json!({
            "name": j.name, "state": run_state(j.state), "url": j.url, "startedAt": j.started_at, "completedAt": j.completed_at,
        })).collect::<Vec<_>>(),
    })
}

/// Whether you have read what a desktop session last said, as the app's sidebar has it; None for a terminal
/// session, or when the app's list could not be read.
fn read_state(live: &Live, a: &AgentInfo) -> Option<bool> {
    let id = a.pane.splitn(3, ':').nth(2)?;
    Some(!live.unread.as_ref()?.contains(id))
}

pub(super) fn agents_view(live: &Live, boards: &[Board], now: DateTime<Utc>) -> Value {
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
                    let (state, status, icon) = if needs_you {
                        ("needs", AgentStatus::Blocked.word(), AgentStatus::Blocked.icon())
                    } else {
                        match (a.status, read_state(live, a)) {
                            (AgentStatus::Working, _) => ("working", a.status.word(), a.status.icon()),
                            (AgentStatus::Done, _) => ("done", a.status.word(), a.status.icon()),
                            (_, Some(false)) => ("unread", "●", "●"),
                            (_, Some(true)) => ("read", "○", "○"),
                            _ => ("idle", a.status.word(), a.status.icon()),
                        }
                    };
                    json!({
                        "pane": a.pane,
                        "label": e.label,
                        "depth": e.depth,
                        "status": status,
                        "icon": icon,
                        "state": state,
                        "topic": a.topic,
                        "source": match crate::claude::entrypoint_of(&a.pane) { "claude-desktop" => "desktop", other => other },
                        "deepLink": crate::claude::desktop_link(&a.pane),
                        "note": note_of(a),
                        "session": a.session,
                        "subagents": a.session.as_ref().and_then(|s| live.subagents.get(s)).map(|list| list.iter().map(|sa| json!({
                            "id": sa.id, "kind": sa.kind, "description": sa.description, "running": sa.running, "depth": sa.depth,
                            "updatedAt": DateTime::<Utc>::from_timestamp(sa.updated as i64, 0).map(|t| t.to_rfc3339()),
                        })).collect::<Vec<_>>()).unwrap_or_default(),
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

/// Every link the page offers: what `/open` is willing to open. Web links only from the fields that
/// hold links (so a link inside a PR's title is not on the list), and the desktop app's own links.
pub(super) fn shown_urls(value: &Value, out: &mut std::collections::HashSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                match v {
                    Value::String(u) if (key == "url" || key == "jiraUrl") && u.starts_with("https://") => {
                        out.insert(u.clone());
                        if map.contains_key("number") {
                            out.insert(format!("{u}/files"));
                        }
                    }
                    Value::String(u) if u.starts_with("claude://code/") || u.starts_with("claude://resume?") => {
                        out.insert(u.clone());
                    }
                    other => shown_urls(other, out),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| shown_urls(v, out)),
        _ => {}
    }
}

impl Server {
    /// Everything the page draws.
    pub(super) fn state_json(&self, now: DateTime<Utc>) -> Value {
        let mut boards = Vec::new();
        let mut broken = Vec::new();
        for key in store::list_keys(&self.root).unwrap_or_default() {
            match store::load(&self.root, &key) {
                Ok(board) => boards.push(board),
                Err(e) => broken.push(json!({ "key": key, "error": format!("{e:#}") })),
            }
        }
        let settings = self.settings();
        let workdir = lock(&self.eng.workdir).clone();
        let sessions = lock(&self.eng.sessions).clone();
        let durations = lock(&self.eng.durations);
        let data = lock(&self.prs);
        let live = lock(&self.live);
        let runs = lock(&self.eng.runs);
        let events = lock(&self.eng.events).clone();
        let known = lock(&self.eng.known).clone();
        let cx = Ctx { boards: &boards, settings: &settings, durations: &durations, now, sessions: &sessions, agents: &live.agents, known: &known, open: &data.items, workdir: &workdir };
        let stories: Vec<Value> = boards
            .iter()
            .map(|b| {
                let key = &b.story.key;
                json!({
                    "key": key,
                    "title": b.story.title,
                    "status": b.story.status.as_str(),
                    "jiraUrl": b.story.jira_url,
                    "tasks": b.tasks.iter().map(|t| task_view(b, t, &cx)).collect::<Vec<_>>(),
                    "allFinished": !b.tasks.is_empty() && b.tasks.iter().all(|t| t.status.is_terminal()),
                    "breakdown": new_session_link(&format!("/story-break-down {key}"), &workdir),
                    "review": new_session_link(&format!("/story-review {key}"), &workdir),
                })
            })
            .collect();
        let mut order: Vec<&PullRequest> = data.items.iter().collect();
        order.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        let red = data.items.iter().filter(|p| p.is_red()).count();
        let waiting = data.items.iter().filter(|p| p.waiting_for() == Some(Waiting::Reviewer)).count();
        let fixing = data.items.iter().filter(|p| p.is_red() && autofix_on(&settings, p)).count();
        json!({
            "now": now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "stories": stories,
            "broken": broken,
            "prs": order.iter().map(|p| pr_view(p, &cx)).collect::<Vec<_>>(),
            "prError": data.error,
            "prUpdated": data.updated.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            "loading": data.loading,
            "red": red,
            "redWithAutofix": fixing,
            "awaitingReview": waiting,
            "agents": agents_view(&live, &boards, now),
            "runs": {
                "on": *lock(&self.eng.runs_on),
                "items": runs.items.iter().map(run_view).collect::<Vec<_>>(),
                "error": runs.error,
                "loading": runs.loading,
                "updated": runs.updated.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            },
            "events": events.iter().rev().map(|e| json!({ "at": e.at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true), "text": e.text })).collect::<Vec<_>>(),
            "settings": {
                "autofix": settings.autofix,
                "autoreview": settings.autocopilot,
                "autoSync": settings.auto_sync,
                "notify": settings.notify,
                "slowWhenAway": settings.slow_when_away,
            },
            "workdir": workdir.to_string_lossy(),
            "usage": lock(&self.eng.usage_path).as_deref().and_then(crate::usage::read).and_then(|u| u.current(now)).map(|u| {
                let win = |w: &Option<crate::usage::Window>| w.as_ref().map(|w| json!({
                    "pct": w.used_percent,
                    "resetsAt": w.resets_at.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
                }));
                json!({
                    "at": u.at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    "fiveHour": win(&u.five_hour),
                    "sevenDay": win(&u.seven_day),
                })
            }),
        })
    }
}
