//! Finding where a Claude Code session was started. `claude --resume ID` only finds a conversation
//! from the directory it began in, and that is the `cwd` of the first entry in its transcript.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub fn config_dir() -> Option<PathBuf> {
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

/// One PR a session says it created or touched, from Claude Code's `pr-link` transcript entries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub url: String,
    pub session: String,
    pub at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrSession {
    pub session: String,
    pub cwd: String,
}

#[derive(Default, Serialize, Deserialize)]
struct FileEntry {
    /// Bytes already read: always the end of a complete line.
    offset: u64,
    /// Distinct directories the session reported being in, in order (capped).
    cwds: Vec<String>,
    links: Vec<Link>,
}

/// Which session made which PR, kept on disk so only the new part of each transcript is read.
#[derive(Default, Serialize, Deserialize)]
pub struct Index {
    files: BTreeMap<String, FileEntry>,
}

const MAX_CWDS: usize = 64;

/// The first `"cwd":"…"` of a transcript line, without parsing the (often huge) line as JSON.
fn cwd_of(line: &str) -> Option<&str> {
    let start = line.find("\"cwd\":\"")? + 7;
    let end = line[start..].find('"')? + start;
    let cwd = &line[start..end];
    (!cwd.contains('\\')).then_some(cwd)
}

