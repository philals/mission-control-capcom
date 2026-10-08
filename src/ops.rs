use crate::model::{Agent, Blocked, Board, Pr, PrState, StoryStatus, Task, TaskType};
use crate::{rules, store};
use anyhow::{bail, Context, Result};
use std::path::Path;

pub fn init_story(root: &Path, key: &str, title: &str, jira_url: Option<&str>) -> Result<()> {
    let dir = store::story_dir(root, key)?;
    if dir.join("board.json").exists() {
        bail!("story {key} already exists");
    }
    std::fs::create_dir_all(dir.join("tasks"))?;
    // Write story.md before saving board.json so a failed write doesn't block retries
    let story_md = dir.join("story.md");
    if !story_md.exists() {
        std::fs::write(
            story_md,
            format!("# {key}: {title}\n\n## Summary\n\n## Acceptance criteria\n\n## Breakdown notes\n"),
        )?;
    }
    store::save(root, key, &Board::new(key, title, jira_url))?;
    Ok(())
}

pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let trimmed: String = out.trim_end_matches('-').chars().take(40).collect();
    let trimmed = trimmed.trim_end_matches('-').to_string();
    if trimmed.is_empty() {
        "task".to_string()
    } else {
        trimmed
    }
}

fn task_mut<'a>(board: &'a mut Board, id: &str) -> Result<&'a mut Task> {
    board
        .tasks
        .iter_mut()
        .find(|t| t.id == id)
        .with_context(|| format!("no task {id}"))
}

fn editable<'a>(board: &'a mut Board, id: &str) -> Result<&'a mut Task> {
    let t = task_mut(board, id)?;
    if t.status.is_terminal() {
        bail!("{id} is {} and cannot be edited", t.status.as_str());
    }
    Ok(t)
}

