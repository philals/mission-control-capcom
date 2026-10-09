//! Starts agents in Herdr: one workspace per story, one tab per piece of work.
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// One agent to start: where it lives and what it is told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    /// The story key: how an existing workspace is recognised.
    pub workspace: String,
    /// What a new (or renamed) workspace is called, such as `PROJ-123 - Notification preferences`.
    pub label: String,
    pub tab: String,
    pub agent: String,
    pub prompt: String,
}

/// Bring back the agent that made a pull request: focus it if it is running, else resume its
/// recorded Claude Code session in a new tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resume {
    /// Empty for a PR that is on no task: the tab then opens in the workspace you are in.
    pub workspace: String,
    pub label: String,
    pub tab: String,
    /// Name for the resumed agent.
    pub agent: String,
    /// Names of agents the TUI may have started for this task, tried in order.
    pub known_agents: Vec<String>,
    /// `(session id, directory it was started in)`, when one is recorded.
    pub session: Option<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Started,
    /// An agent with that name was already running, so it was brought to the front instead.
    Focused,
}

pub trait Herdr: Send + Sync {
    fn launch(&self, launch: &Launch) -> Result<Outcome, String>;
    fn resume(&self, resume: &Resume) -> Result<Outcome, String>;
}

const SHORT_NAME_WORDS: usize = 3;
const SHORT_NAME_CHARS: usize = 28;
const TRAILING_FILLER: [&str; 9] = ["a", "an", "the", "to", "of", "for", "and", "or", "in"];

/// A very short name for a story: its first few words, without trailing filler words.
pub fn short_name(title: &str) -> String {
    let mut words: Vec<&str> = Vec::new();
    let mut length = 0;
    for word in title.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric())).filter(|w| !w.is_empty()) {
        let added = length + word.chars().count() + usize::from(!words.is_empty());
        if words.len() == SHORT_NAME_WORDS || added > SHORT_NAME_CHARS {
            break;
        }
        words.push(word);
        length = added;
    }
    while words.len() > 1 && TRAILING_FILLER.contains(&words[words.len() - 1].to_lowercase().as_str()) {
        words.pop();
    }
    words.join(" ")
}

/// The Herdr workspace name for a story: the key, a hyphen and a few words naming the feature.
pub fn workspace_label(key: &str, title: &str) -> String {
    match short_name(title).as_str() {
        "" => key.to_string(),
        name => format!("{key} - {name}"),
    }
}

/// Herdr names agents `[a-z][a-z0-9_-]{0,31}`.
pub fn agent_name(parts: &[&str]) -> String {
    let joined = parts.join("-").to_lowercase();
    let mut name: String = joined
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
        name.insert(0, 'a');
    }
    name.truncate(32);
    name
}

