//! The Claude Code sessions running now, from the files Claude Code keeps for each live process in
//! `~/.claude/sessions/<pid>.json` (the terminal program and the desktop app both write one).
//! That file format belongs to Claude Code and is not documented, so everything here is lenient:
//! a file that does not look right is skipped, and a status word it does not know is shown as is.
use crate::agents::{AgentInfo, Status};
use serde_json::Value;
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
}
