//! Sending a prompt to the agent that made a PR through the Herdr app, without a TUI: the running agent
//! if it is idle, else its session resumed, else a fresh agent in the repo's folder. Nothing here takes
//! the screen.
use crate::agents::Status;
use crate::herdr_app::{self, HerdrApp, Start};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What to send, and who to send it to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub prompt: String,
    /// `(session id, folder it started in)` of the agent that made the PR, when one is recorded.
    pub session: Option<(String, String)>,
    /// The story key that names the Herdr space to open a tab in; empty for the space shown.
    pub space: String,
    /// What a new space is called.
    pub label: String,
    pub tab: String,
    /// The repo's folder name, for a fresh agent when no session is recorded.
    pub repo: String,
    pub workdir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sent {
    /// An agent that was already running was given the prompt.
    Prompted,
    /// A tab was opened for the session (or a fresh agent) with the prompt as its first message.
    Started,
    /// The agent is in the middle of a turn: nothing was sent, try again later.
    Busy,
}

/// `live` is the Claude Code sessions by process, so a running agent can be found by its session.
pub fn send(app: &HerdrApp, live: &HashMap<u64, String>, ask: &Ask) -> Result<Sent, String> {
    if let Some((id, _)) = &ask.session {
        if let Some(a) = app.agents(live)?.iter().find(|a| a.session.as_deref() == Some(id)) {
            if a.status == Status::Working {
                return Ok(Sent::Busy);
            }
            app.send(herdr_app::terminal_of(&a.pane).ok_or("not a Herdr app agent")?, &ask.prompt)?;
            return Ok(Sent::Prompted);
        }
    }
    let (folder, argv) = match &ask.session {
        Some((id, cwd)) if Path::new(cwd).is_dir() => (PathBuf::from(cwd), vec!["claude".to_string(), "--resume".into(), id.clone()]),
        _ => {
            let dir = ask.workdir.join(&ask.repo);
            if ask.repo.is_empty() || !dir.is_dir() {
                return Err(format!("no session is recorded for this PR and {} does not exist to start a new one in", dir.display()));
            }
            (dir, vec!["claude".to_string()])
        }
    };
    let start = Start { space: &ask.space, label: &ask.label, tab: &ask.tab, folder: &folder.to_string_lossy(), argv, prompt: Some(&ask.prompt), focus: false };
    app.start(&start)?;
    Ok(Sent::Started)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    fn app(status: &str, calls: Arc<Mutex<Vec<String>>>) -> HerdrApp {
        let status = status.to_string();
        HerdrApp::with_runner(Box::new(move |program, args| {
            calls.lock().unwrap().push(format!("{program} {}", args.join(" ")));
            Ok(match (program, args.first().map(String::as_str)) {
                ("herdr-app", Some("ls")) => {
                    json!({ "spaces": [{ "name": "S", "id": "sp", "tabs": [{ "terminals": [{ "terminal": "t1", "app": "claude-code", "name": "", "status": status }] }] }] })
                }
                ("hc", Some("get")) => json!({ "foreground": { "pid": 7 }, "title": "work", "cwd": "/x" }),
                ("herdr-app", Some("status")) => json!({ "space": "sp", "tab": "tb" }),
                ("herdr-app", Some("do")) => json!({ "terminal": "t2" }),
                _ => Value::Null,
            })
        }))
    }

    fn ask(workdir: &Path) -> Ask {
        Ask {
            prompt: "/pr-address u".into(),
            session: Some(("s1".into(), workdir.to_string_lossy().into())),
            space: String::new(),
            label: String::new(),
            tab: "t".into(),
            repo: "api".into(),
            workdir: workdir.to_path_buf(),
        }
    }

    fn live() -> HashMap<u64, String> {
        HashMap::from([(7, "s1".to_string())])
    }

    #[test]
    fn prompts_an_idle_running_agent() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let dir = std::env::temp_dir();
        assert_eq!(send(&app("idle", calls.clone()), &live(), &ask(&dir)), Ok(Sent::Prompted));
        assert!(calls.lock().unwrap().contains(&"hc send t1 /pr-address u".to_string()));
    }

    #[test]
    fn leaves_a_working_agent_alone() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let dir = std::env::temp_dir();
        assert_eq!(send(&app("working", calls.clone()), &live(), &ask(&dir)), Ok(Sent::Busy));
        assert!(!calls.lock().unwrap().iter().any(|c| c.starts_with("hc send")));
    }

    #[test]
    fn resumes_the_session_when_no_agent_runs_it() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let dir = std::env::temp_dir();
        assert_eq!(send(&app("idle", calls.clone()), &HashMap::new(), &ask(&dir)), Ok(Sent::Started));
        let all = calls.lock().unwrap().join("\n");
        assert!(all.contains("tab.new") && all.contains("--resume") && all.contains("/pr-address u"), "{all}");
    }

    #[test]
    fn with_nothing_to_resume_and_no_repo_folder_it_says_so() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut a = ask(Path::new("/no/such/dir"));
        a.session = None;
        assert!(send(&app("idle", calls), &HashMap::new(), &a).is_err());
    }
}
