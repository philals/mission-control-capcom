use crate::model::{Board, PrState, Status};
use crate::rules;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

pub trait PrLookup {
    fn state(&self, url: &str) -> Result<PrState>;
}

const GH_TIMEOUT: Duration = Duration::from_secs(30);

pub struct GhLookup;

pub fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<Option<Output>> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take().context("no stdout")?;
    let mut stderr = child.stderr.take().context("no stderr")?;
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    Ok(Some(Output {
        status,
        stdout: out_reader.join().unwrap_or_default(),
        stderr: err_reader.join().unwrap_or_default(),
    }))
}

impl PrLookup for GhLookup {
    fn state(&self, url: &str) -> Result<PrState> {
        if !url.starts_with("https://") {
            bail!("not an https PR url: {url}");
        }
        // the REST API has its own budget, so these checks never spend the GraphQL one
        if let Some(path) = rest_path(url) {
            let body = crate::github::rest(&path).with_context(|| format!("looking up {url}"))?;
            return parse_rest_state(&body);
        }
        let mut cmd = Command::new("gh");
        cmd.args(["pr", "view", url, "--json", "state,isDraft"]);
        let Some(out) = run_with_timeout(cmd, GH_TIMEOUT).context("running gh")? else {
            bail!("gh timed out after {}s for {url}", GH_TIMEOUT.as_secs());
        };
        if !out.status.success() {
            bail!("gh failed for {url}: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        parse_gh(&String::from_utf8_lossy(&out.stdout))
    }
}

/// The state of a pull request from the REST API's `GET /repos/{owner}/{repo}/pulls/{n}`.
pub fn parse_rest_state(json: &str) -> Result<PrState> {
    let v: serde_json::Value = serde_json::from_str(json).context("parsing the GitHub answer")?;
    let Some(state) = v.get("state").and_then(|s| s.as_str()) else {
        bail!("{}", v.get("message").and_then(|m| m.as_str()).unwrap_or("unexpected answer from GitHub"));
    };
    let draft = v.get("draft").and_then(|d| d.as_bool()).unwrap_or(false);
    let merged = v.get("merged").and_then(|m| m.as_bool()).unwrap_or(false) || v.get("merged_at").is_some_and(|m| !m.is_null());
    match (state, merged, draft) {
        (_, true, _) => Ok(PrState::Merged),
        ("closed", _, _) => Ok(PrState::Closed),
        ("open", _, true) => Ok(PrState::Draft),
        ("open", _, false) => Ok(PrState::Ready),
        (other, _, _) => bail!("unknown PR state {other}"),
    }
}

/// `repos/OWNER/NAME/pulls/N` for a github.com pull request link, else None.
pub fn rest_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://github.com/")?;
    let mut parts = rest.trim_end_matches('/').split('/');
    let (owner, repo, kind, number) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let name_ok = |s: &str| !matches!(s, "" | "." | "..") && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    (kind == "pull" && name_ok(owner) && name_ok(repo) && !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
        .then(|| format!("repos/{owner}/{repo}/pulls/{number}"))
}

pub fn parse_gh(json: &str) -> Result<PrState> {
    #[derive(Deserialize)]
    struct Gh {
        state: String,
        #[serde(rename = "isDraft")]
        is_draft: bool,
    }
    let g: Gh = serde_json::from_str(json).context("parsing gh output")?;
    match (g.state.as_str(), g.is_draft) {
        ("MERGED", _) => Ok(PrState::Merged),
        ("CLOSED", _) => Ok(PrState::Closed),
        ("OPEN", true) => Ok(PrState::Draft),
        ("OPEN", false) => Ok(PrState::Ready),
        (other, _) => bail!("unknown PR state {other}"),
    }
}

pub type Lookups = HashMap<String, Result<PrState, String>>;

pub fn pending_urls(board: &Board) -> Vec<String> {
    let mut urls: Vec<String> = board
        .tasks
        .iter()
        .filter(|t| !t.status.is_terminal())
        .flat_map(|t| t.prs.iter().map(|p| p.url.clone()))
        .collect();
    urls.sort();
    urls.dedup();
    urls
}

/// The PR urls recorded on one task.
pub fn task_urls(board: &Board, id: &str) -> Vec<String> {
    board.task(id).map(|t| t.prs.iter().map(|p| p.url.clone()).collect()).unwrap_or_default()
}

pub fn lookup_all(urls: &[String], lookup: &dyn PrLookup) -> Lookups {
    urls.iter()
        .map(|u| (u.clone(), lookup.state(u).map_err(|e| format!("{e:#}"))))
        .collect()
}

pub fn refresh(board: &mut Board, lookup: &dyn PrLookup) -> Vec<String> {
    let looked_up = lookup_all(&pending_urls(board), lookup);
    apply(board, &looked_up)
}

pub fn apply(board: &mut Board, looked_up: &Lookups) -> Vec<String> {
    let mut messages = Vec::new();
    for idx in 0..board.tasks.len() {
        let id = board.tasks[idx].id.clone();
        if board.tasks[idx].status.is_terminal() || board.tasks[idx].prs.is_empty() {
            continue;
        }
        let mut failed = false;
        for pr_idx in 0..board.tasks[idx].prs.len() {
            let url = board.tasks[idx].prs[pr_idx].url.clone();
            match looked_up.get(&url) {
                Some(Ok(state)) => {
                    let pr = &mut board.tasks[idx].prs[pr_idx];
                    if pr.state != *state {
                        messages.push(format!(
                            "{id}: PR {url} {} -> {}",
                            pr.state.as_str(),
                            state.as_str()
                        ));
                        pr.state = *state;
                    }
                }
                Some(Err(e)) => {
                    failed = true;
                    messages.push(format!("{id}: could not refresh {url}: {e}"));
                }
                None => failed = true,
            }
        }
        if failed || board.tasks[idx].status != Status::Implementing {
            continue;
        }
        let missing = rules::missing_summary(&board.tasks[idx]);
        if !missing.is_empty() {
            messages.push(format!("{id}: waiting for PRs in: {missing}"));
            continue;
        }
        let live = rules::live_prs(&board.tasks[idx]);
        if live.is_empty() {
            messages.push(format!("{id}: every PR is closed; leaving status implementing"));
            continue;
        }
        if live.iter().all(|p| p.state == PrState::Merged) {
            match rules::transition(board, &id, Status::Done) {
                Ok(()) => messages.push(format!("{id}: implementing -> done")),
                Err(e) => messages.push(format!("{id}: could not move to done: {e:#}")),
            }
        }
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use std::collections::HashMap;

    struct Fake(HashMap<String, Result<PrState, String>>);

    impl PrLookup for Fake {
        fn state(&self, url: &str) -> anyhow::Result<PrState> {
            match self.0.get(url) {
                Some(Ok(s)) => Ok(*s),
                Some(Err(e)) => anyhow::bail!("{e}"),
                None => anyhow::bail!("unknown url"),
            }
        }
    }

    fn task(id: &str, status: Status, url: &str, state: PrState) -> Task {
        let mut t = Task::new(id, id, TaskType::Pr, vec![], vec![], format!("tasks/{id}.md"));
        t.status = status;
        t.prs.push(Pr { repo: "r".into(), url: url.into(), state });
        t
    }

    fn board(tasks: Vec<Task>) -> Board {
        let mut b = Board::new("PROJ-1", "s", None);
        b.tasks = tasks;
        b
    }

    #[test]
    fn task_urls_lists_only_that_tasks_prs() {
        let b = board(vec![task("T1", Status::Implementing, U1, PrState::Draft), task("T2", Status::Implementing, "https://github.com/o/r/pull/2", PrState::Draft)]);
        assert_eq!(task_urls(&b, "T2"), vec!["https://github.com/o/r/pull/2".to_string()]);
        assert!(task_urls(&b, "T9").is_empty());
    }

    const U1: &str = "https://github.com/o/r/pull/1";
    const U2: &str = "https://github.com/o/r/pull/2";

    fn fake(pairs: &[(&str, Result<PrState, &str>)]) -> Fake {
        Fake(pairs
            .iter()
            .map(|(u, r)| (u.to_string(), r.clone().map_err(str::to_string)))
            .collect())
    }

    #[test]
    fn parse_gh_maps_states() {
        assert_eq!(parse_gh(r#"{"state":"OPEN","isDraft":true}"#).unwrap(), PrState::Draft);
        assert_eq!(parse_gh(r#"{"state":"OPEN","isDraft":false}"#).unwrap(), PrState::Ready);
        assert_eq!(parse_gh(r#"{"state":"MERGED","isDraft":false}"#).unwrap(), PrState::Merged);
        assert_eq!(parse_gh(r#"{"state":"CLOSED","isDraft":false}"#).unwrap(), PrState::Closed);
        assert!(parse_gh(r#"{"state":"WAT","isDraft":false}"#).is_err());
        assert!(parse_gh("not json").is_err());
    }

    #[test]
    fn ready_prs_leave_an_implementing_task_alone() {
        let mut b = board(vec![task("T1", Status::Implementing, U1, PrState::Draft)]);
        let msgs = refresh(&mut b, &fake(&[(U1, Ok(PrState::Ready))]));
        assert_eq!(b.tasks[0].status, Status::Implementing);
        assert_eq!(b.tasks[0].prs[0].state, PrState::Ready);
        assert!(msgs.iter().all(|m| !m.contains("->") || m.contains("PR ")), "{msgs:?}");
    }

    #[test]
    fn merged_prs_finish_the_task() {
        let mut b = board(vec![task("T1", Status::Implementing, U1, PrState::Ready)]);
        let msgs = refresh(&mut b, &fake(&[(U1, Ok(PrState::Merged))]));
        assert_eq!(b.tasks[0].status, Status::Done);
        assert!(msgs.iter().any(|m| m == "T1: implementing -> done"), "{msgs:?}");
    }

    #[test]
    fn a_pr_returning_to_draft_keeps_the_task_implementing() {
        let mut b = board(vec![task("T1", Status::Implementing, U1, PrState::Ready)]);
        refresh(&mut b, &fake(&[(U1, Ok(PrState::Draft))]));
        assert_eq!(b.tasks[0].status, Status::Implementing);
        assert_eq!(b.tasks[0].prs[0].state, PrState::Draft);
    }

    #[test]
    fn a_failing_lookup_leaves_that_task_alone_and_others_proceed() {
        let mut b = board(vec![
            task("T1", Status::Implementing, U1, PrState::Draft),
            task("T2", Status::Implementing, U2, PrState::Draft),
        ]);
        let msgs = refresh(&mut b, &fake(&[(U1, Ok(PrState::Ready)), (U2, Err("gh: not logged in"))]));
        assert_eq!(b.tasks[0].status, Status::Implementing);
        assert_eq!(b.tasks[0].prs[0].state, PrState::Ready);
        assert_eq!(b.tasks[1].status, Status::Implementing);
        assert_eq!(b.tasks[1].prs[0].state, PrState::Draft);
        assert!(msgs.iter().any(|m| m.contains("could not refresh") && m.contains("not logged in")));
    }

    #[test]
    fn all_closed_prs_do_not_move_the_task() {
        let mut b = board(vec![task("T1", Status::Implementing, U1, PrState::Draft)]);
        refresh(&mut b, &fake(&[(U1, Ok(PrState::Closed))]));
        assert_eq!(b.tasks[0].status, Status::Implementing);
    }

    #[test]
    fn multi_repo_task_waits_for_every_repo_before_done() {
        let mut t = task("T1", Status::Implementing, U1, PrState::Draft);
        t.repos = vec!["r".into(), "ui".into()];
        let mut b = board(vec![t]);
        let msgs = refresh(&mut b, &fake(&[(U1, Ok(PrState::Merged))]));
        assert_eq!(b.tasks[0].status, Status::Implementing);
        assert_eq!(b.tasks[0].prs[0].state, PrState::Merged);
        assert!(msgs.iter().any(|m| m == "T1: waiting for PRs in: ui"), "{msgs:?}");

        b.tasks[0].prs.push(Pr { repo: "ui".into(), url: U2.into(), state: PrState::Merged });
        refresh(&mut b, &fake(&[(U1, Ok(PrState::Merged)), (U2, Ok(PrState::Merged))]));
        assert_eq!(b.tasks[0].status, Status::Done);
    }

    #[test]
    fn apply_ignores_prs_missing_from_the_snapshot() {
        let mut b = board(vec![task("T1", Status::Implementing, U1, PrState::Draft)]);
        let msgs = apply(&mut b, &Lookups::new());
        assert!(msgs.is_empty());
        assert_eq!(b.tasks[0].status, Status::Implementing);
    }

    #[test]
    fn run_with_timeout_kills_slow_commands_and_returns_fast_output() {
        let mut slow = Command::new("sleep");
        slow.arg("5");
        let start = Instant::now();
        assert!(run_with_timeout(slow, Duration::from_millis(100)).unwrap().is_none());
        assert!(start.elapsed() < Duration::from_secs(3));

        let mut fast = Command::new("echo");
        fast.arg("hi");
        let out = run_with_timeout(fast, Duration::from_secs(5)).unwrap().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    }

    #[test]
    fn the_rest_answer_gives_draft_ready_merged_or_closed() {
        let state = |json: &str| parse_rest_state(json).unwrap();
        assert_eq!(state(r#"{"state":"open","draft":true,"merged":false}"#), PrState::Draft);
        assert_eq!(state(r#"{"state":"open","draft":false,"merged":false}"#), PrState::Ready);
        assert_eq!(state(r#"{"state":"open"}"#), PrState::Ready, "a missing draft flag is ready");
        assert_eq!(state(r#"{"state":"closed","draft":false,"merged":true,"merged_at":"2026-10-09T01:00:00Z"}"#), PrState::Merged);
        assert_eq!(state(r#"{"state":"closed","draft":false,"merged":false,"merged_at":null}"#), PrState::Closed);
        assert_eq!(state(r#"{"state":"closed","merged_at":"2026-10-09T01:00:00Z"}"#), PrState::Merged);
        let err = parse_rest_state(r#"{"message":"Not Found"}"#).unwrap_err().to_string();
        assert_eq!(err, "Not Found");
        assert!(parse_rest_state("nope").is_err());
    }

    #[test]
    fn a_pull_request_link_becomes_a_rest_path_and_anything_odd_does_not() {
        assert_eq!(rest_path("https://github.com/acme/api/pull/12").as_deref(), Some("repos/acme/api/pulls/12"));
        assert_eq!(rest_path("https://github.com/acme/api/pull/12/").as_deref(), Some("repos/acme/api/pulls/12"));
        assert_eq!(rest_path("https://github.com/acme/.github/pull/3").as_deref(), Some("repos/acme/.github/pulls/3"));
        for bad in [
            "https://github.com/acme/api/issues/12",
            "https://github.com/acme/api/pull/x",
            "https://github.com/acme/ap i/pull/1",
            "https://example.com/acme/api/pull/1",
            "https://github.com/../api/pull/1",
            "https://github.com/acme/../pull/1",
            "--help",
        ] {
            assert_eq!(rest_path(bad), None, "{bad}");
        }
    }

    #[test]
    fn gh_lookup_refuses_non_https_urls() {
        assert!(GhLookup.state("--help").is_err());
        assert!(GhLookup.state("file:///etc/passwd").is_err());
    }

    #[test]
    fn error_causes_are_preserved_in_messages() {
        struct ErrorLookup;
        impl PrLookup for ErrorLookup {
            fn state(&self, _url: &str) -> anyhow::Result<PrState> {
                Err(anyhow::anyhow!("root cause").context("outer context"))
            }
        }
        let mut b = board(vec![task("T1", Status::Implementing, U1, PrState::Draft)]);
        let msgs = refresh(&mut b, &ErrorLookup);
        let msg = msgs.iter().find(|m| m.contains("could not refresh")).expect("should have error message");
        assert!(msg.contains("outer context"));
        assert!(msg.contains("root cause"));
    }

    #[test]
    fn partial_failure_within_one_task_updates_successful_prs() {
        let const_u3: &str = "https://github.com/o/r/pull/3";
        let mut t = Task::new("T1", "T1", TaskType::Pr, vec![], vec![], "tasks/T1.md".into());
        t.status = Status::Implementing;
        t.prs.push(Pr { repo: "r".into(), url: U1.into(), state: PrState::Draft });
        t.prs.push(Pr { repo: "r".into(), url: const_u3.into(), state: PrState::Draft });

        let mut b = board(vec![t]);
        let msgs = refresh(&mut b, &fake(&[(U1, Ok(PrState::Ready)), (const_u3, Err("network error"))]));

        assert_eq!(b.tasks[0].prs[0].state, PrState::Ready);
        assert_eq!(b.tasks[0].prs[1].state, PrState::Draft);
        assert_eq!(b.tasks[0].status, Status::Implementing);
        assert!(msgs.iter().any(|m| m.contains("PR") && m.contains("draft") && m.contains("ready")));
        assert!(msgs.iter().any(|m| m.contains("could not refresh") && m.contains("network error")));
    }

    #[test]
    fn all_closed_prs_persist_state_and_log_message() {
        let mut b = board(vec![task("T1", Status::Implementing, U1, PrState::Draft)]);
        let msgs = refresh(&mut b, &fake(&[(U1, Ok(PrState::Closed))]));

        assert_eq!(b.tasks[0].prs[0].state, PrState::Closed);
        assert_eq!(b.tasks[0].status, Status::Implementing);
        assert!(msgs.iter().any(|m| m.contains("every PR is closed")));
    }

    #[test]
    fn planned_tasks_with_prs_refresh_state_but_do_not_transition() {
        let mut b = board(vec![task("T1", Status::Planned, U1, PrState::Draft)]);
        refresh(&mut b, &fake(&[(U1, Ok(PrState::Ready))]));

        assert_eq!(b.tasks[0].prs[0].state, PrState::Ready);
        assert_eq!(b.tasks[0].status, Status::Planned);
    }

    #[test]
    fn todo_tasks_with_prs_refresh_state_but_do_not_transition() {
        let mut b = board(vec![task("T1", Status::Todo, U1, PrState::Draft)]);
        refresh(&mut b, &fake(&[(U1, Ok(PrState::Ready))]));

        assert_eq!(b.tasks[0].prs[0].state, PrState::Ready);
        assert_eq!(b.tasks[0].status, Status::Todo);
    }
}
