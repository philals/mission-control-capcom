use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

fn sb(root: &TempDir, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("capcom")
        .unwrap()
        .arg("--root")
        .arg(root.path())
        .args(args)
        .output()
        .unwrap()
}

fn ok(root: &TempDir, args: &[&str]) -> Value {
    let out = sb(root, args);
    assert!(
        out.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn fails(root: &TempDir, args: &[&str]) -> String {
    let out = sb(root, args);
    assert!(!out.status.success(), "{args:?} should have failed");
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn full_task_lifecycle() {
    let root = TempDir::new().unwrap();
    ok(&root, &["init", "PROJ-1", "--title", "A story"]);
    let t1 = ok(&root, &["add-task", "PROJ-1", "--title", "Add endpoint", "--repos", "api"]);
    assert_eq!(t1["id"], "T1");
    let t2 = ok(&root, &["add-task", "PROJ-1", "--title", "Wire UI", "--depends", "T1", "--repos", "ui,api"]);
    assert_eq!(t2["id"], "T2");
    assert_eq!(ok(&root, &["ready", "PROJ-1"])["ready"], serde_json::json!(["T1"]));

    ok(&root, &["status", "PROJ-1", "T1", "planning"]);
    ok(&root, &["set-agent", "PROJ-1", "T1", "--pane", "w1:p1", "--skill", "story-plan-task"]);
    let planned = ok(&root, &["status", "PROJ-1", "T1", "planned"]);
    assert!(planned.get("agent").is_none());
    ok(&root, &["status", "PROJ-1", "T1", "implementing"]);
    ok(&root, &["add-pr", "PROJ-1", "T1", "--repo", "api", "--url", "https://github.com/o/api/pull/1"]);
    ok(&root, &["set-pr", "PROJ-1", "T1", "--url", "https://github.com/o/api/pull/1", "--state", "ready"]);
    fails(&root, &["status", "PROJ-1", "T1", "in_review"]);
    ok(&root, &["set-pr", "PROJ-1", "T1", "--url", "https://github.com/o/api/pull/1", "--state", "merged"]);
    ok(&root, &["status", "PROJ-1", "T1", "done"]);
    let with_session = ok(&root, &["set-session", "PROJ-1", "T1", "--skill", "story-implement-task", "--session", "abc-123", "--cwd", "/work/api"]);
    assert_eq!(with_session["sessions"][0]["id"], "abc-123");
    assert_eq!(with_session["sessions"][0]["cwd"], "/work/api");
    assert_eq!(ok(&root, &["ready", "PROJ-1"])["ready"], serde_json::json!(["T2"]));

    let board = ok(&root, &["show", "PROJ-1"]);
    assert_eq!(board["tasks"][0]["status"], "done");
    ok(&root, &["validate", "PROJ-1"]);
}

#[test]
fn errors_exit_nonzero_with_a_message() {
    let root = TempDir::new().unwrap();
    ok(&root, &["init", "PROJ-1", "--title", "s"]);
    assert!(fails(&root, &["show", "../etc"]).contains("invalid story key"));
    assert!(fails(&root, &["init", "PROJ-1", "--title", "again"]).contains("already exists"));
    assert!(fails(&root, &["add-task", "PROJ-1", "--title", "x", "--depends", "T9"]).contains("unknown task T9"));
    ok(&root, &["add-task", "PROJ-1", "--title", "x"]);
    assert!(fails(&root, &["status", "PROJ-1", "T1", "bogus"]).contains("unknown status"));
    assert!(fails(&root, &["status", "PROJ-1", "T1", "done"]).contains("illegal transition"));
    assert!(fails(&root, &["add-task", "PROJ-1", "--title", "y", "--type", "epic"]).contains("unknown task type"));
}

#[test]
fn list_shows_stories_with_task_counts() {
    let root = TempDir::new().unwrap();
    ok(&root, &["init", "PROJ-2", "--title", "Second"]);
    ok(&root, &["init", "PROJ-1", "--title", "First"]);
    ok(&root, &["add-task", "PROJ-1", "--title", "x"]);
    let list = ok(&root, &["list"]);
    assert_eq!(list[0]["key"], "PROJ-1");
    assert_eq!(list[0]["tasks"], 1);
    assert_eq!(list[1]["key"], "PROJ-2");
}

#[test]
fn hand_edited_invalid_board_is_reported_and_not_rewritten() {
    let root = TempDir::new().unwrap();
    ok(&root, &["init", "PROJ-1", "--title", "s"]);
    let path = root.path().join("PROJ-1/board.json");
    let bad = std::fs::read_to_string(&path).unwrap().replace("\"tasks\": []", "\"tasks\": [], \"oops\": 1");
    std::fs::write(&path, &bad).unwrap();
    assert!(fails(&root, &["add-task", "PROJ-1", "--title", "x"]).contains("schema validation"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), bad);
}

fn story_with_tasks(root: &TempDir, n: usize) {
    ok(root, &["init", "PROJ-1", "--title", "s"]);
    for i in 0..n {
        ok(root, &["add-task", "PROJ-1", "--title", &format!("t{i}")]);
    }
}

#[test]
fn block_and_unblock() {
    let root = TempDir::new().unwrap();
    story_with_tasks(&root, 1);
    ok(&root, &["status", "PROJ-1", "T1", "planning"]);
    ok(&root, &["status", "PROJ-1", "T1", "planned"]);
    let blocked = ok(&root, &["block", "PROJ-1", "T1", "--reason", "waiting on API"]);
    assert_eq!(blocked["blocked"]["reason"], "waiting on API");
    assert!(fails(&root, &["block", "PROJ-1", "T1", "--reason", "  "]).contains("reason"));
    assert!(fails(&root, &["status", "PROJ-1", "T1", "implementing"]).contains("blocked"));
    let unblocked = ok(&root, &["unblock", "PROJ-1", "T1"]);
    assert!(unblocked.get("blocked").is_none());
    ok(&root, &["status", "PROJ-1", "T1", "implementing"]);
}

#[test]
fn set_deps_sets_rejects_and_clears() {
    let root = TempDir::new().unwrap();
    story_with_tasks(&root, 2);
    ok(&root, &["set-deps", "PROJ-1", "T2", "--depends", "T1"]);
    assert_eq!(ok(&root, &["show", "PROJ-1"])["tasks"][1]["dependsOn"], serde_json::json!(["T1"]));
    assert!(fails(&root, &["set-deps", "PROJ-1", "T2", "--depends", "T9"]).contains("unknown task"));
    assert!(fails(&root, &["set-deps", "PROJ-1", "T1", "--depends", "T2"]).contains("cycle"));
    let board = ok(&root, &["show", "PROJ-1"]);
    assert_eq!(board["tasks"][0]["dependsOn"], serde_json::json!([]));
    assert_eq!(board["tasks"][1]["dependsOn"], serde_json::json!(["T1"]));
    ok(&root, &["set-deps", "PROJ-1", "T2"]);
    assert_eq!(ok(&root, &["show", "PROJ-1"])["tasks"][1]["dependsOn"], serde_json::json!([]));
}

#[test]
fn set_repos_sets_and_clears() {
    let root = TempDir::new().unwrap();
    story_with_tasks(&root, 1);
    let t = ok(&root, &["set-repos", "PROJ-1", "T1", "--repos", "api,ui"]);
    assert_eq!(t["repos"], serde_json::json!(["api", "ui"]));
    let t = ok(&root, &["set-repos", "PROJ-1", "T1"]);
    assert_eq!(t["repos"], serde_json::json!([]));
}

#[test]
fn set_subtask_validates_the_key() {
    let root = TempDir::new().unwrap();
    story_with_tasks(&root, 1);
    let t = ok(&root, &["set-subtask", "PROJ-1", "T1", "PROJ-7"]);
    assert_eq!(t["jiraSubtask"], "PROJ-7");
    assert!(fails(&root, &["set-subtask", "PROJ-1", "T1", "not a key"]).contains("invalid Jira key"));
}

#[test]
fn set_and_clear_agent() {
    let root = TempDir::new().unwrap();
    story_with_tasks(&root, 1);
    let t = ok(&root, &["set-agent", "PROJ-1", "T1", "--pane", "w1:p1", "--skill", "story-plan-task"]);
    assert_eq!(t["agent"]["pane"], "w1:p1");
    assert_eq!(t["agent"]["skill"], "story-plan-task");
    assert!(!t["agent"]["startedAt"].as_str().unwrap().is_empty());
    let t = ok(&root, &["clear-agent", "PROJ-1", "T1"]);
    assert!(t.get("agent").is_none());
}

#[test]
fn validate_reports_a_bad_board() {
    let root = TempDir::new().unwrap();
    ok(&root, &["init", "PROJ-1", "--title", "s"]);
    let path = root.path().join("PROJ-1/board.json");
    let bad = std::fs::read_to_string(&path).unwrap().replace("\"tasks\": []", "\"tasks\": [], \"oops\": 1");
    std::fs::write(&path, bad).unwrap();
    assert!(fails(&root, &["validate", "PROJ-1"]).contains("schema validation"));
}

#[test]
fn refresh_without_prs_prints_no_messages() {
    let root = TempDir::new().unwrap();
    story_with_tasks(&root, 1);
    assert_eq!(ok(&root, &["refresh", "PROJ-1"]), serde_json::json!({ "messages": [] }));
}

#[test]
fn failed_mutation_leaves_board_untouched() {
    let root = TempDir::new().unwrap();
    story_with_tasks(&root, 1);
    let path = root.path().join("PROJ-1/board.json");
    let before = std::fs::read(&path).unwrap();
    assert!(fails(&root, &["status", "PROJ-1", "T1", "done"]).contains("illegal transition"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn pr_commands_reject_bad_input() {
    let root = TempDir::new().unwrap();
    story_with_tasks(&root, 1);
    let url = "https://github.com/o/api/pull/1";
    ok(&root, &["add-pr", "PROJ-1", "T1", "--repo", "api", "--url", url]);
    assert!(fails(&root, &["add-pr", "PROJ-1", "T1", "--repo", "api", "--url", url]).contains("already has a PR"));
    assert!(fails(&root, &["set-pr", "PROJ-1", "T1", "--url", "https://x/y", "--state", "ready"]).contains("no PR with url"));
    assert!(fails(&root, &["set-pr", "PROJ-1", "T1", "--url", url, "--state", "bogus"]).contains("unknown PR state"));
    assert!(fails(&root, &["add-pr", "PROJ-1", "T1", "--repo", "api", "--url", "https://x/z", "--state", "bogus"]).contains("unknown PR state"));
}

#[test]
fn empty_depends_value_is_a_clear_error() {
    let root = TempDir::new().unwrap();
    ok(&root, &["init", "PROJ-1", "--title", "s"]);
    assert!(fails(&root, &["add-task", "PROJ-1", "--title", "x", "--depends", ""]).contains("unknown task"));
    assert_eq!(ok(&root, &["show", "PROJ-1"])["tasks"], serde_json::json!([]));
}

#[test]
fn list_reports_a_broken_board_and_continues() {
    let root = TempDir::new().unwrap();
    ok(&root, &["init", "PROJ-1", "--title", "Good"]);
    std::fs::create_dir_all(root.path().join("PROJ-2")).unwrap();
    std::fs::write(root.path().join("PROJ-2/board.json"), "{}").unwrap();
    let rows = ok(&root, &["list"]);
    assert_eq!(rows[0]["key"], "PROJ-1");
    assert_eq!(rows[0]["tasks"], 0);
    assert_eq!(rows[1]["key"], "PROJ-2");
    assert!(rows[1]["error"].as_str().unwrap().contains("schema validation"));
}

#[test]
fn help_describes_subcommands() {
    let root = TempDir::new().unwrap();
    let out = sb(&root, &["status", "--help"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!text.contains("in_review"), "{text}");
    let out = sb(&root, &["ready", "--help"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("already implementing"));
}

#[test]
fn root_prints_the_resolved_stories_folder() {
    let root = TempDir::new().unwrap();
    assert_eq!(ok(&root, &["root"])["root"], root.path().to_str().unwrap());
}

#[test]
fn root_comes_from_the_environment_when_no_flag_is_given() {
    let root = TempDir::new().unwrap();
    let out = Command::cargo_bin("capcom")
        .unwrap()
        .env("CAPCOM_ROOT", root.path())
        .arg("root")
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["root"], root.path().to_str().unwrap());
}

#[test]
fn a_missing_root_is_a_clear_error_and_nothing_is_created() {
    let cwd = TempDir::new().unwrap();
    let out = Command::cargo_bin("capcom")
        .unwrap()
        .env_remove("CAPCOM_ROOT")
        .current_dir(cwd.path())
        .arg("list")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("CAPCOM_ROOT"), "{err}");
    assert_eq!(std::fs::read_dir(cwd.path()).unwrap().count(), 0);
}

#[test]
fn story_review_flow() {
    let root = TempDir::new().unwrap();
    ok(&root, &["init", "PROJ-1", "--title", "A story"]);
    assert_eq!(ok(&root, &["show", "PROJ-1"])["story"]["status"], "in_progress");
    assert!(fails(&root, &["story-status", "PROJ-1", "in_review"]).contains("no tasks"));
    ok(&root, &["add-task", "PROJ-1", "--title", "Look into it", "--type", "spike"]);
    assert!(fails(&root, &["story-status", "PROJ-1", "in_review"]).contains("T1 is todo"));
    for s in ["planning", "planned", "implementing", "done"] {
        ok(&root, &["status", "PROJ-1", "T1", s]);
    }
    let story = ok(&root, &["story-status", "PROJ-1", "in_review"]);
    assert_eq!(story["status"], "in_review");
    assert_eq!(ok(&root, &["list"])[0]["status"], "in_review");
    assert!(fails(&root, &["story-status", "PROJ-1", "bogus"]).contains("unknown story status"));

    ok(&root, &["add-task", "PROJ-1", "--title", "Follow up"]);
    assert_eq!(ok(&root, &["show", "PROJ-1"])["story"]["status"], "in_progress");
    ok(&root, &["status", "PROJ-1", "T2", "dropped"]);
    ok(&root, &["story-status", "PROJ-1", "in_review"]);
    ok(&root, &["story-status", "PROJ-1", "done"]);
    assert!(fails(&root, &["add-task", "PROJ-1", "--title", "Late"]).contains("story is done"));
    ok(&root, &["story-status", "PROJ-1", "in_progress"]);
    ok(&root, &["add-task", "PROJ-1", "--title", "Late"]);
}
