//! Orca's command-line tool: start, resume, list, switch to, type into and close Claude agents.
//! Every call to Orca goes through here, so a change to its CLI is one place to fix.
use crate::agents::{AgentInfo, Status};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub type Runner = Box<dyn Fn(&[String]) -> Result<Value, String> + Send + Sync>;

/// `$ORCA_CLI_BIN_DIR/orca`, then the shim Orca keeps in its config folder, then `orca-ide` on PATH.
/// Never a bare `orca`: on Linux that is the GNOME screen reader.
pub fn find_cli(var: impl Fn(&str) -> Option<String>, home: &Path, on_path: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    if let Some(dir) = var("ORCA_CLI_BIN_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir).join("orca"));
    }
    let shim = home.join(".config/orca/linux-orca-cli-shim/orca");
    if shim.is_file() {
        return Some(shim);
    }
    on_path("orca-ide")
}

/// `find_cli` for this process: its environment, its home folder and its PATH.
pub fn find_cli_here() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let on_path = |name: &str| std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file());
    find_cli(|k| std::env::var(k).ok(), &home, on_path)
}

/// A task started in Orca needs a folder that exists: Orca has no "somewhere sensible" to fall back on.
pub fn check_task_folder(key: &str, task: &str, folder: Option<&str>) -> Result<(), String> {
    match folder {
        None => Err(format!("{task} has no folder to start in: set one with `capcom set-folder {key} {task} <folder>`")),
        Some(f) if !Path::new(f).is_dir() => Err(format!("{task}'s folder {f} no longer exists: set another with `capcom set-folder {key} {task} <folder>`")),
        Some(_) => Ok(()),
    }
}

pub fn handle_of(pane: &str) -> Option<&str> {
    pane.strip_prefix("orca:").filter(|h| !h.is_empty())
}

/// A variable from a running process's environment (Linux only; None elsewhere or when unreadable).
pub fn environ(pid: u64, name: &str) -> Option<String> {
    let bytes = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    let prefix = format!("{name}=");
    bytes.split(|b| *b == 0).find_map(|entry| std::str::from_utf8(entry).ok()?.strip_prefix(&prefix).map(str::to_string))
}

