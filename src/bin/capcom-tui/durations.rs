//! How long each check usually takes, learned from the checks capcom has watched pass, so a running
//! check can say "of ~4m". Kept in the shared cache folder, so it survives a restart.
use crate::prs::{check_seconds, Check, CheckState, PullRequest};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Durations remembered for each check.
const KEEP: usize = 5;
const MAX_CHECKS: usize = 400;
/// Shorter runs are skipped checks and noise; longer ones are not a normal run.
const MIN_SECONDS: i64 = 5;
const MAX_SECONDS: i64 = 6 * 3600;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    /// The last few times it passed, in seconds, oldest first.
    seconds: Vec<u32>,
    /// When the newest one finished, so the same run is never counted twice.
    last: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Durations {
    checks: HashMap<String, Entry>,
}

fn key(repo: &str, check: &Check) -> String {
    format!("{repo}|{}", check.label())
}

impl Durations {
    /// Note every check that has passed since last time. True when anything was learned.
    pub fn learn(&mut self, prs: &[PullRequest]) -> bool {
        let now = Utc::now();
        let mut passed: Vec<(&PullRequest, &Check, i64)> = prs
            .iter()
            .flat_map(|pr| pr.checks.iter().map(move |c| (pr, c)))
            .filter(|(_, c)| c.state == CheckState::Passed && c.completed_at.is_some())
            .filter_map(|(pr, c)| check_seconds(c, now).map(|s| (pr, c, s)))
            .filter(|(_, _, s)| (MIN_SECONDS..=MAX_SECONDS).contains(s))
            .collect();
        passed.sort_by(|a, b| a.1.completed_at.cmp(&b.1.completed_at));
        let mut changed = false;
        for (pr, check, seconds) in passed {
            let done = check.completed_at.clone().unwrap_or_default();
            let entry = self.checks.entry(key(&pr.repo, check)).or_default();
            if done <= entry.last {
                continue;
            }
            entry.seconds.push(seconds as u32);
            if entry.seconds.len() > KEEP {
                entry.seconds.remove(0);
            }
            entry.last = done;
            changed = true;
        }
        if self.checks.len() > MAX_CHECKS {
            let mut by_age: Vec<(String, String)> = self.checks.iter().map(|(k, e)| (e.last.clone(), k.clone())).collect();
            by_age.sort();
            for (_, k) in by_age.into_iter().take(self.checks.len() - MAX_CHECKS) {
                self.checks.remove(&k);
            }
        }
        changed
    }

    /// The middle of the last few passes, in seconds.
    pub fn estimate(&self, repo: &str, check: &Check) -> Option<i64> {
        let mut seconds = self.checks.get(&key(repo, check))?.seconds.clone();
        if seconds.is_empty() {
            return None;
        }
        seconds.sort_unstable();
        Some(i64::from(seconds[seconds.len() / 2]))
    }