/// The first Jira-style key (`PROJ-123`) in pasted text, such as a plain key or a browse URL.
pub fn find_key(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_alphabetic() && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()) {
            let mut j = i;
            while j < chars.len() && chars[j].is_ascii_alphanumeric() {
                j += 1;
            }
            if j - i >= 2 && j + 1 < chars.len() && chars[j] == '-' && chars[j + 1].is_ascii_digit() {
                let mut k = j + 1;
                while k < chars.len() && chars[k].is_ascii_digit() {
                    k += 1;
                }
                if !chars.get(k).is_some_and(|c| c.is_ascii_alphabetic()) {
                    let word: String = chars[i..j].iter().collect();
                    let digits: String = chars[j + 1..k].iter().collect();
                    return Some(format!("{}-{digits}", word.to_uppercase()));
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

pub type Runner = Box<dyn Fn(&[String]) -> Result<Value, String> + Send + Sync>;

pub struct Cli {
    run: Runner,
    cwd: PathBuf,
}

impl Cli {
    pub fn new(cwd: PathBuf) -> Cli {
        Cli { run: Box::new(herdr_json), cwd }
    }

    #[cfg(test)]
    pub fn with_runner(cwd: PathBuf, run: Runner) -> Cli {
        Cli { run, cwd }
    }

    fn call(&self, args: &[&str]) -> Result<Value, String> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        (self.run)(&args)
    }

    /// The workspace for a story, by its key: labelled just `KEY` (older) or `KEY - some words`.
    fn find_workspace(&self, key: &str) -> Result<Option<(String, String)>, String> {
        let list = self.call(&["workspace", "list"])?;
        let described = format!("{key} - ");
        Ok(list["result"]["workspaces"].as_array().and_then(|all| {
            all.iter().find_map(|w| {
                let label = w["label"].as_str()?;
                (label == key || label.starts_with(&described)).then(|| (w["workspace_id"].as_str().map(str::to_string), label.to_string()))
            })
        })
        .and_then(|(id, label)| id.map(|id| (id, label))))
    }

    /// The pane of a running agent whose Claude Code session is `session`, by what Herdr reports.
    fn pane_of_session(&self, session: &str) -> Result<Option<String>, String> {
        let list = self.call(&["agent", "list"])?;
        Ok(list["result"]["agents"].as_array().and_then(|all| {
            all.iter()
                .find(|a| a["agent_session"]["value"] == session)
                .and_then(|a| a["pane_id"].as_str().map(str::to_string))
        }))
    }

    fn agent_running(&self, name: &str) -> Result<bool, String> {
        let list = self.call(&["agent", "list"])?;
        Ok(list["result"]["agents"].as_array().is_some_and(|all| all.iter().any(|a| a["name"] == name)))
    }

    /// The pane to start the agent in: the root pane of a new workspace, or of a new tab in the
    /// story's workspace.
    fn new_pane(&self, l: &Launch, cwd: Option<&str>) -> Result<String, String> {
        let cwd = cwd.map_or_else(|| self.cwd.to_string_lossy().to_string(), str::to_string);
        let pane = |v: &Value| v["result"]["root_pane"]["pane_id"].as_str().map(str::to_string);
        if l.workspace.is_empty() {
            let made = self.call(&["tab", "create", "--label", &l.tab, "--cwd", &cwd, "--no-focus"])?;
            return pane(&made).ok_or_else(|| "herdr did not return the new tab's pane".to_string());
        }
        match self.find_workspace(&l.workspace)? {
            Some((id, label)) => {
                if l.label != l.workspace && label != l.label {
                    let _ = self.call(&["workspace", "rename", &id, &l.label]);
                }
                let made = self.call(&["tab", "create", "--workspace", &id, "--label", &l.tab, "--cwd", &cwd, "--no-focus"])?;
                pane(&made).ok_or_else(|| "herdr did not return the new tab's pane".to_string())
            }
            None => {
                let made = self.call(&["workspace", "create", "--label", &l.label, "--cwd", &cwd, "--no-focus"])?;
                if let Some(tab) = made["result"]["tab"]["tab_id"].as_str() {
                    let _ = self.call(&["tab", "rename", tab, &l.tab]);
                }
                pane(&made).ok_or_else(|| "herdr did not return the new workspace's pane".to_string())
            }
        }
    }
}

impl Herdr for Cli {
    fn launch(&self, l: &Launch) -> Result<Outcome, String> {
        if self.agent_running(&l.agent)? {
            self.call(&["agent", "focus", &l.agent])?;
            return Ok(Outcome::Focused);
        }
        let pane = self.new_pane(l, None)?;
        self.call(&["agent", "start", &l.agent, "--kind", "claude", "--pane", &pane])?;
        self.call(&["agent", "prompt", &l.agent, &l.prompt])?;
        Ok(Outcome::Started)
    }

    fn resume(&self, r: &Resume) -> Result<Outcome, String> {
        self.resume_in_herdr(r)
    }
}

impl Cli {
    fn resume_in_herdr(&self, r: &Resume) -> Result<Outcome, String> {
        if let Some((id, _)) = &r.session {
            if let Some(pane) = self.pane_of_session(id)? {
                self.call(&["agent", "focus", &pane])?;
                return Ok(Outcome::Focused);
            }
        }
        for name in r.known_agents.iter().chain(std::iter::once(&r.agent)) {
            if self.agent_running(name)? {
                self.call(&["agent", "focus", name])?;
                return Ok(Outcome::Focused);
            }
        }
        let Some((id, cwd)) = &r.session else {
            return Err("no session is recorded for this task yet".into());
        };
        let launch = Launch { workspace: r.workspace.clone(), label: r.label.clone(), tab: r.tab.clone(), agent: r.agent.clone(), prompt: String::new() };
        if !std::path::Path::new(cwd).is_dir() {
            return Err(format!("its session started in {cwd}, which no longer exists"));
        }
        let pane = self.new_pane(&launch, Some(cwd))?;
        self.call(&["agent", "start", &r.agent, "--kind", "claude", "--pane", &pane, "--", "--resume", id])?;
        Ok(Outcome::Started)
    }
}

fn herdr_json(args: &[String]) -> Result<Value, String> {
    let mut cmd = Command::new("herdr");
    cmd.args(args);
    let out = run_herdr(cmd)?;
    serde_json::from_str(&out).map_err(|e| format!("herdr gave unreadable output: {e}"))
}

fn run_herdr(cmd: Command) -> Result<String, String> {
    match capcom::refresh::run_with_timeout(cmd, Duration::from_secs(60)) {
        Ok(Some(out)) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).to_string()),
        Ok(Some(out)) => {
            let text = String::from_utf8_lossy(&out.stderr).to_string();
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
                .unwrap_or_else(|| text.lines().next().unwrap_or("herdr failed").to_string());
            Err(message)
        }
        Ok(None) => Err("herdr timed out".into()),
        Err(e) => Err(format!("could not run herdr: {e}")),
    }
}

