//! The Herdr desktop app (`herdr-app`, with `hc` for its terminals): start, list, focus and close Claude
//! agents. Every call to it goes through here, so a change to its commands is one place to fix.
//! This is a different program from the `herdr` command-line tool, with its own spaces and tabs.
use crate::agents::{AgentInfo, Status};
use serde_json::Value;
use std::collections::HashMap;
use std::process::{Command, Stdio};

/// Runs `program` (`herdr-app` or `hc`) with the arguments and returns the JSON it printed.
pub type Runner = Box<dyn Fn(&str, &[String]) -> Result<Value, String> + Send + Sync>;

const PREFIX: &str = "herdr-app:";

/// An agent's pane id in capcom: `herdr-app:<terminal>`.
pub fn pane_of(terminal: &str) -> String {
    format!("{PREFIX}{terminal}")
}

pub fn terminal_of(pane: &str) -> Option<&str> {
    pane.strip_prefix(PREFIX).filter(|t| !t.is_empty())
}

fn run_program(program: &str, args: &[String]) -> Result<Value, String> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null());
    let out = match crate::refresh::run_with_timeout(cmd, std::time::Duration::from_secs(30)) {
        Ok(Some(out)) => out,
        Ok(None) => return Err(format!("{program} timed out")),
        Err(e) => return Err(format!("could not run {program}: {e}")),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str::<Value>(&text).map_err(|_| {
        let err = String::from_utf8_lossy(&out.stderr);
        err.lines().chain(text.lines()).find(|l| !l.trim().is_empty()).unwrap_or("no answer").trim().to_string()
    })
}

/// Whether the app has a window open to ask (`herdr-app status` answers).
pub fn answers() -> bool {
    run_program("herdr-app", &["status".to_string()]).is_ok_and(|v| v.get("error").is_none())
}

fn status_of(state: Option<&str>) -> Status {
    match state {
        Some("working" | "busy" | "running") => Status::Working,
        Some("idle") => Status::Idle,
        Some("done") => Status::Done,
        Some(s) if s.contains("wait") || s.contains("block") || s.contains("permission") || s.contains("input") => Status::Blocked,
        _ => Status::Unknown,
    }
}

/// A terminal title without its leading spinner or mark (`✳ Claude Code` is `Claude Code`).
fn topic_of(title: &str) -> String {
    let title = title.trim();
    match title.split_once(' ') {
        Some((mark, rest)) if mark.chars().all(|c| !c.is_alphanumeric()) => rest.trim().to_string(),
        _ => title.to_string(),
    }
}

/// Where a new agent goes: a tab of the story's space (made when the story has none), or of the space
/// shown when `space` is empty.
pub struct Start<'a> {
    /// The story key that names its space; empty to use the space shown.
    pub space: &'a str,
    /// What a new (or renamed) space is called, such as `PROJ-123 - Notification preferences`.
    pub label: &'a str,
    pub tab: &'a str,
    pub folder: &'a str,
    /// The command, such as `["claude"]` or `["claude", "--resume", id]`.
    pub argv: Vec<String>,
    /// The first message, which Claude is started with.
    pub prompt: Option<&'a str>,
    /// Leave the shown tab as it was, instead of switching to the new one.
    pub focus: bool,
}

pub struct HerdrApp {
    run: Runner,
}

impl HerdrApp {
    pub fn new() -> HerdrApp {
        HerdrApp { run: Box::new(run_program) }
    }

    pub fn with_runner(run: Runner) -> HerdrApp {
        HerdrApp { run }
    }