/// Live Claude sessions started in Orca terminals, by Orca pane key: from each `cli:<pid>` session's
/// `ORCA_PANE_KEY` (read through `environ`, which gives a process's environment variable).
pub fn sessions_by_pane_key(live: &[AgentInfo], environ: impl Fn(u64, &str) -> Option<String>) -> HashMap<String, String> {
    live.iter()
        .filter_map(|a| {
            let pid = a.pane.strip_prefix("cli:")?.parse().ok()?;
            Some((environ(pid, "ORCA_PANE_KEY")?, a.session.clone()?))
        })
        .collect()
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn same_or_inside(folder: &str, checkout: &str) -> bool {
    let (folder, checkout) = (folder.trim_end_matches('/'), checkout.trim_end_matches('/'));
    folder == checkout || folder.strip_prefix(checkout).is_some_and(|rest| rest.starts_with('/'))
}

fn status_of(state: &str) -> Status {
    match state {
        "working" | "running" | "busy" => Status::Working,
        "done" => Status::Done,
        s if s.contains("wait") || s.contains("block") || s.contains("permission") || s.contains("input") => Status::Blocked,
        _ => Status::Idle,
    }
}

fn run_cli(cli: &Path, args: &[String]) -> Result<Value, String> {
    let out = Command::new(cli).args(args).stdin(Stdio::null()).output().map_err(|e| format!("could not run {}: {e}", cli.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => Ok(v),
        Err(_) => {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(err.lines().chain(text.lines()).find(|l| !l.trim().is_empty()).unwrap_or("no answer").trim().to_string())
        }
    }
}

pub struct Orca {
    run: Runner,
}

impl Orca {
    pub fn new(cli: PathBuf) -> Orca {
        Orca { run: Box::new(move |args| run_cli(&cli, args)) }
    }

    pub fn with_runner(run: Runner) -> Orca {
        Orca { run }
    }

    fn call(&self, args: &[&str]) -> Result<Value, String> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).chain(std::iter::once("--json".to_string())).collect();
        let v = (self.run)(&args).map_err(|e| format!("Orca: {e}"))?;
        if v["ok"] == false {
            let message = v["error"]["message"].as_str().or(v["error"].as_str()).unwrap_or("it said no");
            return Err(format!("Orca: {message}"));
        }
        Ok(v)
    }

    /// The Orca worktree that is `folder` or holds it.
    pub fn checkout_of(&self, folder: &str) -> Result<String, String> {
        let list = self.call(&["worktree", "list"])?;
        list["result"]["worktrees"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|w| w["path"].as_str())
            .filter(|p| same_or_inside(folder, p))
            .max_by_key(|p| p.len())
            .map(|p| p.trim_end_matches('/').to_string())
            .ok_or_else(|| format!("Orca does not know {folder}: add it with `orca repo add --path {folder}`"))
    }

    fn create(&self, checkout: &str, command: &str, focus: bool) -> Result<String, String> {
        let worktree = format!("path:{checkout}");
        let mut args = vec!["terminal", "create", "--worktree", &worktree, "--command", command];
        if focus {
            args.push("--focus");
        }
        let made = self.call(&args)?;
        made["result"]["terminal"]["handle"].as_str().map(str::to_string).ok_or_else(|| "Orca: no terminal handle in its answer".to_string())
    }

    /// Start Claude in `folder` with `prompt` sent; returns its terminal handle.
    pub fn launch(&self, folder: &str, prompt: &str, focus: bool) -> Result<String, String> {
        let checkout = self.checkout_of(folder)?;
        if checkout != folder.trim_end_matches('/') {
            return self.create(&checkout, &format!("cd {} && claude {}", quote(folder), quote(prompt)), focus);
        }
        let worktree = format!("path:{checkout}");
        let mut args = vec!["agent", "launch", "--agent", "claude", "--surface", "terminal", "--worktree", &worktree, "--prompt", prompt];
        if focus {
            args.push("--focus");
        }
        let made = self.call(&args)?;
        made["result"]["outcome"]["handle"].as_str().map(str::to_string).ok_or_else(|| "Orca: no agent handle in its answer".to_string())
    }

    /// Resume `session` from the folder it started in, sending `prompt` with it when given.
    pub fn resume(&self, checkout: &str, session_cwd: &str, session: &str, prompt: Option<&str>, focus: bool) -> Result<String, String> {
        let mut command = format!("cd {} && claude --resume {}", quote(session_cwd), quote(session));
        if let Some(p) = prompt {
            command = format!("{command} {}", quote(p));
        }
        self.create(checkout, &command, focus)
    }

    /// The Claude agents in Orca terminals, with the session each runs when `sessions` knows it.
    pub fn agents(&self, sessions: &HashMap<String, String>) -> Result<Vec<AgentInfo>, String> {
        let ps = self.call(&["worktree", "ps"])?;
        let terminals = self.call(&["terminal", "list"])?;
        let handle_for = |key: &str| {
            terminals["result"]["terminals"].as_array().into_iter().flatten().find(|t| t["recordedPaneKey"] == key || t["paneKey"] == key).and_then(|t| t["handle"].as_str())
        };
        let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
        let mut found = Vec::new();
        for w in ps["result"]["worktrees"].as_array().into_iter().flatten() {
            for a in w["agents"].as_array().into_iter().flatten().filter(|a| a["agentType"] == "claude") {
                let key = a["paneKey"].as_str().unwrap_or_default();
                let Some(handle) = handle_for(key) else { continue };
                found.push(AgentInfo {
                    pane: format!("orca:{handle}"),
                    name: a["displayName"].as_str().filter(|n| !n.is_empty()).map(str::to_string),
                    status: status_of(a["state"].as_str().unwrap_or_default()),
                    topic: text(&a["conversationName"]),
                    workspace: text(&w["displayName"]),
                    cwd: text(&w["path"]),
                    session: sessions.get(key).cloned(),
                });
            }
        }
        Ok(found)
    }

    pub fn switch(&self, handle: &str) -> Result<(), String> {
        self.call(&["terminal", "switch", "--terminal", handle]).map(|_| ())
    }

    pub fn send(&self, handle: &str, text: &str) -> Result<(), String> {
        self.call(&["terminal", "send", "--terminal", handle, "--text", text, "--enter"]).map(|_| ())
    }

    pub fn close(&self, handle: &str) -> Result<(), String> {
        self.call(&["terminal", "close", "--terminal", handle]).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    type Calls = Arc<Mutex<Vec<Vec<String>>>>;

    fn worktrees() -> Value {
        json!([
            {"path": "/w/app", "displayName": "main"},
            {"path": "/w/lib", "displayName": "lib-main"},
        ])
    }

    fn ps() -> Value {
        json!({"ok": true, "result": {"worktrees": [
            {"path": "/w/app", "displayName": "main", "agents": [
                {"paneKey": "tab1:leaf1", "state": "working", "agentType": "claude", "conversationName": "Implement T2"},
                {"paneKey": "tab2:leaf2", "state": "done", "agentType": "codex", "conversationName": "codex thing"},
                {"paneKey": "tab3:leaf3", "state": "done", "agentType": "claude", "conversationName": "no terminal any more"},
            ]},
        ]}})
    }

    fn orca(fail: Option<&'static str>) -> (Orca, Calls) {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let log = calls.clone();
        let run: Runner = Box::new(move |args| {
            log.lock().unwrap().push(args.to_vec());
            if let Some(e) = fail {
                return Err(e.to_string());
            }
            let a: Vec<&str> = args.iter().map(String::as_str).collect();
            Ok(match a.as_slice() {
                ["worktree", "list", ..] => json!({"ok": true, "result": {"worktrees": worktrees()}}),
                ["worktree", "ps", ..] => ps(),
                ["terminal", "list", ..] => json!({"ok": true, "result": {"terminals": [
                    {"handle": "term_9", "recordedPaneKey": "tab1:leaf1", "worktreePath": "/w/app"},
                    {"handle": "term_8", "recordedPaneKey": "tab2:leaf2", "worktreePath": "/w/app"},
                ]}}),
                ["agent", "launch", ..] => json!({"ok": true, "result": {"outcome": {"kind": "terminal", "handle": "term_9"}}}),
                ["terminal", "create", ..] => json!({"ok": true, "result": {"terminal": {"handle": "term_7"}}}),
                _ => json!({"ok": true, "result": {}}),
            })
        });
        (Orca::with_runner(run), calls)
    }

    fn last(calls: &Calls) -> Vec<String> {
        calls.lock().unwrap().last().cloned().unwrap_or_default()
    }

    #[test]
    fn the_cli_is_found_from_orca_cli_bin_dir_then_the_shim_never_bare_orca() {
        let home = tempfile::tempdir().unwrap();
        let asked = Mutex::new(Vec::new());
        let on_path = |name: &str| {
            asked.lock().unwrap().push(name.to_string());
            (name == "orca-ide").then(|| PathBuf::from("/usr/bin/orca-ide"))
        };
        let dir = |k: &str| (k == "ORCA_CLI_BIN_DIR").then(|| "/opt/orca/bin".to_string());
        assert_eq!(find_cli(dir, home.path(), on_path), Some(PathBuf::from("/opt/orca/bin/orca")));
        assert_eq!(find_cli(|_| None, home.path(), on_path), Some(PathBuf::from("/usr/bin/orca-ide")));
        let shim = home.path().join(".config/orca/linux-orca-cli-shim");
        std::fs::create_dir_all(&shim).unwrap();
        std::fs::write(shim.join("orca"), "").unwrap();
        assert_eq!(find_cli(|_| None, home.path(), on_path), Some(shim.join("orca")));
        assert!(!asked.lock().unwrap().iter().any(|n| n == "orca"), "never looks up bare orca");
        assert_eq!(find_cli(|_| None, tempfile::tempdir().unwrap().path(), |_| None), None);
    }

    #[test]
    fn launch_asks_for_a_terminal_in_the_folder_and_returns_the_handle() {
        let (o, calls) = orca(None);
        assert_eq!(o.launch("/w/app", "/story-implement-task K-1 T2", false), Ok("term_9".to_string()));
        assert_eq!(
            last(&calls),
            ["agent", "launch", "--agent", "claude", "--surface", "terminal", "--worktree", "path:/w/app", "--prompt", "/story-implement-task K-1 T2", "--json"]
        );
        o.launch("/w/app", "hi", true).unwrap();
        assert!(last(&calls).contains(&"--focus".to_string()));
    }

    #[test]
    fn launch_in_a_folder_inside_a_checkout_starts_claude_there() {
        let (o, calls) = orca(None);
        assert_eq!(o.launch("/w/app/.claude/worktrees/x", "/plan it's", false), Ok("term_7".to_string()));
        assert_eq!(
            last(&calls),
            ["terminal", "create", "--worktree", "path:/w/app", "--command", "cd '/w/app/.claude/worktrees/x' && claude '/plan it'\\''s'", "--json"]
        );
    }

    #[test]
    fn a_folder_orca_does_not_know_says_how_to_add_it() {
        let (o, _) = orca(None);
        let err = o.checkout_of("/w/other").unwrap_err();
        assert!(err.contains("orca repo add --path /w/other"), "{err}");
        assert!(o.launch("/w/other", "hi", false).is_err());
    }

    #[test]
    fn a_checkout_is_matched_on_whole_folder_names() {
        let (o, _) = orca(None);
        assert!(o.checkout_of("/w/application").is_err());
        assert_eq!(o.checkout_of("/w/app/"), Ok("/w/app".to_string()));
    }

    #[test]
    fn a_session_in_a_claude_worktree_resumes_from_the_enclosing_checkout() {
        let (o, calls) = orca(None);
        assert_eq!(o.checkout_of("/w/app/.claude/worktrees/x"), Ok("/w/app".to_string()));
        assert_eq!(o.resume("/w/app", "/w/app/.claude/worktrees/x", "s-1", None, false), Ok("term_7".to_string()));
        assert_eq!(
            last(&calls),
            ["terminal", "create", "--worktree", "path:/w/app", "--command", "cd '/w/app/.claude/worktrees/x' && claude --resume 's-1'", "--json"]
        );
    }

    #[test]
    fn a_resume_can_carry_a_prompt() {
        let (o, calls) = orca(None);
        o.resume("/w/app", "/w/app", "s-1", Some("/pr-address x"), true).unwrap();
        assert_eq!(
            last(&calls),
            ["terminal", "create", "--worktree", "path:/w/app", "--command", "cd '/w/app' && claude --resume 's-1' '/pr-address x'", "--focus", "--json"]
        );
    }

    #[test]
    fn agents_are_claude_rows_of_worktree_ps_with_session_ids_and_handles() {
        let (o, _) = orca(None);
        let sessions = HashMap::from([("tab1:leaf1".to_string(), "654367da-be09-46d0-a58d-b6100bd5b313".to_string())]);
        let agents = o.agents(&sessions).unwrap();
        assert_eq!(agents.len(), 1, "codex rows and rows with no terminal are left out: {agents:?}");
        assert_eq!(agents[0].pane, "orca:term_9");
        assert_eq!(agents[0].session.as_deref(), Some("654367da-be09-46d0-a58d-b6100bd5b313"));
        assert_eq!(agents[0].status, Status::Working);
        assert_eq!(agents[0].topic, "Implement T2");
        assert_eq!(agents[0].workspace, "main");
        assert_eq!(agents[0].cwd, "/w/app");
    }

    #[test]
    fn switch_send_and_close_use_the_handle() {
        let (o, calls) = orca(None);
        o.switch("term_9").unwrap();
        assert_eq!(last(&calls), ["terminal", "switch", "--terminal", "term_9", "--json"]);
        o.send("term_9", "/pr-address x").unwrap();
        assert_eq!(last(&calls), ["terminal", "send", "--terminal", "term_9", "--text", "/pr-address x", "--enter", "--json"]);
        o.close("term_9").unwrap();
        assert_eq!(last(&calls), ["terminal", "close", "--terminal", "term_9", "--json"]);
    }

    #[test]
    fn orca_not_running_is_an_error_naming_orca() {
        let (o, _) = orca(Some("Orca CLI is unavailable; reopen Orca or register the CLI again."));
        let err = o.launch("/w/app", "hi", false).unwrap_err();
        assert!(err.starts_with("Orca: "), "{err}");
        assert!(o.agents(&HashMap::new()).unwrap_err().starts_with("Orca: "));
    }

    #[test]
    fn pane_handles_and_session_pane_keys() {
        assert_eq!(handle_of("orca:term_9"), Some("term_9"));
        assert_eq!(handle_of("cli:7"), None);
        let live = vec![
            AgentInfo { pane: "cli:11".into(), name: None, status: Status::Idle, topic: String::new(), workspace: String::new(), cwd: String::new(), session: Some("s-11".into()) },
            AgentInfo { pane: "cli:12".into(), name: None, status: Status::Idle, topic: String::new(), workspace: String::new(), cwd: String::new(), session: Some("s-12".into()) },
            AgentInfo { pane: "claude-desktop:13:local_a".into(), name: None, status: Status::Idle, topic: String::new(), workspace: String::new(), cwd: String::new(), session: Some("s-13".into()) },
        ];
        let environ = |pid: u64, name: &str| (pid == 11 && name == "ORCA_PANE_KEY").then(|| "tab1:leaf1".to_string());
        assert_eq!(sessions_by_pane_key(&live, environ), HashMap::from([("tab1:leaf1".to_string(), "s-11".to_string())]));
    }
}
