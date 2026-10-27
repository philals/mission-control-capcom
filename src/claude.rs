//! The Claude Code sessions running now, from the files Claude Code keeps for each live process in
//! `~/.claude/sessions/<pid>.json` (the terminal program and the desktop app both write one).
//! That file format belongs to Claude Code and is not documented, so everything here is lenient:
//! a file that does not look right is skipped, and a status word it does not know is shown as is.
use crate::agents::{AgentInfo, Status};
use serde_json::{json, Value};
use std::path::Path;

/// `cli:1234` or `claude-desktop:1234:local_…`: who runs the session, its process and, for the desktop
/// app, the id the app knows the session by. Opaque to everything else.
pub fn pane_of(entrypoint: &str, pid: u64, host: Option<&str>) -> String {
    match host {
        Some(host) => format!("{entrypoint}:{pid}:{host}"),
        None => format!("{entrypoint}:{pid}"),
    }
}

/// The link that has the desktop app import a terminal session and open it (`claude://resume?session=…`);
/// None unless the id looks like one (letters, digits and hyphens).
pub fn import_link(session: &str) -> Option<String> {
    let valid = !session.is_empty() && session.len() <= 64 && session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    valid.then(|| format!("claude://resume?session={session}"))
}

/// The link that opens a desktop session in the Claude Code desktop app (`claude://code/continue?session=…`),
/// from the id in its pane id; None for a session the desktop app does not know. The app accepts only
/// ids of the form `local_` and letters, digits and hyphens.
pub fn desktop_link(pane: &str) -> Option<String> {
    let host = pane.splitn(3, ':').nth(2)?;
    let valid = host.strip_prefix("local_").is_some_and(|rest| !rest.is_empty() && rest.len() <= 64 && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    valid.then(|| format!("claude://code/continue?session={host}"))
}

/// The part before the colon of a `pane_of` id: `cli`, `claude-desktop`.
pub fn entrypoint_of(pane: &str) -> &str {
    pane.split(':').next().unwrap_or_default()
}

/// Whether the process in a session file is still the one that wrote it (the id may have been reused).
fn alive(pid: u64, proc_start: Option<&str>) -> bool {
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        // field 22 is the start time; the command name before it may hold spaces, so count after the last ')'
        let started = stat.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().nth(19));
        return match (started, proc_start) {
            (Some(now), Some(then)) => now == then,
            _ => true,
        };
    }
    // no /proc (macOS): ask ps
    std::process::Command::new("ps").args(["-p", &pid.to_string(), "-o", "pid="]).output().is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
}

fn status_of(word: &str) -> Status {
    match word {
        "busy" | "working" | "running" => Status::Working,
        "idle" => Status::Idle,
        w if ["wait", "block", "permission", "input", "approval"].iter().any(|k| w.contains(k)) => Status::Blocked,
        _ => Status::Unknown,
    }
}

fn folder_name(cwd: &str) -> String {
    Path::new(cwd).file_name().map_or_else(|| cwd.to_string(), |n| n.to_string_lossy().to_string())
}

/// The live sessions, in the order of their start.
pub fn live(config: &Path) -> Vec<AgentInfo> {
    let mut found: Vec<(u64, AgentInfo)> = std::fs::read_dir(config.join("sessions"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let v: Value = serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()?;
            let pid = v["pid"].as_u64()?;
            let session = v["sessionId"].as_str()?.to_string();
            let proc_start = v["procStart"].as_str().map(str::to_string).or_else(|| v["procStart"].as_u64().map(|n| n.to_string()));
            if !alive(pid, proc_start.as_deref()) {
                return None;
            }
            let cwd = v["cwd"].as_str().unwrap_or_default().to_string();
            let name = v["name"].as_str().filter(|n| !n.is_empty()).map(str::to_string);
            Some((
                v["startedAt"].as_u64().unwrap_or(0),
                AgentInfo {
                    pane: pane_of(v["entrypoint"].as_str().unwrap_or("cli"), pid, v["hostSessionId"].as_str()),
                    name,
                    status: status_of(v["status"].as_str().unwrap_or_default()),
                    topic: String::new(),
                    workspace: folder_name(&cwd),
                    cwd,
                    session: Some(session),
                },
            ))
        })
        .collect();
    found.sort_by_key(|(started, _)| *started);
    found.into_iter().map(|(_, a)| a).collect()
}