    fn call(&self, program: &str, args: &[&str]) -> Result<Value, String> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let v = (self.run)(program, &args).map_err(|e| format!("Herdr app: {e}"))?;
        match v.get("error") {
            Some(e) => Err(format!("Herdr app: {}", e.as_str().or(e["message"].as_str()).unwrap_or("it said no"))),
            None => Ok(v),
        }
    }

    fn app(&self, args: &[&str]) -> Result<Value, String> {
        self.call("herdr-app", args)
    }

    fn act(&self, command: &str, args: &[&str]) -> Result<Value, String> {
        let all: Vec<&str> = ["do", command].into_iter().chain(args.iter().copied()).collect();
        self.app(&all)
    }

    /// The space for a story, by its key: named just `KEY` or `KEY - some words`. Returns its id and name.
    fn find_space(&self, key: &str) -> Result<Option<(String, String)>, String> {
        let listed = self.app(&["ls"])?;
        let described = format!("{key} - ");
        Ok(listed["spaces"].as_array().into_iter().flatten().find_map(|s| {
            let name = s["name"].as_str()?;
            (name == key || name.starts_with(&described)).then(|| Some((s["id"].as_str()?.to_string(), name.to_string()))).flatten()
        }))
    }

    /// The space and tab shown now, to put back after starting something without taking the screen.
    fn shown(&self) -> Option<(String, String)> {
        let status = self.app(&["status"]).ok()?;
        Some((status["space"].as_str()?.to_string(), status["tab"].as_str()?.to_string()))
    }

    fn show(&self, (space, tab): &(String, String)) {
        let _ = self.act("space.select", &["--id", space]);
        let _ = self.act("tab.select", &["--id", tab]);
    }

    /// Start an agent; returns its terminal.
    pub fn start(&self, s: &Start) -> Result<String, String> {
        let before = if s.focus { None } else { self.shown() };
        let argv = serde_json::to_string(&s.argv).map_err(|e| e.to_string())?;
        let mut args: Vec<&str> = vec!["--folder", s.folder, "--argv", &argv];
        if let Some(p) = s.prompt {
            args.extend(["--task", p]);
        }
        let found = if s.space.is_empty() { None } else { self.find_space(s.space)? };
        let made = match &found {
            Some((id, name)) => {
                if s.label != s.space && name != s.label {
                    let _ = self.act("space.rename", &["--id", id, "--name", s.label]);
                }
                args.extend(["--space", id, "--name", s.tab]);
                self.act("tab.new", &args)?
            }
            None if s.space.is_empty() => {
                args.extend(["--name", s.tab]);
                self.act("tab.new", &args)?
            }
            None => {
                args.extend(["--name", s.label, "--tab", s.tab]);
                self.act("space.new", &args)?
            }
        };
        if let Some(b) = &before {
            self.show(b);
        }
        made["terminal"].as_str().map(str::to_string).ok_or_else(|| "Herdr app: no terminal in its answer".to_string())
    }

    /// The Claude agents in terminals, with the session each runs when `sessions` knows its process.
    pub fn agents(&self, sessions: &HashMap<u64, String>) -> Result<Vec<AgentInfo>, String> {
        let listed = self.app(&["ls"])?;
        let mut found = Vec::new();
        for space in listed["spaces"].as_array().into_iter().flatten() {
            for tab in space["tabs"].as_array().into_iter().flatten() {
                for t in tab["terminals"].as_array().into_iter().flatten().filter(|t| t["app"] == "claude-code") {
                    let Some(terminal) = t["terminal"].as_str() else { continue };
                    let Ok(got) = self.call("hc", &["get", terminal]) else { continue };
                    let pid = got["foreground"]["pid"].as_u64();
                    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
                    found.push(AgentInfo {
                        pane: pane_of(terminal),
                        name: t["name"].as_str().filter(|n| !n.is_empty()).map(str::to_string),
                        status: status_of(t["status"].as_str()),
                        topic: topic_of(got["title"].as_str().unwrap_or_default()),
                        workspace: text(&space["name"]),
                        cwd: text(&got["cwd"]),
                        session: pid.and_then(|p| sessions.get(&p).cloned()),
                    });
                }
            }
        }
        Ok(found)
    }

    pub fn focus(&self, terminal: &str) -> Result<(), String> {
        self.act("agent.focus", &["--terminal", terminal]).map(|_| ())
    }

    pub fn send(&self, terminal: &str, text: &str) -> Result<(), String> {
        self.call("hc", &["send", terminal, text]).map(|_| ())
    }

    pub fn close(&self, terminal: &str) -> Result<(), String> {
        self.act("terminal.kill", &["--terminal", terminal]).map(|_| ())
    }
}