pub fn add_task(
    story_dir: &Path,
    board: &mut Board,
    title: &str,
    kind: TaskType,
    depends: Vec<String>,
    repos: Vec<String>,
) -> Result<String> {
    if title.trim().is_empty() {
        bail!("task title must not be empty");
    }
    let story_status = board.story.status;
    if story_status == StoryStatus::Done {
        bail!("story is done; reopen it with `capcom story-status <KEY> in_progress` before adding tasks");
    }
    let next = board
        .tasks
        .iter()
        .filter_map(|t| t.id.strip_prefix('T')?.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    let id = format!("T{next}");
    let file = format!("tasks/{id}-{}.md", slug(title));
    board
        .tasks
        .push(Task::new(&id, title.trim(), kind, depends, repos, file.clone()));
    if let Err(e) = rules::check(board) {
        board.tasks.pop();
        return Err(e);
    }
    let path = story_dir.join(&file);
    if let Err(e) = std::fs::create_dir_all(path.parent().unwrap()) {
        board.tasks.pop();
        return Err(e.into());
    }
    board.story.status = StoryStatus::InProgress;
    if !path.exists() {
        if let Err(e) = std::fs::write(
            &path,
            format!(
                "# {id}: {}\n\nType: {}\n\n## Description\n\n## Plan\n\n## Decisions\n\n## Progress\n",
                title.trim(),
                kind.as_str()
            ),
        ) {
            board.tasks.pop();
            board.story.status = story_status;
            let _ = std::fs::remove_file(&path);
            return Err(e.into());
        }
    }
    Ok(id)
}

pub fn set_deps(board: &mut Board, id: &str, deps: Vec<String>) -> Result<()> {
    let old = editable(board, id)?.depends_on.clone();
    task_mut(board, id)?.depends_on = deps;
    if let Err(e) = rules::check(board) {
        task_mut(board, id)?.depends_on = old;
        return Err(e);
    }
    Ok(())
}

pub fn set_repos(board: &mut Board, id: &str, repos: Vec<String>) -> Result<()> {
    editable(board, id)?.repos = repos;
    Ok(())
}

pub fn block(board: &mut Board, id: &str, reason: &str) -> Result<()> {
    if reason.trim().is_empty() {
        bail!("a blocked reason is required");
    }
    editable(board, id)?.blocked = Some(Blocked { reason: reason.trim().to_string() });
    Ok(())
}

pub fn unblock(board: &mut Board, id: &str) -> Result<()> {
    editable(board, id)?.blocked = None;
    Ok(())
}

pub fn add_pr(board: &mut Board, id: &str, repo: &str, url: &str, state: PrState) -> Result<()> {
    let t = editable(board, id)?;
    if t.prs.iter().any(|p| p.url == url) {
        bail!("{id} already has a PR with url {url}");
    }
    t.prs.push(Pr { repo: repo.to_string(), url: url.to_string(), state });
    Ok(())
}

pub fn set_pr_state(board: &mut Board, id: &str, url: &str, state: PrState) -> Result<()> {
    let t = editable(board, id)?;
    let pr = t
        .prs
        .iter_mut()
        .find(|p| p.url == url)
        .with_context(|| format!("{id} has no PR with url {url}"))?;
    pr.state = state;
    Ok(())
}

pub fn set_agent(board: &mut Board, id: &str, pane: &str, skill: &str, started_at: &str) -> Result<()> {
    editable(board, id)?.agent = Some(Agent {
        pane: pane.to_string(),
        skill: skill.to_string(),
        started_at: started_at.to_string(),
    });
    Ok(())
}

pub fn clear_agent(board: &mut Board, id: &str) -> Result<()> {
    task_mut(board, id)?.agent = None;
    Ok(())
}

pub fn set_subtask(board: &mut Board, id: &str, jira_key: &str) -> Result<()> {
    if !store::valid_key(jira_key) {
        bail!("invalid Jira key {jira_key:?}");
    }
    editable(board, id)?.jira_subtask = Some(jira_key.to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use tempfile::TempDir;

    fn fresh() -> (TempDir, Board) {
        let root = TempDir::new().unwrap();
        init_story(root.path(), "PROJ-1", "A story", None).unwrap();
        let board = crate::store::load(root.path(), "PROJ-1").unwrap();
        (root, board)
    }

    fn dir(root: &TempDir) -> std::path::PathBuf {
        root.path().join("PROJ-1")
    }

    #[test]
    fn init_story_creates_board_story_file_and_tasks_dir() {
        let (root, board) = fresh();
        assert!(board.tasks.is_empty());
        assert!(dir(&root).join("story.md").exists());
        assert!(dir(&root).join("tasks").is_dir());
    }

    #[test]
    fn init_story_refuses_to_overwrite() {
        let (root, _) = fresh();
        assert!(init_story(root.path(), "PROJ-1", "again", None).is_err());
    }

    #[test]
    fn slug_is_filesystem_safe() {
        assert_eq!(slug("Add the /notices endpoint!"), "add-the-notices-endpoint");
        assert_eq!(slug("!!!"), "task");
        assert!(slug(&"x".repeat(100)).len() <= 40);
    }

    #[test]
    fn add_task_assigns_sequential_ids_and_writes_a_stub() {
        let (root, mut board) = fresh();
        let a = add_task(&dir(&root), &mut board, "Add endpoint", TaskType::Pr, vec![], vec!["api".into()]).unwrap();
        let b = add_task(&dir(&root), &mut board, "Wire UI", TaskType::Pr, vec![a.clone()], vec![]).unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("T1", "T2"));
        let stub = std::fs::read_to_string(dir(&root).join("tasks/T1-add-endpoint.md")).unwrap();
        assert!(stub.starts_with("# T1: Add endpoint"));
        assert!(stub.contains("## Decisions"));
        assert!(!stub.contains("Depends on"));
    }

    #[test]
    fn task_ids_are_never_reused_after_a_drop() {
        let (root, mut board) = fresh();
        let a = add_task(&dir(&root), &mut board, "One", TaskType::Pr, vec![], vec![]).unwrap();
        crate::rules::transition(&mut board, &a, Status::Dropped).unwrap();
        let b = add_task(&dir(&root), &mut board, "Two", TaskType::Pr, vec![], vec![]).unwrap();
        assert_eq!(b, "T2");
    }

    #[test]
    fn add_task_with_unknown_dependency_is_rejected_and_leaves_no_trace() {
        let (root, mut board) = fresh();
        let err = add_task(&dir(&root), &mut board, "One", TaskType::Pr, vec!["T9".into()], vec![]).unwrap_err();
        assert!(err.to_string().contains("unknown task T9"));
        assert!(board.tasks.is_empty());
        assert_eq!(std::fs::read_dir(dir(&root).join("tasks")).unwrap().count(), 0);
    }

    #[test]
    fn set_deps_rolls_back_on_a_cycle() {
        let (root, mut board) = fresh();
        let t0 = add_task(&dir(&root), &mut board, "Zero", TaskType::Pr, vec![], vec![]).unwrap();
        let t1 = add_task(&dir(&root), &mut board, "One", TaskType::Pr, vec![t0.clone()], vec![]).unwrap();
        let t2 = add_task(&dir(&root), &mut board, "Two", TaskType::Pr, vec![t1.clone()], vec![]).unwrap();
        assert!(set_deps(&mut board, &t1, vec![t2.clone()]).is_err());
        assert_eq!(board.task(&t1).unwrap().depends_on, vec![t0.clone()]);
    }

    #[test]
    fn add_task_with_self_dependency_is_rejected_and_leaves_no_trace() {
        let (root, mut board) = fresh();
        let err = add_task(&dir(&root), &mut board, "One", TaskType::Pr, vec!["T1".into()], vec![]).unwrap_err();
        assert!(err.to_string().contains("depends on itself"));
        assert!(board.tasks.is_empty());
        assert_eq!(std::fs::read_dir(dir(&root).join("tasks")).unwrap().count(), 0);
    }

    #[test]
    fn set_deps_refuses_self_dependency() {
        let (root, mut board) = fresh();
        let t1 = add_task(&dir(&root), &mut board, "One", TaskType::Pr, vec![], vec![]).unwrap();
        let err = set_deps(&mut board, &t1, vec![t1.clone()]).unwrap_err();
        assert!(err.to_string().contains("depends on itself"));
        assert!(board.task(&t1).unwrap().depends_on.is_empty());
    }

    #[test]
    fn set_deps_refuses_unknown_dependency() {
        let (root, mut board) = fresh();
        let t1 = add_task(&dir(&root), &mut board, "One", TaskType::Pr, vec![], vec![]).unwrap();
        let err = set_deps(&mut board, &t1, vec!["T9".into()]).unwrap_err();
        assert!(err.to_string().contains("unknown task T9"));
        assert!(board.task(&t1).unwrap().depends_on.is_empty());
    }

    #[test]
    fn set_deps_refuses_unknown_task() {
        let (_root, mut board) = fresh();
        let err = set_deps(&mut board, "T99", vec![]).unwrap_err();
        assert!(err.to_string().contains("no task T99"));
    }

    #[test]
    fn add_task_with_io_failure_leaves_no_trace() {
        let (root, mut board) = fresh();
        // a file in place of the dir makes create_dir_all fail
        let tasks_path = dir(&root).join("tasks");
        std::fs::remove_dir(tasks_path.clone()).unwrap();
        std::fs::write(&tasks_path, "not a directory").unwrap();

        let err = add_task(&dir(&root), &mut board, "One", TaskType::Pr, vec![], vec![]).unwrap_err();
        assert!(!err.to_string().is_empty());
        assert!(board.tasks.is_empty());
    }

    #[test]
    fn block_and_unblock() {
        let (root, mut board) = fresh();
        add_task(&dir(&root), &mut board, "One", TaskType::Pr, vec![], vec![]).unwrap();
        assert!(block(&mut board, "T1", "  ").is_err());
        block(&mut board, "T1", "needs schema change").unwrap();
        assert_eq!(board.tasks[0].blocked.as_ref().unwrap().reason, "needs schema change");
        unblock(&mut board, "T1").unwrap();
        assert!(board.tasks[0].blocked.is_none());
    }

    #[test]
    fn prs_agent_and_subtask_edits() {
        let (root, mut board) = fresh();
        add_task(&dir(&root), &mut board, "One", TaskType::Pr, vec![], vec![]).unwrap();
        add_pr(&mut board, "T1", "api", "https://github.com/o/api/pull/5", PrState::Draft).unwrap();
        assert!(add_pr(&mut board, "T1", "api", "https://github.com/o/api/pull/5", PrState::Draft).is_err());
        set_pr_state(&mut board, "T1", "https://github.com/o/api/pull/5", PrState::Ready).unwrap();
        assert_eq!(board.tasks[0].prs[0].state, PrState::Ready);
        assert!(set_pr_state(&mut board, "T1", "https://nope", PrState::Ready).is_err());
        set_agent(&mut board, "T1", "w1:p2", "story-plan-task", "2026-10-02T00:00:00Z").unwrap();
        assert_eq!(board.tasks[0].agent.as_ref().unwrap().pane, "w1:p2");
        clear_agent(&mut board, "T1").unwrap();
        assert!(board.tasks[0].agent.is_none());
        set_subtask(&mut board, "T1", "PROJ-9").unwrap();
        assert_eq!(board.tasks[0].jira_subtask.as_deref(), Some("PROJ-9"));
        assert!(set_subtask(&mut board, "T1", "not a key").is_err());
    }

    #[test]
    fn terminal_tasks_cannot_be_edited() {
        let (root, mut board) = fresh();
        add_task(&dir(&root), &mut board, "One", TaskType::Pr, vec![], vec![]).unwrap();
        crate::rules::transition(&mut board, "T1", Status::Dropped).unwrap();
        assert!(block(&mut board, "T1", "x").is_err());
        assert!(add_pr(&mut board, "T1", "api", "https://github.com/o/api/pull/1", PrState::Draft).is_err());
    }
    #[test]
    fn add_task_reopens_a_story_in_review_and_refuses_a_done_story() {
        let (root, mut board) = fresh();
        add_task(&dir(&root), &mut board, "One", TaskType::Spike, vec![], vec![]).unwrap();
        board.tasks[0].status = Status::Done;
        board.story.status = StoryStatus::InReview;
        add_task(&dir(&root), &mut board, "Two", TaskType::Pr, vec![], vec![]).unwrap();
        assert_eq!(board.story.status, StoryStatus::InProgress);

        board.story.status = StoryStatus::Done;
        let err = add_task(&dir(&root), &mut board, "Three", TaskType::Pr, vec![], vec![]).unwrap_err();
        assert!(err.to_string().contains("story is done"), "{err}");
        assert_eq!(board.tasks.len(), 2);
        assert_eq!(board.story.status, StoryStatus::Done);
        assert!(!dir(&root).join("tasks/T3-three.md").exists());
    }
}
