//! The remembered panel sizes, kept in the user's config folder (never in a repository).
use crate::app::{HEIGHT_RANGE, SPLIT_RANGE};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const COLUMNS: usize = 6;
pub const MIN_COLUMN_WEIGHT: u16 = 20;
pub const DEFAULT_SPLIT: u16 = 58;
pub const DEFAULT_COLUMN_WEIGHT: u16 = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub split_pct: u16,
    pub bottom_pct: Option<u16>,
    pub split_pinned: bool,
    pub columns: Vec<u16>,
    /// Write PR states found on GitHub to the open story's board (merged PRs finish their task).
    pub auto_sync: bool,
    pub watched: Vec<crate::runs::Watch>,
    pub autofix: bool,
    /// Per-PR auto-fix choices that override `autofix`.
    pub autofix_prs: Vec<(String, bool)>,
    pub autocopilot: bool,
    pub autocopilot_prs: Vec<(String, bool)>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            split_pct: DEFAULT_SPLIT,
            bottom_pct: None,
            split_pinned: false,
            columns: vec![DEFAULT_COLUMN_WEIGHT; COLUMNS],
            auto_sync: false,
            watched: Vec::new(),
            autofix: false,
            autofix_prs: Vec::new(),
            autocopilot: false,
            autocopilot_prs: Vec::new(),
        }
    }
}

impl Settings {
    /// Keep every value inside the range the layout can draw, whatever the file said.
    fn sanitized(mut self) -> Settings {
        self.split_pct = self.split_pct.clamp(SPLIT_RANGE.0, SPLIT_RANGE.1);
        self.bottom_pct = self.bottom_pct.map(|p| p.clamp(HEIGHT_RANGE.0, HEIGHT_RANGE.1));
        self.columns.resize(COLUMNS, DEFAULT_COLUMN_WEIGHT);
        for weight in &mut self.columns {
            *weight = (*weight).max(MIN_COLUMN_WEIGHT);
        }
        self
    }
}

/// Missing, unreadable or damaged settings simply mean the defaults.
pub fn load(path: &Path) -> Settings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
        .map_or_else(Settings::default, Settings::sanitized)
}

pub fn save(path: &Path, settings: &Settings) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(settings)? + "\n")
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|s| !s.is_empty())
}

pub fn path_from(override_path: Option<&str>, xdg: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = non_empty(override_path) {
        return Some(PathBuf::from(path));
    }
    if let Some(xdg) = non_empty(xdg) {
        return Some(Path::new(xdg).join("capcom/tui.json"));
    }
    non_empty(home).map(|home| Path::new(home).join(".config/capcom/tui.json"))
}

pub fn default_path() -> Option<PathBuf> {
    let var = |name: &str| std::env::var(name).ok();
    path_from(
        var("CAPCOM_TUI_SETTINGS").as_deref(),
        var("XDG_CONFIG_HOME").as_deref(),
        var("HOME").as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn defaults_are_the_automatic_layout() {
        let d = Settings::default();
        assert_eq!((d.split_pct, d.bottom_pct, d.split_pinned), (58, None, false));
        assert_eq!(d.columns, vec![100; 6]);
    }

    #[test]
    fn settings_survive_a_save_and_a_load() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("tui.json");
        let saved = Settings { split_pct: 63, bottom_pct: Some(50), split_pinned: true, columns: vec![120, 80, 100, 100, 100, 100], auto_sync: true, watched: vec![], autofix: true, autofix_prs: vec![("https://github.com/acme/api/pull/1".into(), false)], autocopilot: true, autocopilot_prs: vec![("https://github.com/acme/api/pull/2".into(), true)] };
        save(&path, &saved).unwrap();
        assert_eq!(load(&path), saved);
    }

    #[test]
    fn a_missing_or_damaged_file_gives_the_defaults() {
        let dir = TempDir::new().unwrap();
        assert_eq!(load(&dir.path().join("nope.json")), Settings::default());
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, "{ not json").unwrap();
        assert_eq!(load(&bad), Settings::default());
        std::fs::write(&bad, r#"{"split_pct": "wide"}"#).unwrap();
        assert_eq!(load(&bad), Settings::default());
    }

    #[test]
    fn loaded_values_are_clamped_and_unknown_fields_are_ignored() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("tui.json");
        std::fs::write(&path, r#"{"split_pct": 5, "bottom_pct": 200, "split_pinned": true, "columns": [0, 7], "future": 1}"#).unwrap();
        let s = load(&path);
        assert_eq!(s.split_pct, 25);
        assert_eq!(s.bottom_pct, Some(80));
        assert!(s.split_pinned);
        assert_eq!(s.columns.len(), 6, "always one weight per column");
        assert!(s.columns.iter().all(|w| *w >= 20), "{:?}", s.columns);
        std::fs::write(&path, r#"{"bottom_pct": 3}"#).unwrap();
        assert_eq!(load(&path).bottom_pct, Some(15));
    }

    #[test]
    fn saving_creates_the_folder_and_leaves_no_temporary_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a/b/tui.json");
        save(&path, &Settings::default()).unwrap();
        assert!(path.exists());
        let names: Vec<String> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        assert_eq!(names, vec!["tui.json"]);
    }

    #[test]
    fn the_settings_path_follows_the_override_then_xdg_then_home() {
        let p = |o: Option<&str>, x: Option<&str>, h: Option<&str>| path_from(o, x, h).map(|p| p.to_string_lossy().to_string());
        assert_eq!(p(Some("/x/s.json"), Some("/c"), Some("/h")).as_deref(), Some("/x/s.json"));
        assert_eq!(p(None, Some("/c"), Some("/h")).as_deref(), Some("/c/capcom/tui.json"));
        assert_eq!(p(None, None, Some("/h")).as_deref(), Some("/h/.config/capcom/tui.json"));
        assert_eq!(p(None, Some(""), Some("/h")).as_deref(), Some("/h/.config/capcom/tui.json"));
        assert_eq!(p(None, None, None), None);
    }
}