/// A subagent a session started with its Agent tool, from `projects/<folder>/<session>/subagents/agent-<id>.meta.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subagent {
    pub id: String,
    pub kind: String,
    pub description: String,
    pub running: bool,
    /// When its transcript last changed, in seconds since 1970.
    pub updated: u64,
    pub depth: u64,
}

/// A subagent not marked finished that has written nothing for this long has stopped (the parent was
/// interrupted, or an older Claude Code that never marks them finished).
const SUBAGENT_QUIET: u64 = 10 * 60;
/// Finished subagents stay listed this long, at most this many.
const SUBAGENT_KEPT: u64 = 30 * 60;
const SUBAGENTS_MAX: usize = 8;

fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn subagent_dirs(config: &Path, session: &str) -> Vec<std::path::PathBuf> {
    if !safe_id(session) {
        return Vec::new();
    }
    std::fs::read_dir(config.join("projects")).into_iter().flatten().flatten().map(|e| e.path().join(session).join("subagents")).filter(|d| d.is_dir()).collect()
}

fn modified(path: &Path) -> u64 {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

/// The subagents of a session that are running or finished lately, newest first.
pub fn subagents(config: &Path, session: &str, now: u64) -> Vec<Subagent> {
    let mut found: Vec<Subagent> = subagent_dirs(config, session)
        .iter()
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let id = name.strip_prefix("agent-")?.strip_suffix(".meta.json")?.to_string();
            if !safe_id(&id) {
                return None;
            }
            let meta: Value = serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()?;
            let updated = modified(&e.path().with_file_name(format!("agent-{id}.jsonl"))).max(modified(&e.path()));
            let running = !meta["finished"].as_bool().unwrap_or(false) && now.saturating_sub(updated) < SUBAGENT_QUIET;
            let text = |k: &str| meta[k].as_str().unwrap_or_default().to_string();
            Some(Subagent { id, kind: text("agentType"), description: text("description"), running, updated, depth: meta["spawnDepth"].as_u64().unwrap_or(1).max(1) })
        })
        .filter(|s| s.running || now.saturating_sub(s.updated) < SUBAGENT_KEPT)
        .collect();
    found.sort_by_key(|s| (!s.running, std::cmp::Reverse(s.updated)));
    found.truncate(SUBAGENTS_MAX);
    found
}

const TRANSCRIPT_MAX: usize = 300;
const TOOL_TEXT_MAX: usize = 600;

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// What a tool was asked, in a line: its most telling input.
fn tool_line(name: &str, input: &Value) -> String {
    let pick = ["description", "command", "file_path", "pattern", "query", "url", "prompt"].iter().find_map(|k| input[*k].as_str());
    match pick {
        Some(p) => format!("{name} · {}", clip(p, 200)),
        None => name.to_string(),
    }
}