    pub fn load(path: &Path) -> Durations {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("tmp");
        if serde_json::to_vec(self).ok().and_then(|bytes| std::fs::write(&tmp, bytes).ok()).is_some() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}

/// `~4m`, `~45s` or `~1h 20m`: a rounded figure for an estimate.
pub fn approx(seconds: i64) -> String {
    match seconds {
        i64::MIN..=59 => format!("~{}s", ((seconds + 2) / 5 * 5).max(5)),
        60..=3599 => format!("~{}m", (seconds + 30) / 60),
        _ => format!("~{}h {:02}m", seconds / 3600, (seconds % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prs::{Feedback, Review};

    fn check(name: &str, state: CheckState, start: &str, end: &str) -> Check {
        Check {
            name: name.into(),
            workflow: Some("CI".into()),
            state,
            started_at: Some(start.into()),
            completed_at: (!end.is_empty()).then(|| end.into()),
            url: None,
            external: false,
        }
    }

    fn pr(repo: &str, checks: Vec<Check>) -> PullRequest {
        PullRequest {
            repo: repo.into(),
            number: 1,
            title: "t".into(),
            url: format!("https://github.com/{repo}/pull/1"),
            is_draft: false,
            labels: vec![],
            review: Review::None,
            comments: 0,
            updated_at: String::new(),
            checks,
            feedback: Feedback::default(),
        }
    }

    #[test]
    fn a_passed_check_teaches_its_duration_once_and_the_estimate_is_the_middle_of_the_last_few() {
        let mut d = Durations::default();
        let run = |start: &str, end: &str| pr("acme/api", vec![check("build", CheckState::Passed, start, end)]);
        assert!(d.learn(&[run("2026-10-09T01:00:00Z", "2026-10-09T01:04:00Z")]));
        assert!(!d.learn(&[run("2026-10-09T01:00:00Z", "2026-10-09T01:04:00Z")]), "the same run is not counted twice");
        d.learn(&[run("2026-10-09T02:00:00Z", "2026-10-09T02:02:00Z")]);
        d.learn(&[run("2026-10-09T03:00:00Z", "2026-10-09T03:05:00Z")]);
        let build = check("build", CheckState::Running, "2026-10-09T04:00:00Z", "");
        assert_eq!(d.estimate("acme/api", &build), Some(240), "the middle of 2m, 4m and 5m");
        assert_eq!(d.estimate("acme/web", &build), None, "another repo has its own history");
        assert_eq!(d.estimate("acme/api", &check("lint", CheckState::Running, "x", "")), None);
    }

    #[test]
    fn failed_skipped_instant_and_endless_runs_teach_nothing() {
        let mut d = Durations::default();
        let p = pr(
            "acme/api",
            vec![
                check("failed", CheckState::Failed, "2026-10-09T01:00:00Z", "2026-10-09T01:00:30Z"),
                check("skipped", CheckState::Skipped, "2026-10-09T01:00:00Z", "2026-10-09T01:00:01Z"),
                check("instant", CheckState::Passed, "2026-10-09T01:00:00Z", "2026-10-09T01:00:02Z"),
                check("endless", CheckState::Passed, "2026-10-09T01:00:00Z", "2026-10-10T01:00:00Z"),
                check("running", CheckState::Running, "2026-10-09T01:00:00Z", ""),
            ],
        );
        assert!(!d.learn(&[p]));
    }

    #[test]
    fn only_the_last_few_runs_are_kept_and_old_checks_are_forgotten_past_the_limit() {
        let mut d = Durations::default();
        for minute in 1..=8 {
            let end = format!("2026-10-09T0{minute}:10:00Z");
            let start = format!("2026-10-09T0{minute}:00:00Z");
            d.learn(&[pr("acme/api", vec![check("build", CheckState::Passed, &start, &end)])]);
        }
        assert_eq!(d.checks["acme/api|CI / build"].seconds.len(), KEEP);
        for n in 0..(MAX_CHECKS + 10) {
            let name = format!("check{n:03}");
            let end = format!("2026-10-10T00:{:02}:{:02}Z", n / 60 % 60, n % 60);
            d.learn(&[pr("acme/api", vec![check(&name, CheckState::Passed, "2026-10-10T00:00:00Z", &end)])]);
        }
        assert!(d.checks.len() <= MAX_CHECKS, "{}", d.checks.len());
    }

    #[test]
    fn durations_survive_a_restart_and_a_damaged_file_means_a_fresh_start() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sub/durations.json");
        let mut d = Durations::default();
        d.learn(&[pr("acme/api", vec![check("build", CheckState::Passed, "2026-10-09T01:00:00Z", "2026-10-09T01:03:00Z")])]);
        d.save(&file);
        assert_eq!(Durations::load(&file), d);
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(Durations::load(&file), Durations::default());
        assert_eq!(Durations::load(&dir.path().join("missing")), Durations::default());
    }

    #[test]
    fn estimates_read_as_rounded_figures() {
        assert_eq!(approx(240), "~4m");
        assert_eq!(approx(95), "~2m");
        assert_eq!(approx(45), "~45s");
        assert_eq!(approx(7), "~5s");
        assert_eq!(approx(4800), "~1h 20m");
    }
}
