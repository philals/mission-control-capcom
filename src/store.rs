use crate::model::Board;
use crate::rules;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const LOCK_FILE: &str = ".board.lock";

pub const SCHEMA: &str = include_str!("../schemas/board.schema.json");

pub fn valid_key(key: &str) -> bool {
    let Some((prefix, number)) = key.split_once('-') else {
        return false;
    };
    let mut chars = prefix.chars();
    let starts_alpha = chars.next().map_or(false, |c| c.is_ascii_alphabetic());
    starts_alpha
        && prefix.chars().all(|c| c.is_ascii_alphanumeric())
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

pub fn story_dir(root: &Path, key: &str) -> Result<PathBuf> {
    if !valid_key(key) {
        bail!("invalid story key {key:?} (expected like PROJ-123)");
    }
    Ok(root.join(key))
}

pub fn board_path(root: &Path, key: &str) -> Result<PathBuf> {
    Ok(story_dir(root, key)?.join("board.json"))
}

pub fn validate_value(value: &serde_json::Value) -> Result<()> {
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA).context("embedded schema is not valid JSON")?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|e| anyhow::anyhow!("embedded schema is invalid: {e}"))?;
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|e| format!("{} (at {})", e, e.instance_path))
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        bail!("board.json failed schema validation:\n  {}", errors.join("\n  "))
    }
}

pub fn load(root: &Path, key: &str) -> Result<Board> {
    let path = board_path(root, key)?;
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    let mut value: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("parsing {}", path.display()))?;
    migrate_legacy(&mut value);
    validate_value(&value)?;
    let board: Board = serde_json::from_value(value)?;
    check_key(&board, key)?;
    rules::check(&board)?;
    Ok(board)
}

/// Boards written before task review moved to the story used an `in_review` task status.
fn migrate_legacy(value: &mut serde_json::Value) {
    let Some(tasks) = value.get_mut("tasks").and_then(|t| t.as_array_mut()) else {
        return;
    };
    for task in tasks {
        if task.get("status").and_then(|s| s.as_str()) == Some("in_review") {
            task["status"] = serde_json::Value::String("implementing".to_string());
        }
    }
}