/// How Claude Code names a project folder after a directory: everything but letters and digits
/// becomes `-`.
fn project_name(dir: &str) -> String {
    dir.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

const LINK_MARK: &str = "\"type\":\"pr-link\"";

impl Index {
    pub fn load(path: &Path) -> Index {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec(self)?)?;
        std::fs::rename(tmp, path)
    }

    /// Read what has been added to every transcript since last time. True when anything changed.
    pub fn refresh(&mut self, config: &Path) -> bool {
        let mut seen = std::collections::HashSet::new();
        let mut changed = false;
        let projects = std::fs::read_dir(config.join("projects")).into_iter().flatten().flatten();
        for project in projects {
            for file in std::fs::read_dir(project.path()).into_iter().flatten().flatten() {
                let path = file.path();
                if path.extension().is_some_and(|e| e == "jsonl") {
                    let key = path.to_string_lossy().to_string();
                    changed |= self.read_new(&key, &path);
                    seen.insert(key);
                }
            }
        }
        let before = self.files.len();
        self.files.retain(|k, _| seen.contains(k));
        changed || self.files.len() != before
    }

    fn read_new(&mut self, key: &str, path: &Path) -> bool {
        let Ok(len) = path.metadata().map(|m| m.len()) else {
            return false;
        };
        let entry = self.files.entry(key.to_string()).or_default();
        if len < entry.offset {
            *entry = FileEntry::default();
        }
        if len == entry.offset {
            return false;
        }
        let Ok(mut file) = std::fs::File::open(path) else {
            return false;
        };
        if file.seek(SeekFrom::Start(entry.offset)).is_err() {
            return false;
        }
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        let mut changed = false;
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) if line.last() != Some(&b'\n') => break,
                Ok(n) => entry.offset += n as u64,
            }
            let text = String::from_utf8_lossy(&line);
            if let Some(cwd) = cwd_of(&text) {
                if entry.cwds.len() < MAX_CWDS && !entry.cwds.iter().any(|c| c == cwd) {
                    entry.cwds.push(cwd.to_string());
                }
            }
            if text.contains(LINK_MARK) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let (Some(url), Some(session)) = (v["prUrl"].as_str(), v["sessionId"].as_str()) {
                        let at = v["timestamp"].as_str().unwrap_or_default().to_string();
                        entry.links.push(Link { url: url.to_string(), session: session.to_string(), at });
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    /// Every PR with the session that first linked it (the one that made it) and where it started.
    pub fn sessions(&self) -> Vec<(String, PrSession)> {
        let mut best: BTreeMap<String, (&str, PrSession)> = BTreeMap::new();
        for (path, entry) in &self.files {
            // `--resume` finds a session only from the directory its transcript folder is named after
            let folder = Path::new(path).parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string());
            let home = entry.cwds.iter().find(|c| Some(project_name(c)) == folder).or(entry.cwds.first());
            let Some(cwd) = home else {
                continue;
            };
            for link in &entry.links {
                let found = PrSession { session: link.session.clone(), cwd: cwd.clone() };
                let key = link.url.trim_end_matches('/').to_lowercase();
                match best.get(&key) {
                    Some((at, _)) if *at <= link.at.as_str() => {}
                    _ => {
                        best.insert(key, (&link.at, found));
                    }
                }
            }
        }
        best.into_iter().map(|(url, (_, s))| (url, s)).collect()
    }
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

    fn link(session: &str, url: &str, at: &str) -> String {
        format!("{{\"type\":\"pr-link\",\"sessionId\":\"{session}\",\"prUrl\":\"{url}\",\"timestamp\":\"{at}\"}}\n")
    }

    fn transcript(dir: &Path, project: &str, session: &str, cwd: &str, body: &str) -> PathBuf {
        let folder = dir.join("projects").join(project);
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join(format!("{session}.jsonl"));
        std::fs::write(&path, format!("{{\"cwd\":\"{cwd}\"}}\n{body}")).unwrap();
        path
    }

    const URL: &str = "https://github.com/acme/api/pull/7";

    #[test]
    fn the_session_that_first_linked_a_pr_is_the_one_that_made_it() {
        let dir = tempfile::tempdir().unwrap();
        transcript(dir.path(), "-w-api", "later", "/w/api", &link("later", URL, "2026-10-09T05:00:00Z"));
        transcript(dir.path(), "-w-api-tree", "maker", "/w/api/tree", &link("maker", URL, "2026-10-09T03:00:00Z"));
        let mut index = Index::default();
        assert!(index.refresh(dir.path()));
        let all = index.sessions();
        assert_eq!(all, vec![(URL.to_string(), PrSession { session: "maker".into(), cwd: "/w/api/tree".into() })]);
    }

    #[test]
    fn a_session_that_moved_into_a_worktree_resumes_from_the_folder_its_transcript_lives_under() {
        let dir = tempfile::tempdir().unwrap();
        let body = "{\"cwd\":\"/w/api/.claude/worktrees/fix\"}\n".to_string() + &link("s1", URL, "t1");
        transcript(dir.path(), "-w-api--claude-worktrees-fix", "s1", "/w/api", &body);
        let mut index = Index::default();
        index.refresh(dir.path());
        assert_eq!(index.sessions()[0].1.cwd, "/w/api/.claude/worktrees/fix");
    }

    #[test]
    fn one_session_with_many_prs_in_many_repos_links_them_all() {
        let dir = tempfile::tempdir().unwrap();
        let body = link("s1", URL, "t1") + &link("s1", "https://github.com/acme/web/pull/9", "t2");
        transcript(dir.path(), "-w", "s1", "/w", &body);
        let mut index = Index::default();
        index.refresh(dir.path());
        let all = index.sessions();
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|(_, s)| s.session == "s1"));
    }

    #[test]
    fn only_the_new_part_of_a_growing_transcript_is_read_and_a_half_written_line_waits() {
        let dir = tempfile::tempdir().unwrap();
        let path = transcript(dir.path(), "-w", "s1", "/w", &link("s1", URL, "t1"));
        let mut index = Index::default();
        assert!(index.refresh(dir.path()));
        assert!(!index.refresh(dir.path()), "nothing new");
        let mut text = std::fs::read_to_string(&path).unwrap();
        let second = link("s1", "https://github.com/acme/web/pull/9", "t2");
        text.push_str(&second[..20]);
        std::fs::write(&path, &text).unwrap();
        assert!(!index.refresh(dir.path()), "a partial line is not read yet");
        text.push_str(&second[20..]);
        std::fs::write(&path, &text).unwrap();
        assert!(index.refresh(dir.path()));
        assert_eq!(index.sessions().len(), 2);
        std::fs::remove_file(&path).unwrap();
        assert!(index.refresh(dir.path()));
        assert!(index.sessions().is_empty());
    }

    #[test]
    fn the_index_survives_a_round_trip_through_its_file() {
        let dir = tempfile::tempdir().unwrap();
        transcript(dir.path(), "-w", "s1", "/w", &link("s1", URL, "t1"));
        let mut index = Index::default();
        index.refresh(dir.path());
        let file = dir.path().join("pr-links.json");
        index.save(&file).unwrap();
        let again = Index::load(&file);
        assert_eq!(again.sessions(), index.sessions());
        assert!(Index::load(&dir.path().join("missing")).sessions().is_empty());
    }
}
