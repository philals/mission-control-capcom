use crate::model::{Board, Pr, PrState, Status, StoryStatus, Task, TaskType};
use anyhow::{bail, Context, Result};
use std::collections::{HashMap, HashSet};

pub fn check(board: &Board) -> Result<()> {
    let mut ids: HashSet<&str> = HashSet::new();
    for t in &board.tasks {
        if !ids.insert(t.id.as_str()) {
            bail!("duplicate task id {}", t.id);
        }
    }
    for t in &board.tasks {
        for d in &t.depends_on {
            if d == &t.id {
                bail!("{} depends on itself", t.id);
            }
            if !ids.contains(d.as_str()) {
                bail!("{} depends on unknown task {}", t.id, d);
            }
        }
    }
    let mut remaining: HashMap<&str, usize> = board
        .tasks
        .iter()
        .map(|t| (t.id.as_str(), t.depends_on.iter().collect::<HashSet<_>>().len()))
        .collect();
    loop {
        let free: Vec<&str> = remaining
            .iter()
            .filter(|(_, n)| **n == 0)
            .map(|(id, _)| *id)
            .collect();
        if free.is_empty() {
            break;
        }
        for id in free {
            remaining.remove(id);
            for t in &board.tasks {
                if t.depends_on.iter().any(|d| d == id) {
                    if let Some(n) = remaining.get_mut(t.id.as_str()) {
                        *n -= 1;
                    }
                }
            }
        }
    }
    if !remaining.is_empty() {
        let mut stuck: Vec<&str> = remaining.keys().copied().collect();
        stuck.sort();
        bail!("dependency cycle among: {}", stuck.join(", "));
    }
    Ok(())
}

pub fn is_ready(board: &Board, task: &Task) -> bool {
    task.depends_on
        .iter()
        .all(|d| board.task(d).map_or(false, |x| x.status == Status::Done))
}

pub fn ready_ids(board: &Board) -> Vec<String> {
    board
        .tasks
        .iter()
        .filter(|t| !t.status.is_terminal() && is_ready(board, t))
        .map(|t| t.id.clone())
        .collect()
}

pub fn live_prs(task: &Task) -> Vec<&Pr> {
    task.prs.iter().filter(|p| p.state != PrState::Closed).collect()
}

pub fn missing_repos(task: &Task) -> Vec<&str> {
    if task.kind != TaskType::Pr {
        return Vec::new();
    }
    task.repos
        .iter()
        .filter(|r| !task.prs.iter().any(|p| &p.repo == *r && p.state != PrState::Closed))
        .map(String::as_str)
        .collect()
}

fn allowed(from: Status, to: Status) -> bool {
    use Status::*;
    matches!(
        (from, to),
        (Todo, Planning)
            | (Todo, Implementing)
            | (Planning, Planned)
            | (Planning, Todo)
            | (Planned, Implementing)
            | (Planned, Planning)
            | (Implementing, Done)
            | (Implementing, Planned)
            | (Todo | Planning | Planned | Implementing, Dropped)
    )
}