impl Default for HerdrApp {
    fn default() -> HerdrApp {
        HerdrApp::new()
    }
}

/// Live Claude sessions by the process running them, from each `cli:<pid>` session.
pub fn sessions_by_pid(live: &[AgentInfo]) -> HashMap<u64, String> {
    live.iter().filter_map(|a| Some((a.pane.strip_prefix("cli:")?.parse().ok()?, a.session.clone()?))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    type Calls = Arc<Mutex<Vec<Vec<String>>>>;

    fn ls() -> Value {
        json!({"spaces": [
            {"id": "sp1", "name": "PROJ-1 - Notices", "tabs": [
                {"id": "tb1", "name": "1", "terminals": [
                    {"terminal": "t1", "status": "working", "app": "claude-code", "name": null},
                    {"terminal": "t2", "status": null, "app": null, "name": null},
                ]},
            ]},
            {"id": "sp2", "name": "other", "tabs": [{"id": "tb2", "name": "1", "terminals": [{"terminal": "t3", "status": "idle", "app": "claude-code", "name": null}]}]},
        ]})
    }

    fn app(fail: Option<&'static str>) -> (HerdrApp, Calls) {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let log = calls.clone();
        let run: Runner = Box::new(move |program, args| {
            let mut line = vec![program.to_string()];
            line.extend(args.iter().cloned());
            log.lock().unwrap().push(line);
            if let Some(e) = fail {
                return Err(e.to_string());
            }
            let a: Vec<&str> = args.iter().map(String::as_str).collect();
            Ok(match (program, a.as_slice()) {
                ("herdr-app", ["ls"]) => ls(),
                ("herdr-app", ["status"]) => json!({"space": "sp2", "tab": "tb2"}),
                ("herdr-app", ["do", "tab.new" | "space.new", ..]) => json!({"space": "sp1", "tab": "tbN", "terminal": "t9"}),
                ("hc", ["get", "t1"]) => json!({"cwd": "/w/app", "title": "✳ Implement T2", "foreground": {"pid": 11}}),
                ("hc", ["get", "t3"]) => json!({"cwd": "/w/x", "title": "Claude Code", "foreground": {"pid": 12}}),
                _ => json!({"ok": true}),
            })
        });
        (HerdrApp::with_runner(run), calls)
    }

    fn calls_of(calls: &Calls) -> Vec<String> {
        calls.lock().unwrap().iter().map(|c| c.join(" ")).collect()
    }

    fn start<'a>(space: &'a str, label: &'a str, focus: bool) -> Start<'a> {
        Start { space, label, tab: "t2 impl", folder: "/w/app", argv: vec!["claude".into()], prompt: Some("/story-implement-task PROJ-1 T2"), focus }
    }

    #[test]
    fn panes_carry_the_apps_name() {
        assert_eq!(pane_of("t8"), "herdr-app:t8");
        assert_eq!(terminal_of("herdr-app:t8"), Some("t8"));
        assert_eq!(terminal_of("orca:t8"), None);
        assert_eq!(terminal_of("herdr-app:"), None);
    }

    #[test]
    fn titles_lose_their_spinner() {
        assert_eq!(topic_of("✳ Claude Code"), "Claude Code");
        assert_eq!(topic_of("Claude Code"), "Claude Code");
        assert_eq!(topic_of(""), "");
    }

    #[test]
    fn a_story_with_a_space_gets_a_tab_in_it_and_the_shown_tab_is_put_back() {
        let (app, calls) = app(None);
        assert_eq!(app.start(&start("PROJ-1", "PROJ-1 - Notices", false)).unwrap(), "t9");
        let all = calls_of(&calls);
        assert!(all.iter().any(|c| c.starts_with("herdr-app do tab.new --folder /w/app --argv [\"claude\"] --task /story-implement-task PROJ-1 T2 --space sp1 --name t2 impl")), "{all:?}");
        assert!(!all.iter().any(|c| c.contains("space.rename") || c.contains("space.new")), "{all:?}");
        assert!(all.contains(&"herdr-app do space.select --id sp2".to_string()), "{all:?}");
        assert!(all.contains(&"herdr-app do tab.select --id tb2".to_string()), "{all:?}");
    }

    #[test]
    fn a_story_without_a_space_gets_one_named_after_it_with_its_first_tab() {
        let (app, calls) = app(None);
        app.start(&start("PROJ-2", "PROJ-2 - Other", true)).unwrap();
        let all = calls_of(&calls);
        assert!(all.iter().any(|c| c.starts_with("herdr-app do space.new") && c.ends_with("--name PROJ-2 - Other --tab t2 impl")), "{all:?}");
        assert!(!all.iter().any(|c| c.contains("tab.select")), "focus was asked for: {all:?}");
    }

    #[test]
    fn a_space_named_only_by_the_key_is_renamed_to_the_descriptive_name() {
        let (app, calls) = app(None);
        let found = app.find_space("other").unwrap();
        assert_eq!(found, Some(("sp2".to_string(), "other".to_string())));
        app.start(&start("other", "other - words", true)).unwrap();
        assert!(calls_of(&calls).contains(&"herdr-app do space.rename --id sp2 --name other - words".to_string()));
    }

    #[test]
    fn another_story_is_not_confused_with_this_one() {
        let (app, _) = app(None);
        assert_eq!(app.find_space("PROJ-10").unwrap(), None);
        assert_eq!(app.find_space("PROJ-1").unwrap().map(|s| s.0), Some("sp1".to_string()));
    }

    #[test]
    fn with_no_story_the_tab_opens_in_the_space_shown() {
        let (app, calls) = app(None);
        app.start(&start("", "", true)).unwrap();
        let all = calls_of(&calls);
        assert!(all.iter().any(|c| c.starts_with("herdr-app do tab.new") && !c.contains("--space")), "{all:?}");
    }

    #[test]
    fn claude_agents_are_listed_with_status_title_folder_and_session() {
        let (app, _) = app(None);
        let sessions = sessions_by_pid(&[AgentInfo { pane: "cli:11".into(), name: None, status: Status::Idle, topic: String::new(), workspace: String::new(), cwd: String::new(), session: Some("s-1".into()) }]);
        let agents = app.agents(&sessions).unwrap();
        assert_eq!(agents.len(), 2, "the plain shell is not an agent");
        assert_eq!((agents[0].pane.as_str(), agents[0].status, agents[0].topic.as_str()), ("herdr-app:t1", Status::Working, "Implement T2"));
        assert_eq!((agents[0].workspace.as_str(), agents[0].cwd.as_str(), agents[0].session.as_deref()), ("PROJ-1 - Notices", "/w/app", Some("s-1")));
        assert_eq!((agents[1].status, agents[1].session.clone()), (Status::Idle, None));
    }

    #[test]
    fn focus_send_and_close_name_the_terminal() {
        let (app, calls) = app(None);
        app.focus("t1").unwrap();
        app.send("t1", "hello").unwrap();
        app.close("t1").unwrap();
        assert_eq!(calls_of(&calls), ["herdr-app do agent.focus --terminal t1", "hc send t1 hello", "herdr-app do terminal.kill --terminal t1"]);
    }

    #[test]
    fn an_error_from_the_app_is_reported_as_coming_from_it() {
        let (app, _) = app(Some("no window is open"));
        assert_eq!(app.focus("t1").unwrap_err(), "Herdr app: no window is open");
        let app = HerdrApp::with_runner(Box::new(|_, _| Ok(json!({"error": "unknown command"}))));
        assert_eq!(app.focus("t1").unwrap_err(), "Herdr app: unknown command");
    }
}
