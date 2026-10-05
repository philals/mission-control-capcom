use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use storyboard::model::{Board, PrState, Status, StoryStatus, TaskType};
use storyboard::{ops, refresh, rules, store};

#[derive(Parser)]
#[command(name = "storyboard", about = "Local kanban boards for Jira stories", after_help = "Errors exit 1 (usage errors exit 2). All output is JSON.")]
struct Cli {
    /// Directory holding one folder per story (required: set it here or in STORYBOARD_ROOT)
    #[arg(long, env = "STORYBOARD_ROOT", global = true)]
    root: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a story folder with an empty board and story.md
    Init {
        /// Jira story key, for example PROJ-123
        key: String,
        /// Story title
        #[arg(long)]
        title: String,
        /// Link to the Jira story
        #[arg(long)]
        jira_url: Option<String>,
    },
    /// Print the stories folder in use (from --root or STORYBOARD_ROOT)
    Root,
    /// List every story with its title and task count
    List,
    /// Print a story's whole board as JSON
    Show {
        /// Jira story key
        key: String,
    },
    /// Check a story's board against the schema and rules
    Validate {
        /// Jira story key
        key: String,
    },
    /// List unfinished tasks whose dependencies are done (includes tasks already implementing; check the status too)
    Ready {
        /// Jira story key
        key: String,
    },
    /// Add a task and create its task file stub
    AddTask {
        /// Jira story key
        key: String,
        /// Task title
        #[arg(long)]
        title: String,
        /// Task type: pr, spike or decision
        #[arg(long = "type", default_value = "pr")]
        kind: String,
        /// Comma-separated ids of tasks this one depends on
        #[arg(long, value_delimiter = ',')]
        depends: Vec<String>,
        /// Comma-separated repos the task will open PRs in
        #[arg(long, value_delimiter = ',')]
        repos: Vec<String>,
    },
    /// Replace a task's dependencies (comma-separated ids; omit to clear)
    SetDeps {
        /// Jira story key
        key: String,
        /// Task id, for example T2
        id: String,
        /// Comma-separated ids of tasks this one depends on
        #[arg(long, value_delimiter = ',')]
        depends: Vec<String>,
    },
    /// Replace a task's repos (comma-separated; omit to clear)
    SetRepos {
        /// Jira story key
        key: String,
        /// Task id, for example T2
        id: String,
        /// Comma-separated repos the task will open PRs in
        #[arg(long, value_delimiter = ',')]
        repos: Vec<String>,
    },
    /// Move a task to a status: todo, planning, planned, implementing, done or dropped
    Status {
        /// Jira story key
        key: String,
        /// Task id, for example T2
        id: String,
        /// New status: todo, planning, planned, implementing, done or dropped
        status: String,
    },
    /// Move the story itself between in_progress, in_review and done
    ///
    /// in_review needs every task done or dropped (and at least one done). done only follows
    /// in_review. Moving back to in_progress reopens the story.
    StoryStatus {
        /// Jira story key
        key: String,
        /// New story status: in_progress, in_review or done
        status: String,
    },
    /// Mark a task blocked with a reason
    Block {
        /// Jira story key
        key: String,
        /// Task id, for example T2
        id: String,
        /// Why the task is blocked
        #[arg(long)]
        reason: String,
    },
    /// Clear a task's blocked flag
    Unblock {
        /// Jira story key
        key: String,
        /// Task id, for example T2
        id: String,
    },
    /// Record a PR against a task
    AddPr {
        /// Jira story key
        key: String,
        /// Task id, for example T2
        id: String,
        /// Repo the PR is in
        #[arg(long)]
        repo: String,
        /// PR url (https)
        #[arg(long)]
        url: String,
        /// PR state: draft, ready, merged or closed
        #[arg(long, default_value = "draft")]
        state: String,
    },
    /// Set the state of a recorded PR: draft, ready, merged or closed
    SetPr {
        /// Jira story key
        key: String,
        /// Task id, for example T2
        id: String,
        /// PR url as recorded
        #[arg(long)]
        url: String,
        /// PR state: draft, ready, merged or closed
        #[arg(long)]
        state: String,
    },
    /// Record the agent pane and skill working on a task
    SetAgent {
        /// Jira story key
        key: String,
        /// Task id, for example T2
        id: String,
        /// Herdr pane identifier
        #[arg(long)]
        pane: String,
        /// Skill the agent is running
        #[arg(long)]
        skill: String,
    },
    /// Clear a task's agent record
    ClearAgent {
        /// Jira story key
        key: String,
        /// Task id, for example T2
        id: String,
    },
    /// Record the Jira sub-task key for a task
    SetSubtask {
        /// Jira story key
        key: String,
        /// Task id, for example T2
        id: String,
        /// Jira sub-task key, for example PROJ-456
        jira: String,
    },
    /// Update PR states from GitHub and mark tasks done once every PR is merged (needs gh authenticated)
    Refresh {
        /// Jira story key
        key: String,
    },
}