pub fn transition(board: &mut Board, id: &str, to: Status) -> Result<()> {
    let idx = board
        .tasks
        .iter()
        .position(|t| t.id == id)
        .with_context(|| format!("no task {id}"))?;
    let from = board.tasks[idx].status;
    if !allowed(from, to) {
        bail!("illegal transition for {id}: {} -> {}", from.as_str(), to.as_str());
    }
    {
        let t = &board.tasks[idx];
        match to {
            Status::Implementing if matches!(from, Status::Planned | Status::Todo) => {
                if let Some(b) = &t.blocked {
                    bail!("{id} is blocked: {}", b.reason);
                }
                let waiting: Vec<&str> = t
                    .depends_on
                    .iter()
                    .filter(|d| board.task(d).map_or(true, |x| x.status != Status::Done))
                    .map(String::as_str)
                    .collect();
                if !waiting.is_empty() {
                    bail!("{id} is waiting on unfinished dependencies: {}", waiting.join(", "));
                }
            }
            Status::Done if !missing_repos(t).is_empty() => {
                bail!("{id} is waiting for PRs in: {}", missing_repos(t).join(", "));
            }
            Status::Done => {
                let live = live_prs(t);
                if live.iter().any(|p| p.state != PrState::Merged) {
                    bail!("{id} has PRs that are not merged");
                }
                if t.kind == TaskType::Pr && !live.iter().any(|p| p.state == PrState::Merged) {
                    bail!("{id} is a pr task with no merged PR");
                }
            }
            Status::Dropped => {
                let dependents: Vec<String> = board
                    .tasks
                    .iter()
                    .filter(|o| {
                        o.id != id
                            && !o.status.is_terminal()
                            && o.depends_on.iter().any(|d| d == id)
                    })
                    .map(|o| format!("{} depends on {id}", o.id))
                    .collect();
                if !dependents.is_empty() {
                    bail!("cannot drop {id}: {} (drop or re-point them first)", dependents.join("; "));
                }
            }
            _ => {}
        }
    }
    let t = &mut board.tasks[idx];
    t.status = to;
    if matches!(
        to,
        Status::Todo | Status::Planned | Status::Done | Status::Dropped
    ) {
        t.agent = None;
    }
    Ok(())
}