fn check_key(board: &Board, key: &str) -> Result<()> {
    if board.story.key != key {
        bail!("board is for story {} but folder is {key}", board.story.key);
    }
    Ok(())
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn save(root: &Path, key: &str, board: &Board) -> Result<()> {
    let path = board_path(root, key)?;
    let value = serde_json::to_value(board)?;
    validate_value(&value)?;
    check_key(board, key)?;
    rules::check(board)?;
    let tmp = path.with_extension(format!(
        "json.{}.{}.tmp",
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let written = std::fs::write(&tmp, serde_json::to_string_pretty(&value)? + "\n")
        .with_context(|| format!("writing {}", tmp.display()))
        .and_then(|_| {
            std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))
        });
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

pub fn update<T>(root: &Path, key: &str, f: impl FnOnce(&mut Board) -> Result<T>) -> Result<T> {
    let dir = story_dir(root, key)?;
    if !dir.is_dir() {
        load(root, key)?;
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(dir.join(LOCK_FILE))
        .with_context(|| format!("opening lock file in {}", dir.display()))?;
    lock.lock().context("locking board")?;
    let mut board = load(root, key)?;
    let out = f(&mut board)?;
    save(root, key, &board)?;
    Ok(out)
}

pub fn list_keys(root: &Path) -> Result<Vec<String>> {
    let mut keys = Vec::new();
    if !root.exists() {
        return Ok(keys);
    }
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if valid_key(&name) && entry.path().join("board.json").exists() {
            keys.push(name);
        }
    }
    keys.sort();
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Board;
    use tempfile::TempDir;

    fn story(root: &TempDir, key: &str) {
        std::fs::create_dir_all(root.path().join(key)).unwrap();
    }

    #[test]
    fn valid_key_accepts_jira_keys_only() {
        assert!(valid_key("PROJ-123"));
        assert!(valid_key("AB2-7"));
        for bad in ["", "../x", "a/b", "PROJ", "PROJ-", "-1", "1-1", "PROJ-1/..", "PROJ-1 ", "PROJ--1"] {
            assert!(!valid_key(bad), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn story_dir_rejects_path_traversal() {
        let root = TempDir::new().unwrap();
        assert!(story_dir(root.path(), "../etc").is_err());
        assert!(board_path(root.path(), "a/b-1").is_err());
    }

    #[test]
    fn save_then_load_round_trips() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1");
        let board = Board::new("PROJ-1", "A story", Some("https://j/PROJ-1"));
        save(root.path(), "PROJ-1", &board).unwrap();
        assert_eq!(load(root.path(), "PROJ-1").unwrap(), board);
        assert!(!root.path().join("PROJ-1/board.json.tmp").exists());
    }

    #[test]
    fn load_rejects_unknown_status_and_leaves_file_untouched() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1");
        let text = r#"{"schemaVersion":1,"story":{"key":"PROJ-1","title":"s"},"tasks":[
          {"id":"T1","title":"t","type":"pr","status":"finished","dependsOn":[],"file":"tasks/T1.md","repos":[],"prs":[]}]}"#;
        let path = root.path().join("PROJ-1/board.json");
        std::fs::write(&path, text).unwrap();
        let err = load(root.path(), "PROJ-1").unwrap_err().to_string();
        assert!(err.contains("schema validation"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn load_rejects_unknown_field() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1");
        let text = r#"{"schemaVersion":1,"story":{"key":"PROJ-1","title":"s"},"tasks":[],"extra":1}"#;
        std::fs::write(root.path().join("PROJ-1/board.json"), text).unwrap();
        assert!(load(root.path(), "PROJ-1").is_err());
    }

    #[test]
    fn load_and_save_refuse_a_board_for_another_story() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1");
        story(&root, "PROJ-2");
        let board = Board::new("PROJ-1", "A story", None);
        let err = save(root.path(), "PROJ-2", &board).unwrap_err().to_string();
        assert!(err.contains("board is for story PROJ-1 but folder is PROJ-2"), "{err}");
        save(root.path(), "PROJ-1", &board).unwrap();
        std::fs::copy(
            root.path().join("PROJ-1/board.json"),
            root.path().join("PROJ-2/board.json"),
        )
        .unwrap();
        let err = load(root.path(), "PROJ-2").unwrap_err().to_string();
        assert!(err.contains("board is for story PROJ-1 but folder is PROJ-2"), "{err}");
    }

    #[test]
    fn concurrent_updates_do_not_lose_tasks() {
        let root = TempDir::new().unwrap();
        crate::ops::init_story(root.path(), "PROJ-1", "A story", None).unwrap();
        let dir = root.path().join("PROJ-1");
        std::thread::scope(|s| {
            for i in 0..8 {
                let (root, dir) = (root.path(), &dir);
                s.spawn(move || {
                    update(root, "PROJ-1", |b| {
                        crate::ops::add_task(dir, b, &format!("Task {i}"), crate::model::TaskType::Pr, vec![], vec![])
                    })
                    .unwrap();
                });
            }
        });
        let board = load(root.path(), "PROJ-1").unwrap();
        assert_eq!(board.tasks.len(), 8);
        let leftovers = std::fs::read_dir(&dir)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn update_on_a_missing_story_reports_the_read_error() {
        let root = TempDir::new().unwrap();
        let err = update(root.path(), "PROJ-1", |_| Ok(())).unwrap_err().to_string();
        assert!(err.contains("reading"), "{err}");
    }

    #[test]
    fn list_keys_returns_only_valid_story_dirs() {
        let root = TempDir::new().unwrap();
        for k in ["PROJ-2", "PROJ-1", "notes"] {
            story(&root, k);
            std::fs::write(root.path().join(k).join("board.json"), "{}").unwrap();
        }
        assert_eq!(list_keys(root.path()).unwrap(), vec!["PROJ-1", "PROJ-2"]);
    }
    #[test]
    fn legacy_boards_load_with_in_review_tasks_as_implementing_and_story_in_progress() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1");
        let text = r#"{"schemaVersion":1,"story":{"key":"PROJ-1","title":"s"},"tasks":[
          {"id":"T1","title":"t","type":"pr","status":"in_review","dependsOn":[],"file":"tasks/T1.md","repos":[],"prs":[]}]}"#;
        std::fs::write(root.path().join("PROJ-1/board.json"), text).unwrap();
        let board = load(root.path(), "PROJ-1").unwrap();
        assert_eq!(board.tasks[0].status, crate::model::Status::Implementing);
        assert_eq!(board.story.status, crate::model::StoryStatus::InProgress);
        save(root.path(), "PROJ-1", &board).unwrap();
        let saved = std::fs::read_to_string(root.path().join("PROJ-1/board.json")).unwrap();
        assert!(saved.contains("\"status\": \"implementing\""), "{saved}");
        assert!(saved.contains("\"status\": \"in_progress\""), "{saved}");
        assert!(!saved.contains("in_review"), "{saved}");
    }

    #[test]
    fn an_unknown_story_status_is_rejected() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1");
        let text = r#"{"schemaVersion":1,"story":{"key":"PROJ-1","title":"s","status":"finished"},"tasks":[]}"#;
        std::fs::write(root.path().join("PROJ-1/board.json"), text).unwrap();
        assert!(load(root.path(), "PROJ-1").unwrap_err().to_string().contains("schema validation"));
    }
}
