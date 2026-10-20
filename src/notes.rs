//! A short note each agent keeps about itself: what it was asked, where it has got to, what it is
//! waiting on and what you need to do. One small file per Claude Code session, kept in a state
//! folder (not the cache, which is tidied after a day) so it is still there when you come back.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// The Claude Code session that wrote it.
    pub session: String,
    /// What the agent was asked to do, in a line.
    #[serde(default)]
    pub task: String,
    /// Where it has got to.
    #[serde(default)]
    pub progress: String,
    /// What it is waiting on (empty when it is not waiting).
    #[serde(default)]
    pub waiting: String,
    /// The wait is on you.
    #[serde(default)]
    pub needs_you: bool,
    /// What you need to do next, if anything.
    #[serde(default)]
    pub next: String,
    /// Where the session was started, so it can be resumed.
    #[serde(default)]
    pub cwd: String,
    /// RFC 3339 time of the last update.
    #[serde(default)]
    pub updated_at: String,
}

/// What a `capcom note` call changes; fields left as `None` keep what the note already says.
#[derive(Clone, Debug, Default)]
pub struct Update {
    pub task: Option<String>,
    pub progress: Option<String>,
    pub waiting: Option<String>,
    pub needs_you: Option<bool>,
    pub next: Option<String>,
    pub cwd: Option<String>,
}

const LONGEST: usize = 400;
const KEEP_DAYS: i64 = 60;

/// `$CAPCOM_NOTES_DIR`, else `$XDG_STATE_HOME/capcom/notes`, else `~/.local/state/capcom/notes`.
pub fn dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CAPCOM_NOTES_DIR") {
        return Some(PathBuf::from(dir));
    }
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/state")))?;
    Some(state.join("capcom/notes"))
}

fn file_for(dir: &Path, session: &str) -> Result<PathBuf> {
    if session.is_empty() || !session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!("not a session id: {session:?}");
    }
    Ok(dir.join(format!("{session}.json")))
}

fn clean(text: &str) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    one_line.chars().take(LONGEST).collect()
}

pub fn read(dir: &Path, session: &str) -> Option<Note> {
    serde_json::from_slice(&std::fs::read(file_for(dir, session).ok()?).ok()?).ok()
}

/// Apply `update` to the session's note (starting a new one if there is none) and save it.
pub fn write(dir: &Path, session: &str, update: Update, now: &str) -> Result<Note> {
    let file = file_for(dir, session)?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut note = read(dir, session).unwrap_or_else(|| Note { session: session.to_string(), ..Note::default() });
    if let Some(v) = update.task {
        note.task = clean(&v);
    }
    if let Some(v) = update.progress {
        note.progress = clean(&v);
    }
    if let Some(v) = update.waiting {
        note.waiting = clean(&v);
    }
    if let Some(v) = update.needs_you {
        note.needs_you = v;
    }
    if let Some(v) = update.next {
        note.next = clean(&v);
    }
    if let Some(v) = update.cwd {
        note.cwd = v;
    }
    note.updated_at = now.to_string();
    let tmp = file.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&note)?)?;
    std::fs::rename(&tmp, &file)?;
    Ok(note)
}

/// Take a session's note away (you have dealt with it, or it is no longer wanted).
pub fn remove(dir: &Path, session: &str) {
    if let Ok(file) = file_for(dir, session) {
        let _ = std::fs::remove_file(file);
    }
}

/// Every note, newest first. Notes not touched for two months are deleted as they are found.
pub fn read_all(dir: &Path, now: chrono::DateTime<chrono::Utc>) -> Vec<Note> {
    let mut notes: Vec<Note> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice::<Note>(&std::fs::read(e.path()).ok()?).ok())
        .collect();
    notes.retain(|n| {
        let old = chrono::DateTime::parse_from_rfc3339(&n.updated_at).is_ok_and(|t| now.signed_duration_since(t).num_days() > KEEP_DAYS);
        if old {
            remove(dir, &n.session);
        }
        !old
    });
    notes.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    notes
}

/// Cheap to compute: changes whenever a note is added, changed or removed.
pub fn stamp(dir: &Path) -> Vec<(std::ffi::OsString, Option<std::time::SystemTime>)> {
    let mut all: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| (e.file_name(), e.metadata().ok().and_then(|m| m.modified().ok())))
        .collect();
    all.sort();
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(f: impl FnOnce(&mut Update)) -> Update {
        let mut u = Update::default();
        f(&mut u);
        u
    }

    #[test]
    fn a_note_is_built_up_by_partial_updates() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "abc-1", update(|u| u.task = Some("find why the retry loops".into())), "2026-10-01T00:00:00Z").unwrap();
        let note = write(dir.path(), "abc-1", update(|u| u.progress = Some("read the handler\nit retries on 500".into())), "2026-10-01T01:00:00Z").unwrap();
        assert_eq!(note.task, "find why the retry loops", "an update leaves the other fields alone");
        assert_eq!(note.progress, "read the handler it retries on 500", "one line");
        assert_eq!(note.updated_at, "2026-10-01T01:00:00Z");
        let note = write(dir.path(), "abc-1", update(|u| { u.waiting = Some("your call on the fix".into()); u.needs_you = Some(true); u.next = Some("pick option A or B".into()) }), "2026-10-01T02:00:00Z").unwrap();
        assert!(note.needs_you);
        let cleared = write(dir.path(), "abc-1", update(|u| { u.waiting = Some(String::new()); u.needs_you = Some(false); u.next = Some(String::new()) }), "2026-10-01T03:00:00Z").unwrap();
        assert_eq!((cleared.waiting.as_str(), cleared.needs_you, cleared.next.as_str()), ("", false, ""));
        assert_eq!(read(dir.path(), "abc-1"), Some(cleared));
    }

    #[test]
    fn a_session_id_cannot_name_a_file_outside_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert!(write(dir.path(), "../x", Update::default(), "t").is_err());
        assert!(write(dir.path(), "", Update::default(), "t").is_err());
        assert!(read(dir.path(), "a/b").is_none());
    }

    #[test]
    fn notes_come_back_newest_first_and_old_ones_are_deleted() {
        let dir = tempfile::tempdir().unwrap();
        for (id, at) in [("old", "2026-01-01T00:00:00Z"), ("a", "2026-10-01T00:00:00Z"), ("b", "2026-10-02T00:00:00Z")] {
            write(dir.path(), id, update(|u| u.task = Some(id.into())), at).unwrap();
        }
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-05T00:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let ids: Vec<String> = read_all(dir.path(), now).into_iter().map(|n| n.session).collect();
        assert_eq!(ids, vec!["b", "a"]);
        assert!(read(dir.path(), "old").is_none(), "gone from the disk too");
    }

    #[test]
    fn the_stamp_changes_when_a_note_does() {
        let dir = tempfile::tempdir().unwrap();
        let before = stamp(dir.path());
        write(dir.path(), "s", Update::default(), "t").unwrap();
        assert_ne!(stamp(dir.path()), before);
    }

    #[test]
    fn a_very_long_note_is_cut() {
        let dir = tempfile::tempdir().unwrap();
        let note = write(dir.path(), "s", update(|u| u.progress = Some("x".repeat(5000))), "t").unwrap();
        assert_eq!(note.progress.chars().count(), LONGEST);
    }
}