pub fn story_transition(board: &mut Board, to: StoryStatus) -> Result<()> {
    use StoryStatus::*;
    let from = board.story.status;
    if from == to {
        bail!("story is already {}", to.as_str());
    }
    if !matches!((from, to), (InProgress, InReview) | (InReview, Done) | (InReview, InProgress) | (Done, InProgress)) {
        bail!("illegal story transition: {} -> {}", from.as_str(), to.as_str());
    }
    if matches!(to, InReview | Done) {
        let open: Vec<String> = board
            .tasks
            .iter()
            .filter(|t| !matches!(t.status, Status::Done | Status::Dropped))
            .map(|t| format!("{} is {}", t.id, t.status.as_str()))
            .collect();
        if !open.is_empty() {
            bail!("story has unfinished tasks: {}", open.join(", "));
        }
        if !board.tasks.iter().any(|t| t.status == Status::Done) {
            bail!("story has no tasks that are done");
        }
    }
    board.story.status = to;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn t(id: &str, deps: &[&str]) -> Task {
        Task::new(
            id,
            id,
            TaskType::Pr,
            deps.iter().map(|d| d.to_string()).collect(),
            vec![],
            format!("tasks/{id}.md"),
        )
    }

    fn board(tasks: Vec<Task>) -> Board {
        let mut b = Board::new("PROJ-1", "s", None);
        b.tasks = tasks;
        b
    }

    fn pr(state: PrState) -> Pr {
        Pr { repo: "r".into(), url: "https://github.com/o/r/pull/1".into(), state }
    }

    fn set(b: &mut Board, id: &str, status: Status) {
        b.tasks.iter_mut().find(|t| t.id == id).unwrap().status = status;
    }

    fn prs(b: &mut Board, id: &str, prs: Vec<Pr>) {
        b.tasks.iter_mut().find(|t| t.id == id).unwrap().prs = prs;
    }

    fn err(r: anyhow::Result<()>) -> String {
        r.unwrap_err().to_string()
    }

    #[test]
    fn check_rejects_duplicate_ids() {
        assert!(err(check(&board(vec![t("T1", &[]), t("T1", &[])]))).contains("duplicate"));
    }

    #[test]
    fn check_rejects_unknown_dependency() {
        assert!(err(check(&board(vec![t("T1", &["T9"])]))).contains("unknown task T9"));
    }

    #[test]
    fn check_rejects_self_dependency() {
        assert!(err(check(&board(vec![t("T1", &["T1"])]))).contains("itself"));
    }

    #[test]
    fn check_rejects_cycles() {
        let b = board(vec![t("T1", &["T2"]), t("T2", &["T1"]), t("T3", &[])]);
        assert!(err(check(&b)).contains("cycle"));
    }

    #[test]
    fn check_accepts_a_valid_graph() {
        assert!(check(&board(vec![t("T1", &[]), t("T2", &["T1"]), t("T3", &["T1", "T2"])])).is_ok());
    }

    #[test]
    fn ready_ids_need_all_dependencies_done() {
        let mut b = board(vec![t("T1", &[]), t("T2", &["T1"]), t("T3", &["T2"])]);
        assert_eq!(ready_ids(&b), vec!["T1"]);
        set(&mut b, "T1", Status::Done);
        assert_eq!(ready_ids(&b), vec!["T2"]);
    }

    #[test]
    fn happy_path_to_implementing() {
        let mut b = board(vec![t("T1", &[])]);
        transition(&mut b, "T1", Status::Planning).unwrap();
        transition(&mut b, "T1", Status::Planned).unwrap();
        transition(&mut b, "T1", Status::Implementing).unwrap();
        assert_eq!(b.tasks[0].status, Status::Implementing);
    }

    #[test]
    fn a_task_may_skip_planning_and_go_straight_from_todo_to_implementing() {
        let mut b = board(vec![t("T1", &[]), t("T2", &["T1"]), t("T3", &[])]);
        transition(&mut b, "T1", Status::Implementing).unwrap();
        assert_eq!(b.tasks[0].status, Status::Implementing);
        assert!(err(transition(&mut b, "T2", Status::Implementing)).contains("waiting on"), "dependencies still count");
        b.tasks[2].blocked = Some(Blocked { reason: "waiting on DBA".into() });
        assert!(err(transition(&mut b, "T3", Status::Implementing)).contains("blocked"));
        assert_eq!(b.tasks[1].status, Status::Todo);
    }

    #[test]
    fn illegal_transition_is_refused() {
        let mut b = board(vec![t("T1", &[])]);
        assert!(err(transition(&mut b, "T1", Status::Done)).contains("illegal transition"));
    }

    #[test]
    fn implementing_waits_for_dependencies() {
        let mut b = board(vec![t("T1", &[]), t("T2", &["T1"])]);
        set(&mut b, "T2", Status::Planned);
        assert!(err(transition(&mut b, "T2", Status::Implementing)).contains("waiting on"));
        set(&mut b, "T1", Status::Done);
        transition(&mut b, "T2", Status::Implementing).unwrap();
    }

    #[test]
    fn implementing_refused_while_blocked() {
        let mut b = board(vec![t("T1", &[])]);
        set(&mut b, "T1", Status::Planned);
        b.tasks[0].blocked = Some(Blocked { reason: "waiting on DBA".into() });
        assert!(err(transition(&mut b, "T1", Status::Implementing)).contains("blocked"));
    }

    #[test]
    fn in_review_is_not_a_task_status() {
        assert!(Status::parse("in_review").is_none());
        let mut b = board(vec![t("T1", &[])]);
        set(&mut b, "T1", Status::Implementing);
        prs(&mut b, "T1", vec![pr(PrState::Ready)]);
        transition(&mut b, "T1", Status::Done).unwrap_err();
        assert_eq!(b.tasks[0].status, Status::Implementing);
    }

    #[test]
    fn done_needs_merged_prs_and_pr_tasks_need_a_pr() {
        let mut b = board(vec![t("T1", &[])]);
        set(&mut b, "T1", Status::Implementing);
        assert!(err(transition(&mut b, "T1", Status::Done)).contains("no merged PR"));
        prs(&mut b, "T1", vec![pr(PrState::Ready)]);
        assert!(err(transition(&mut b, "T1", Status::Done)).contains("not merged"));
        prs(&mut b, "T1", vec![pr(PrState::Merged)]);
        transition(&mut b, "T1", Status::Done).unwrap();
    }

    #[test]
    fn multi_repo_tasks_need_a_pr_in_every_repo_for_done() {
        let mut b = board(vec![t("T1", &[])]);
        b.tasks[0].repos = vec!["r".into(), "ui".into()];
        set(&mut b, "T1", Status::Implementing);
        prs(&mut b, "T1", vec![pr(PrState::Merged)]);
        assert!(err(transition(&mut b, "T1", Status::Done)).contains("waiting for PRs in: ui"));
        let mut ui = pr(PrState::Closed);
        ui.repo = "ui".into();
        ui.url = "https://github.com/o/ui/pull/2".into();
        prs(&mut b, "T1", vec![pr(PrState::Merged), ui.clone()]);
        assert!(err(transition(&mut b, "T1", Status::Done)).contains("waiting for PRs in: ui"));
        ui.state = PrState::Merged;
        prs(&mut b, "T1", vec![pr(PrState::Merged), ui]);
        transition(&mut b, "T1", Status::Done).unwrap();
    }

    #[test]
    fn spike_can_finish_without_a_pr() {
        let mut b = board(vec![t("T1", &[])]);
        b.tasks[0].kind = TaskType::Spike;
        set(&mut b, "T1", Status::Implementing);
        transition(&mut b, "T1", Status::Done).unwrap();
    }

    #[test]
    fn closed_prs_are_ignored_for_done() {
        let mut b = board(vec![t("T1", &[])]);
        set(&mut b, "T1", Status::Implementing);
        prs(&mut b, "T1", vec![pr(PrState::Closed), pr(PrState::Merged)]);
        transition(&mut b, "T1", Status::Done).unwrap();
    }

    #[test]
    fn dropping_a_task_with_live_dependents_is_refused() {
        let mut b = board(vec![t("T1", &[]), t("T2", &["T1"])]);
        assert!(err(transition(&mut b, "T1", Status::Dropped)).contains("T2 depends on"));
        transition(&mut b, "T2", Status::Dropped).unwrap();
        transition(&mut b, "T1", Status::Dropped).unwrap();
    }

    #[test]
    fn agent_is_cleared_when_a_run_finishes() {
        let mut b = board(vec![t("T1", &[])]);
        transition(&mut b, "T1", Status::Planning).unwrap();
        b.tasks[0].agent = Some(Agent { pane: "w1:p1".into(), skill: "story-plan-task".into(), started_at: "now".into() });
        transition(&mut b, "T1", Status::Planned).unwrap();
        assert!(b.tasks[0].agent.is_none());
    }
    fn done_task(id: &str) -> Task {
        let mut t = t(id, &[]);
        t.kind = TaskType::Spike;
        t.status = Status::Done;
        t
    }

    #[test]
    fn story_review_needs_at_least_one_task() {
        let mut b = board(vec![]);
        assert!(err(story_transition(&mut b, StoryStatus::InReview)).contains("no tasks"));
    }

    #[test]
    fn story_review_needs_every_task_done_and_ignores_dropped() {
        let mut dropped = t("T3", &[]);
        dropped.status = Status::Dropped;
        let mut b = board(vec![done_task("T1"), t("T2", &[]), dropped]);
        let e = err(story_transition(&mut b, StoryStatus::InReview));
        assert!(e.contains("T2 is todo"), "{e}");
        assert!(!e.contains("T3"), "{e}");
        b.tasks[1].status = Status::Dropped;
        story_transition(&mut b, StoryStatus::InReview).unwrap();
        assert_eq!(b.story.status, StoryStatus::InReview);
    }

    #[test]
    fn story_done_only_follows_review() {
        let mut b = board(vec![done_task("T1")]);
        assert!(err(story_transition(&mut b, StoryStatus::Done)).contains("illegal story transition"));
        story_transition(&mut b, StoryStatus::InReview).unwrap();
        story_transition(&mut b, StoryStatus::Done).unwrap();
        assert!(err(story_transition(&mut b, StoryStatus::InReview)).contains("illegal story transition"));
    }

    #[test]
    fn story_can_be_reopened_from_review_and_from_done() {
        let mut b = board(vec![done_task("T1")]);
        story_transition(&mut b, StoryStatus::InReview).unwrap();
        story_transition(&mut b, StoryStatus::InProgress).unwrap();
        story_transition(&mut b, StoryStatus::InReview).unwrap();
        story_transition(&mut b, StoryStatus::Done).unwrap();
        story_transition(&mut b, StoryStatus::InProgress).unwrap();
        assert_eq!(b.story.status, StoryStatus::InProgress);
    }

    #[test]
    fn story_status_change_to_the_same_value_is_refused() {
        let mut b = board(vec![done_task("T1")]);
        assert!(err(story_transition(&mut b, StoryStatus::InProgress)).contains("already"));
    }
}
