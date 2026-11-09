//! End to end against the real Herdr desktop app. Skipped unless `CAPCOM_LIVE_HERDR_APP=1`, because it
//! needs the app open and starts a real Claude Code session (one short reply) in a scratch space.
//!
//!   CAPCOM_LIVE_HERDR_APP=1 cargo test --test herdr_app_live -- --nocapture
use capcom::herdr_app::{answers, pane_of, sessions_by_pid, HerdrApp, Start};
use serde_json::Value;
use std::process::Command;
use std::time::{Duration, Instant};

const SPACE: &str = "capcom-e2e";

fn herdr_app(args: &[&str]) -> Value {
    let out = Command::new("herdr-app").args(args).output().expect("herdr-app runs");
    serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
}

fn spaces_named(name: &str) -> Vec<String> {
    herdr_app(&["ls"])["spaces"].as_array().into_iter().flatten().filter(|s| s["name"] == name).filter_map(|s| s["id"].as_str().map(str::to_string)).collect()
}

fn until<T>(what: &str, secs: u64, mut f: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn clean() {
    for id in spaces_named(SPACE) {
        herdr_app(&["do", "space.close", "--id", &id, "--confirmed", "true"]);
    }
}

#[test]
fn an_agent_is_started_found_focused_and_closed_in_the_herdr_app() {
    if std::env::var("CAPCOM_LIVE_HERDR_APP").as_deref() != Ok("1") {
        eprintln!("skipped: set CAPCOM_LIVE_HERDR_APP=1 to run against the open Herdr app");
        return;
    }
    assert!(answers(), "the Herdr app has no window open");
    clean();
    let folder = std::env::current_dir().unwrap().to_string_lossy().to_string();
    let config = capcom::session::config_dir().expect("a Claude config folder");
    let app = HerdrApp::new();
    let shown = herdr_app(&["status"]);
    let start = |tab: &str| {
        app.start(&Start { space: SPACE, label: SPACE, tab, folder: &folder, argv: vec!["claude".into()], prompt: Some("reply with the single word ok"), focus: false }).expect("started")
    };

    let terminal = start("first");
    assert_eq!(herdr_app(&["status"])["tab"], shown["tab"], "the tab you were on is shown again");
    assert_eq!(spaces_named(SPACE).len(), 1);

    let agent = until("the agent to be listed with its session", 90, || {
        let sessions = sessions_by_pid(&capcom::claude::live(&config));
        app.agents(&sessions).ok()?.into_iter().find(|a| a.pane == pane_of(&terminal) && a.session.is_some())
    });
    assert_eq!((agent.workspace.as_str(), agent.cwd.as_str()), (SPACE, folder.as_str()));

    let second = start("second");
    assert_ne!(second, terminal);
    assert_eq!(spaces_named(SPACE).len(), 1, "a second tab joins the story's space");

    app.focus(&terminal).expect("focused");
    assert_eq!(herdr_app(&["status"])["agent"], terminal.as_str(), "focus shows that agent");

    app.close(&terminal).expect("closed");
    until("the agent to be gone", 30, || (!app.agents(&Default::default()).ok()?.iter().any(|a| a.pane == pane_of(&terminal))).then_some(()));
    clean();
    assert!(spaces_named(SPACE).is_empty());
    herdr_app(&["do", "tab.select", "--id", shown["tab"].as_str().unwrap_or_default()]);
}