fn print(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn task_json(board: &Board, id: &str) -> Result<Value> {
    match board.task(id) {
        Some(t) => Ok(serde_json::to_value(t)?),
        None => bail!("no task {id}"),
    }
}

fn mutate(
    root: &Path,
    key: &str,
    f: impl FnOnce(&mut Board, &Path) -> Result<Value>,
) -> Result<()> {
    let dir = store::story_dir(root, key)?;
    let out = store::update(root, key, |board| f(board, &dir))?;
    print(&out)
}

fn parse_status(s: &str) -> Result<Status> {
    Status::parse(s).ok_or_else(|| anyhow::anyhow!("unknown status {s:?}"))
}

fn parse_pr_state(s: &str) -> Result<PrState> {
    PrState::parse(s).ok_or_else(|| anyhow::anyhow!("unknown PR state {s:?}"))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = cli.root.ok_or_else(|| {
        anyhow::anyhow!("no stories folder: set STORYBOARD_ROOT or pass --root DIR")
    })?;
    let root = root.as_path();
    match cli.cmd {
        Cmd::Init { key, title, jira_url } => {
            ops::init_story(root, &key, &title, jira_url.as_deref())?;
            print(&json!({ "key": key, "dir": store::story_dir(root, &key)? }))
        }
        Cmd::Root => print(&json!({ "root": root })),
        Cmd::List => {
            let mut rows = Vec::new();
            for key in store::list_keys(root)? {
                match store::load(root, &key) {
                    Ok(board) => rows.push(
                        json!({
                            "key": key,
                            "title": board.story.title,
                            "status": board.story.status.as_str(),
                            "tasks": board.tasks.len()
                        }),
                    ),
                    Err(e) => rows.push(json!({ "key": key, "error": format!("{e:#}") })),
                }
            }
            print(&Value::Array(rows))
        }
        Cmd::Show { key } => print(&serde_json::to_value(store::load(root, &key)?)?),
        Cmd::Validate { key } => {
            store::load(root, &key)?;
            print(&json!({ "valid": true }))
        }
        Cmd::Ready { key } => print(&json!({ "ready": rules::ready_ids(&store::load(root, &key)?) })),
        Cmd::AddTask { key, title, kind, depends, repos } => {
            let kind = TaskType::parse(&kind)
                .ok_or_else(|| anyhow::anyhow!("unknown task type {kind:?} (pr, spike, decision)"))?;
            mutate(root, &key, |board, dir| {
                let id = ops::add_task(dir, board, &title, kind, depends, repos)?;
                let file = board.task(&id).map(|t| t.file.clone()).unwrap_or_default();
                Ok(json!({ "id": id, "file": file }))
            })
        }
        Cmd::SetDeps { key, id, depends } => mutate(root, &key, |b, _| {
            ops::set_deps(b, &id, depends)?;
            task_json(b, &id)
        }),
        Cmd::SetRepos { key, id, repos } => mutate(root, &key, |b, _| {
            ops::set_repos(b, &id, repos)?;
            task_json(b, &id)
        }),
        Cmd::Status { key, id, status } => {
            let to = parse_status(&status)?;
            mutate(root, &key, |b, _| {
                rules::transition(b, &id, to)?;
                task_json(b, &id)
            })
        }
        Cmd::StoryStatus { key, status } => {
            let to = StoryStatus::parse(&status)
                .ok_or_else(|| anyhow::anyhow!("unknown story status {status:?} (in_progress, in_review, done)"))?;
            mutate(root, &key, |b, _| {
                rules::story_transition(b, to)?;
                Ok(json!({ "key": key, "status": b.story.status.as_str() }))
            })
        }
        Cmd::Block { key, id, reason } => mutate(root, &key, |b, _| {
            ops::block(b, &id, &reason)?;
            task_json(b, &id)
        }),
        Cmd::Unblock { key, id } => mutate(root, &key, |b, _| {
            ops::unblock(b, &id)?;
            task_json(b, &id)
        }),
        Cmd::AddPr { key, id, repo, url, state } => {
            let state = parse_pr_state(&state)?;
            mutate(root, &key, |b, _| {
                ops::add_pr(b, &id, &repo, &url, state)?;
                task_json(b, &id)
            })
        }
        Cmd::SetPr { key, id, url, state } => {
            let state = parse_pr_state(&state)?;
            mutate(root, &key, |b, _| {
                ops::set_pr_state(b, &id, &url, state)?;
                task_json(b, &id)
            })
        }
        Cmd::SetAgent { key, id, pane, skill } => mutate(root, &key, |b, _| {
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            ops::set_agent(b, &id, &pane, &skill, &now)?;
            task_json(b, &id)
        }),
        Cmd::ClearAgent { key, id } => mutate(root, &key, |b, _| {
            ops::clear_agent(b, &id)?;
            task_json(b, &id)
        }),
        Cmd::SetSubtask { key, id, jira } => mutate(root, &key, |b, _| {
            ops::set_subtask(b, &id, &jira)?;
            task_json(b, &id)
        }),
        Cmd::Refresh { key } => {
            let snapshot = store::load(root, &key)?;
            let looked_up = refresh::lookup_all(&refresh::pending_urls(&snapshot), &refresh::GhLookup);
            let messages = store::update(root, &key, |b| Ok(refresh::apply(b, &looked_up)))?;
            print(&json!({ "messages": messages }))
        }
    }
}
