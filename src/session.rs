//! Finding where a Claude Code session was started. `claude --resume ID` only finds a conversation
//! from the directory it began in, and that is the `cwd` of the first entry in its transcript.
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".claude")))
}

pub fn start_dir(session_id: &str) -> Option<String> {
    start_dir_in(&config_dir()?, session_id)
}

pub fn start_dir_in(config: &Path, session_id: &str) -> Option<String> {
    if session_id.is_empty() || session_id.contains(['/', '\\']) {
        return None;
    }
    let file = format!("{session_id}.jsonl");
    for project in std::fs::read_dir(config.join("projects")).ok()?.flatten() {
        let Ok(transcript) = std::fs::File::open(project.path().join(&file)) else {
            continue;
        };
        let first = BufReader::new(transcript).lines().take(20).map_while(Result::ok).find_map(|line| {
            let v: serde_json::Value = serde_json::from_str(&line).ok()?;
            v["cwd"].as_str().map(str::to_string)
        });
        if first.is_some() {
            return first;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_start_dir_is_the_first_cwd_in_the_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("projects/-work-api");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("abc.jsonl"), "{\"type\":\"summary\"}\n{\"cwd\":\"/work/api\"}\n{\"cwd\":\"/work/api/src\"}\n").unwrap();
        assert_eq!(start_dir_in(dir.path(), "abc").as_deref(), Some("/work/api"));
        assert_eq!(start_dir_in(dir.path(), "missing"), None);
        assert_eq!(start_dir_in(dir.path(), "../abc"), None);
    }
}