/// Title Herdr shows for the pane running this program (set at start-up, so it is reliable).
pub const PANE_TITLE: &str = "capcom-tui";

/// From `herdr pane list` output: the pane already running capcom-tui, as `pane_id tab_id`.
pub fn find_pane(list_json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(list_json).ok()?;
    value["result"]["panes"].as_array()?.iter().find_map(|p| {
        let title = p["terminal_title_stripped"].as_str().or_else(|| p["terminal_title"].as_str())?;
        if title != PANE_TITLE {
            return None;
        }
        Some(format!("{} {}", p["pane_id"].as_str()?, p["tab_id"].as_str()?))
    })
}

/// Herdr sets this in every pane it manages.
pub fn inside_herdr() -> bool {
    std::env::var("HERDR_ENV").is_ok_and(|v| v == "1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[test]
    fn keys_are_found_in_plain_text_and_in_urls() {
        assert_eq!(find_key("PROJ-123").as_deref(), Some("PROJ-123"));
        assert_eq!(find_key("  proj-7 \n").as_deref(), Some("PROJ-7"));
        assert_eq!(find_key("https://acme.atlassian.net/browse/ABC2-45?focusedId=9").as_deref(), Some("ABC2-45"));
        assert_eq!(find_key("see https://x.example/jira/DEMO-482#c").as_deref(), Some("DEMO-482"));
        assert_eq!(find_key("no key here, 2026-10-09"), None);
        assert_eq!(find_key("A-1"), None, "a key needs at least two letters");
        assert_eq!(find_key("PROJ-12abc"), None);
    }

    #[test]
    fn agent_names_follow_herdrs_rules() {
        assert_eq!(agent_name(&["PROJ-123", "T2", "plan"]), "proj-123-t2-plan");
        assert_eq!(agent_name(&["9x"]), "a9x");
        let long = agent_name(&["A".repeat(40).as_str(), "t1"]);
        assert_eq!(long.len(), 32);
        assert!(long.starts_with(|c: char| c.is_ascii_lowercase()));
        assert_eq!(agent_name(&["we ird.key", "T1"]), "we-ird-key-t1");
    }

    #[test]
    fn an_existing_capcom_pane_is_found_by_its_title() {
        let list = json!({"result": {"panes": [
            {"pane_id": "w1:p1", "tab_id": "w1:t1", "terminal_title_stripped": "zsh"},
            {"pane_id": "w1:p4", "tab_id": "w1:t3", "terminal_title": "capcom-tui", "terminal_title_stripped": "capcom-tui"},
        ]}});
        assert_eq!(find_pane(&list.to_string()).as_deref(), Some("w1:p4 w1:t3"));
        let none = json!({"result": {"panes": [{"pane_id": "w1:p1", "tab_id": "w1:t1", "terminal_title_stripped": "capcom"}]}});
        assert_eq!(find_pane(&none.to_string()), None, "only the exact title counts");
        assert_eq!(find_pane("not json"), None);
        assert_eq!(find_pane("{}"), None);
    }

    type Calls = Arc<Mutex<Vec<String>>>;

    fn cli(workspaces: Value, agents: Value) -> (Cli, Calls) {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let log = calls.clone();
        let run: Runner = Box::new(move |args| {
            log.lock().unwrap().push(args.join(" "));
            let a: Vec<&str> = args.iter().map(String::as_str).collect();
            Ok(match a.as_slice() {
                ["workspace", "list"] => json!({"result": {"workspaces": workspaces}}),
                ["agent", "list"] => json!({"result": {"agents": agents}}),
                ["workspace", "create", ..] => json!({"result": {
                    "workspace": {"workspace_id": "w9"}, "tab": {"tab_id": "w9:t1"}, "root_pane": {"pane_id": "w9:p1"}}}),
                ["tab", "create", ..] => json!({"result": {"tab": {"tab_id": "w3:t2"}, "root_pane": {"pane_id": "w3:p2"}}}),
                _ => json!({"result": {}}),
            })
        });
        (Cli::with_runner(PathBuf::from("/work"), run), calls)
    }

    fn launch() -> Launch {
        Launch {
            workspace: "PROJ-123".into(),
            label: "PROJ-123 - Notification preferences".into(),
            tab: "T2 plan".into(),
            agent: "proj-123-t2-plan".into(),
            prompt: "/story-plan-task PROJ-123 T2".into(),
        }
    }

    #[test]
    fn a_story_without_a_workspace_gets_one_and_its_first_pane_runs_the_agent() {
        let (cli, calls) = cli(json!([{"label": "other", "workspace_id": "w1"}]), json!([]));
        assert_eq!(cli.launch(&launch()), Ok(Outcome::Started));
        let calls = calls.lock().unwrap();
        assert!(calls.contains(&"workspace create --label PROJ-123 - Notification preferences --cwd /work --no-focus".to_string()), "{calls:?}");
        assert!(calls.contains(&"tab rename w9:t1 T2 plan".to_string()), "{calls:?}");
        assert!(calls.contains(&"agent start proj-123-t2-plan --kind claude --pane w9:p1".to_string()), "{calls:?}");
        assert_eq!(calls.last().unwrap(), "agent prompt proj-123-t2-plan /story-plan-task PROJ-123 T2");
    }

    fn resume() -> Resume {
        Resume {
            workspace: "PROJ-123".into(),
            label: "PROJ-123 - Notification preferences".into(),
            tab: "T2 resume".into(),
            agent: "proj-123-t2-resume".into(),
            known_agents: vec!["proj-123-t2-impl".into()],
            session: Some(("abc-123".into(), "/work/api".into())),
        }
    }

    #[test]
    fn the_agent_running_a_recorded_session_is_focused_by_its_pane() {
        let (cli, calls) = cli(json!([]), json!([{"pane_id": "w5:p1", "agent_session": {"value": "abc-123"}}]));
        assert_eq!(cli.resume(&resume()), Ok(Outcome::Focused));
        assert!(calls.lock().unwrap().contains(&"agent focus w5:p1".to_string()));
    }

    #[test]
    fn a_running_agent_started_by_the_board_is_focused_by_name() {
        let (cli, calls) = cli(json!([]), json!([{"name": "proj-123-t2-impl", "pane_id": "w5:p1"}]));
        assert_eq!(cli.resume(&resume()), Ok(Outcome::Focused));
        assert!(calls.lock().unwrap().contains(&"agent focus proj-123-t2-impl".to_string()));
    }

    #[test]
    fn a_closed_session_is_resumed_in_a_new_tab_of_the_story_workspace() {
        let (cli, calls) = cli(json!([{"label": "PROJ-123 - Notification preferences", "workspace_id": "w3"}]), json!([]));
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        let mut r = resume();
        r.session = Some(("abc-123".into(), dir.clone()));
        assert_eq!(cli.resume(&r), Ok(Outcome::Started));
        let calls = calls.lock().unwrap();
        assert!(calls.contains(&format!("tab create --workspace w3 --label T2 resume --cwd {dir} --no-focus")), "{calls:?}");
        assert!(calls.contains(&"agent start proj-123-t2-resume --kind claude --pane w3:p2 -- --resume abc-123".to_string()), "{calls:?}");
    }

    #[test]
    fn a_pr_on_no_task_is_resumed_in_a_tab_of_the_current_workspace() {
        let (cli, calls) = cli(json!([]), json!([]));
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        let mut r = resume();
        r.workspace = String::new();
        r.known_agents = vec![];
        r.tab = "api#7 agent".into();
        r.session = Some(("abc-123".into(), dir.clone()));
        assert_eq!(cli.resume(&r), Ok(Outcome::Started));
        let calls = calls.lock().unwrap();
        assert!(calls.contains(&format!("tab create --label api#7 agent --cwd {dir} --no-focus")), "{calls:?}");
        assert!(!calls.iter().any(|c| c.starts_with("workspace ")), "{calls:?}");
    }

    #[test]
    fn a_session_whose_folder_is_gone_says_so_instead_of_starting_in_the_wrong_place() {
        let (cli, calls) = cli(json!([]), json!([]));
        let mut r = resume();
        r.session = Some(("abc-123".into(), "/no/such/folder".into()));
        let error = cli.resume(&r).unwrap_err();
        assert!(error.contains("/no/such/folder") && error.contains("no longer exists"), "{error}");
        assert!(!calls.lock().unwrap().iter().any(|c| c.starts_with("agent start")));
    }

    #[test]
    fn with_nothing_running_and_no_session_the_user_is_told() {
        let (cli, _) = cli(json!([]), json!([]));
        let mut r = resume();
        r.session = None;
        assert!(cli.resume(&r).unwrap_err().contains("no session"));
    }

    #[test]
    fn short_names_are_a_few_words_of_the_title() {
        assert_eq!(short_name("Notification preferences"), "Notification preferences");
        assert_eq!(short_name("Check Lambda business logic"), "Check Lambda business");
        assert_eq!(short_name("Add the preferences endpoint to the API"), "Add the preferences");
        assert_eq!(short_name("Billing export to CSV"), "Billing export");
        assert_eq!(short_name("  Dark mode!  "), "Dark mode");
        assert_eq!(short_name("Extraordinarily-long-unbreakable-word-name more words"), "");
        assert_eq!(short_name(""), "");
        assert!(short_name("Internationalisation Documentation Infrastructure rewrite").chars().count() <= 28);
    }

    #[test]
    fn workspace_labels_are_the_key_a_hyphen_and_a_few_words() {
        assert_eq!(workspace_label("PROJ-123", "Notification preferences"), "PROJ-123 - Notification preferences");
        assert_eq!(workspace_label("PROJ-123", ""), "PROJ-123");
        assert_eq!(workspace_label("PROJ-123", "   "), "PROJ-123");
    }

    #[test]
    fn an_older_workspace_named_only_by_the_key_is_found_and_given_the_descriptive_name() {
        let (cli, calls) = cli(json!([{"label": "PROJ-123", "workspace_id": "w3"}]), json!([]));
        cli.launch(&launch()).unwrap();
        let calls = calls.lock().unwrap();
        assert!(calls.contains(&"workspace rename w3 PROJ-123 - Notification preferences".to_string()), "{calls:?}");
        assert!(!calls.iter().any(|c| c.starts_with("workspace create")));
    }

    #[test]
    fn a_workspace_with_the_right_name_is_left_alone_and_other_keys_are_not_confused() {
        let (first, calls) = cli(json!([{"label": "PROJ-123 - Notification preferences", "workspace_id": "w3"}]), json!([]));
        first.launch(&launch()).unwrap();
        assert!(!calls.lock().unwrap().iter().any(|c| c.starts_with("workspace rename")));
        let (second, calls) = cli(json!([{"label": "PROJ-1234 - Something else", "workspace_id": "w4"}, {"label": "PROJ-12", "workspace_id": "w5"}]), json!([]));
        second.launch(&launch()).unwrap();
        let calls = calls.lock().unwrap();
        assert!(calls.iter().any(|c| c.starts_with("workspace create")), "PROJ-123 is not PROJ-1234 or PROJ-12: {calls:?}");
    }

    #[test]
    fn a_launch_labelled_only_with_the_key_never_renames_a_descriptive_workspace() {
        let (cli, calls) = cli(json!([{"label": "PROJ-123 - Notification preferences", "workspace_id": "w3"}]), json!([]));
        let mut plain = launch();
        plain.label = "PROJ-123".into();
        cli.launch(&plain).unwrap();
        assert!(!calls.lock().unwrap().iter().any(|c| c.starts_with("workspace rename")));
    }

    #[test]
    fn a_story_with_a_workspace_gets_a_new_tab_in_it() {
        let (cli, calls) = cli(json!([{"label": "PROJ-123 - Notification preferences", "workspace_id": "w3"}]), json!([]));
        assert_eq!(cli.launch(&launch()), Ok(Outcome::Started));
        let calls = calls.lock().unwrap();
        assert!(
            calls.contains(&"tab create --workspace w3 --label T2 plan --cwd /work --no-focus".to_string()),
            "{calls:?}"
        );
        assert!(calls.contains(&"agent start proj-123-t2-plan --kind claude --pane w3:p2".to_string()));
        assert!(!calls.iter().any(|c| c.starts_with("workspace create")));
    }

    #[test]
    fn a_running_agent_is_focused_not_started_twice() {
        let (cli, calls) = cli(json!([]), json!([{"name": "proj-123-t2-plan", "pane_id": "w3:p2"}]));
        assert_eq!(cli.launch(&launch()), Ok(Outcome::Focused));
        let calls = calls.lock().unwrap();
        assert_eq!(calls.last().unwrap(), "agent focus proj-123-t2-plan");
        assert!(!calls.iter().any(|c| c.starts_with("agent start") || c.starts_with("agent prompt")));
    }

    #[test]
    fn a_herdr_error_stops_the_launch_and_is_reported() {
        let run: Runner = Box::new(|args| {
            if args[0] == "agent" && args[1] == "start" {
                Err("agent_not_ready".into())
            } else if args[0] == "agent" {
                Ok(json!({"result": {"agents": []}}))
            } else if args[0] == "workspace" && args[1] == "list" {
                Ok(json!({"result": {"workspaces": []}}))
            } else {
                Ok(json!({"result": {"root_pane": {"pane_id": "w9:p1"}, "tab": {"tab_id": "w9:t1"}}}))
            }
        });
        let cli = Cli::with_runner(PathBuf::from("/work"), run);
        assert_eq!(cli.launch(&launch()), Err("agent_not_ready".to_string()));
    }
}