/// Text without the `<system-reminder>` blocks Claude Code adds to a conversation.
fn without_reminders(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("<system-reminder>") {
        out.push_str(&rest[..at]);
        rest = rest[at..].find("</system-reminder>").map_or("", |end| &rest[at + end + "</system-reminder>".len()..]);
    }
    out.push_str(rest);
    out.trim().to_string()
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// A subagent's conversation, simplified for reading: `{kind, text}` with kind `prompt`, `text`,
/// `tool` or `result`; the last few hundred of them. None when there is no such subagent.
pub fn subagent_transcript(config: &Path, session: &str, agent: &str) -> Option<Vec<Value>> {
    if !safe_id(agent) {
        return None;
    }
    let file = subagent_dirs(config, session).into_iter().map(|d| d.join(format!("agent-{agent}.jsonl"))).find(|f| f.is_file())?;
    let raw = std::fs::read_to_string(file).ok()?;
    let mut out: Vec<Value> = Vec::new();
    let said = |out: &mut Vec<Value>, kind: &str, text: &str| {
        let text = without_reminders(text);
        if !text.is_empty() {
            let kind = if kind == "text" && out.is_empty() { "prompt" } else { kind };
            out.push(json!({ "kind": kind, "text": text }));
        }
    };
    for line in raw.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let content = &v["message"]["content"];
        match (v["type"].as_str(), content) {
            (Some("user"), Value::String(text)) => said(&mut out, "text", text),
            (Some("user" | "assistant"), Value::Array(parts)) => {
                for part in parts {
                    match part["type"].as_str() {
                        Some("text") => said(&mut out, "text", part["text"].as_str().unwrap_or_default()),
                        Some("tool_use") if part["name"] == "SubagentHandback" => said(&mut out, "report", part["input"]["message"].as_str().unwrap_or_default()),
                        Some("tool_use") => out.push(json!({ "kind": "tool", "text": tool_line(part["name"].as_str().unwrap_or("tool"), &part["input"]) })),
                        Some("tool_result") => out.push(json!({ "kind": "result", "text": clip(&result_text(&part["content"]), TOOL_TEXT_MAX) })),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let skip = out.len().saturating_sub(TRANSCRIPT_MAX);
    Some(out.split_off(skip))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn own_start() -> String {
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
        stat.rsplit_once(')').unwrap().1.split_whitespace().nth(19).unwrap().to_string()
    }

    fn write(dir: &Path, name: &str, value: Value) {
        std::fs::create_dir_all(dir.join("sessions")).unwrap();
        std::fs::write(dir.join("sessions").join(name), serde_json::to_vec(&value).unwrap()).unwrap();
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn only_processes_that_are_still_running_are_listed_with_their_status_and_who_runs_them() {
        let dir = tempfile::tempdir().unwrap();
        let me = std::process::id() as u64;
        write(dir.path(), "a.json", json!({ "pid": me, "sessionId": "s-1", "cwd": "/w/capcom", "startedAt": 20, "procStart": own_start(), "entrypoint": "claude-desktop", "name": "capcom work", "status": "busy" }));
        write(dir.path(), "b.json", json!({ "pid": me, "sessionId": "s-2", "cwd": "/w/api", "startedAt": 10, "procStart": own_start(), "entrypoint": "cli", "status": "idle" }));
        write(dir.path(), "dead.json", json!({ "pid": 4_000_000_000u64, "sessionId": "s-3", "cwd": "/w/x", "startedAt": 1, "status": "idle" }));
        write(dir.path(), "reused.json", json!({ "pid": me, "sessionId": "s-4", "cwd": "/w/y", "startedAt": 2, "procStart": "1", "status": "idle" }));
        write(dir.path(), "odd.json", json!({ "hello": "world" }));
        std::fs::write(dir.path().join("sessions").join("broken.json"), "{ not json").unwrap();
        std::fs::write(dir.path().join("sessions").join("x.key"), "secret").unwrap();
        let agents = live(dir.path());
        let sessions: Vec<&str> = agents.iter().filter_map(|a| a.session.as_deref()).collect();
        assert_eq!(sessions, vec!["s-2", "s-1"], "oldest first; the dead, reused-id and odd files are left out");
        assert_eq!(agents[0].status, Status::Idle);
        assert_eq!(agents[1].status, Status::Working);
        assert_eq!(agents[1].workspace, "capcom");
        assert_eq!(agents[1].name.as_deref(), Some("capcom work"));
        assert_eq!(entrypoint_of(&agents[1].pane), "claude-desktop");
        assert_eq!(entrypoint_of(&agents[0].pane), "cli");
    }

    #[test]
    fn a_desktop_session_has_a_link_that_opens_it_and_nothing_else_does() {
        assert_eq!(desktop_link(&pane_of("claude-desktop", 7, Some("local_f54e7d98-8fe4-40e1"))).as_deref(), Some("claude://code/continue?session=local_f54e7d98-8fe4-40e1"));
        assert_eq!(desktop_link(&pane_of("cli", 7, None)), None, "a terminal session is not in the app");
        assert_eq!(desktop_link(&pane_of("claude-desktop", 7, None)), None);
        for bad in ["local_", "local_a b", "local_a&x=1", "local_../x", "session_abc", "local_é"] {
            assert_eq!(desktop_link(&pane_of("claude-desktop", 7, Some(bad))), None, "{bad}");
        }
        assert_eq!(entrypoint_of(&pane_of("claude-desktop", 7, Some("local_a"))), "claude-desktop");
    }

    #[test]
    fn a_terminal_session_can_be_imported_by_its_id() {
        assert_eq!(import_link("6bd808b2-6f76-440e-b1b8-220d83f4a05c").as_deref(), Some("claude://resume?session=6bd808b2-6f76-440e-b1b8-220d83f4a05c"));
        for bad in ["", "a b", "a&b=1", "../x", "é"] {
            assert_eq!(import_link(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn status_words_are_read_leniently() {
        assert_eq!(status_of("busy"), Status::Working);
        assert_eq!(status_of("idle"), Status::Idle);
        assert_eq!(status_of("waiting_for_permission"), Status::Blocked);
        assert_eq!(status_of("something new"), Status::Unknown);
    }

    #[test]
    fn a_missing_folder_is_no_sessions() {
        assert!(live(Path::new("/nonexistent/claude")).is_empty());
    }

    fn subagent(config: &Path, session: &str, id: &str, meta: Value, lines: &[Value]) {
        let dir = config.join("projects").join("-w-capcom").join(session).join("subagents");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("agent-{id}.meta.json")), serde_json::to_vec(&meta).unwrap()).unwrap();
        std::fs::write(dir.join(format!("agent-{id}.jsonl")), lines.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
    }

    #[test]
    fn a_sessions_subagents_are_listed_running_first_and_old_finished_ones_are_left_out() {
        let dir = tempfile::tempdir().unwrap();
        subagent(dir.path(), "s-1", "a1", json!({ "agentType": "Explore", "description": "Scan things", "finished": true, "spawnDepth": 1 }), &[]);
        subagent(dir.path(), "s-1", "a2", json!({ "agentType": "general-purpose", "description": "Fix it" }), &[]);
        subagent(dir.path(), "s-2", "a3", json!({ "agentType": "Explore" }), &[]);
        let now = modified(&dir.path().join("projects/-w-capcom/s-1/subagents/agent-a1.jsonl"));
        let list = subagents(dir.path(), "s-1", now);
        assert_eq!(list.iter().map(|s| (s.id.as_str(), s.running)).collect::<Vec<_>>(), vec![("a2", true), ("a1", false)]);
        assert_eq!(list[1].kind, "Explore");
        assert_eq!(list[1].description, "Scan things");
        assert!(subagents(dir.path(), "s-1", now + SUBAGENT_KEPT + SUBAGENT_QUIET).is_empty(), "long quiet: stopped and too old to list");
        assert!(subagents(dir.path(), "../s-1", now).is_empty());
    }

    #[test]
    fn a_subagents_transcript_is_simplified_for_reading() {
        let dir = tempfile::tempdir().unwrap();
        subagent(dir.path(), "s-1", "a1", json!({}), &[
            json!({ "type": "user", "message": { "role": "user", "content": "Find the leak" } }),
            json!({ "type": "assistant", "message": { "role": "assistant", "content": [{ "type": "text", "text": "<system-reminder>\nhush\n</system-reminder>" }, { "type": "text", "text": "Looking." }, { "type": "tool_use", "name": "Bash", "input": { "command": "ls" } }] } }),
            json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "tool_result", "content": [{ "type": "text", "text": "a.rs" }] }] } }),
            json!({ "type": "summary" }),
            json!({ "type": "assistant", "message": { "role": "assistant", "content": [{ "type": "tool_use", "name": "SubagentHandback", "input": { "message": "Found it." } }] } }),
        ]);
        let t = subagent_transcript(dir.path(), "s-1", "a1").unwrap();
        let kinds: Vec<(&str, &str)> = t.iter().map(|e| (e["kind"].as_str().unwrap(), e["text"].as_str().unwrap())).collect();
        assert_eq!(kinds, vec![("prompt", "Find the leak"), ("text", "Looking."), ("tool", "Bash · ls"), ("result", "a.rs"), ("report", "Found it.")]);
        assert!(subagent_transcript(dir.path(), "s-1", "nope").is_none());
        assert!(subagent_transcript(dir.path(), "s-1", "../a1").is_none());
    }
}
