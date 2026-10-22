use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use capcom::model::{Board, PrState, Status, StoryStatus, TaskType};
use capcom::{ops, refresh, rules, store};

#[derive(Parser)]
#[command(name = "capcom", about = "Local kanban boards for Jira stories", after_help = "Errors exit 1 (usage errors exit 2). All output is JSON.")]
struct Cli {
    /// Directory holding one folder per story (required: set it here or in CAPCOM_ROOT)
    #[arg(long, env = "CAPCOM_ROOT", global = true)]
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
    /// Keep a short note about the agent running this session, shown in capcom-tui's agents panel.
    /// Leave an option out to keep what the note says; give an empty text to clear it.
    Note {
        /// What you were asked to do, in one line
        #[arg(long)]
        task: Option<String>,
        /// Where you have got to
        #[arg(long)]
        progress: Option<String>,
        /// What you are waiting on (empty: not waiting)
        #[arg(long)]
        waiting: Option<String>,
        /// The wait is on the user (true or false)
        #[arg(long)]
        needs_you: Option<bool>,
        /// What the user needs to do next (empty: nothing)
        #[arg(long)]
        next: Option<String>,
        /// Claude Code session id (Claude Code sets CLAUDE_CODE_SESSION_ID in its shell)
        #[arg(long, env = "CLAUDE_CODE_SESSION_ID")]
        session: String,
    },
    /// Serve the boards and your open PRs as a page on localhost (read-only; it polls GitHub itself while a browser is open)
    Web {
        /// Port on 127.0.0.1 to listen on
        #[arg(long, env = "CAPCOM_WEB_PORT", default_value_t = 7878)]
        port: u16,
        /// GitHub search for the pull request list (@me is resolved by GitHub)
        #[arg(long, env = "CAPCOM_PR_QUERY", default_value = capcom::prs::DEFAULT_QUERY)]
        pr_query: String,
        /// Do not fetch pull requests from GitHub
        #[arg(long)]
        no_prs: bool,
        /// Command that opens a link when you click one on the page (default: xdg-open, so your default browser)
        #[arg(long, env = "CAPCOM_BROWSER")]
        browser: Option<String>,
        /// Folder new Claude Code sessions started from the page begin in (default: the current folder)
        #[arg(long, env = "CAPCOM_WORKDIR")]
        workdir: Option<PathBuf>,
        /// Extra repos (owner/name, comma separated) to look for your manual workflow runs in
        #[arg(long, env = "CAPCOM_DEPLOY_REPOS", value_delimiter = ',')]
        deploy_repos: Vec<String>,
        /// Do not fetch your manual workflow runs from GitHub
        #[arg(long)]
        no_runs: bool,
    },
    /// Print the stories folder in use (from --root or CAPCOM_ROOT)
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
    /// Record the Claude Code session working on a task, so it can be resumed from the board
    SetSession {
        /// Jira story key
        key: String,
        /// Task id, for example T2
        id: String,
        /// Skill the session is running
        #[arg(long)]
        skill: String,
        /// Claude Code session id (Claude Code sets CLAUDE_CODE_SESSION_ID in its shell)
        #[arg(long, env = "CLAUDE_CODE_SESSION_ID")]
        session: String,
        /// Directory the session was started in (default: read from its transcript, else the current directory)
        #[arg(long)]
        cwd: Option<String>,
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
    if let Cmd::Note { task, progress, waiting, needs_you, next, session } = cli.cmd {
        let dir = capcom::notes::dir().ok_or_else(|| anyhow::anyhow!("no home folder for the notes: set CAPCOM_NOTES_DIR"))?;
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let cwd = capcom::notes::read(&dir, &session).filter(|n| !n.cwd.is_empty()).map(|n| n.cwd).or_else(|| capcom::session::start_dir(&session)).or_else(|| std::env::current_dir().ok().map(|d| d.to_string_lossy().to_string()));
        let note = capcom::notes::write(&dir, &session, capcom::notes::Update { task, progress, waiting, needs_you, next, cwd }, &now)?;
        return print(&serde_json::to_value(note)?);
    }
    let root = cli.root.ok_or_else(|| {
        anyhow::anyhow!("no stories folder: set CAPCOM_ROOT or pass --root DIR")
    })?;
    let root = root.as_path();
    match cli.cmd {
        Cmd::Init { key, title, jira_url } => {
            ops::init_story(root, &key, &title, jira_url.as_deref())?;
            print(&json!({ "key": key, "dir": store::story_dir(root, &key)? }))
        }
        Cmd::Note { .. } => unreachable!("handled before the root is needed"),
        Cmd::Web { port, pr_query, no_prs, browser, workdir, deploy_repos, no_runs } => {
            use capcom::{cache, fixstate, prs, runs, settings, web};
            use std::sync::Arc;
            let (listener, port) = web::bind(port)?;
            let server = web::Server::new(root.to_path_buf(), port);
            server.set_opener(Arc::new(web::SystemBrowser { command: browser }));
            server.set_tools(web::Tools::real());
            server.set_workdir(workdir.or_else(|| std::env::current_dir().ok()).unwrap_or_default());
            server.set_extra_repos(deploy_repos);
            server.load_settings(settings::default_path());
            let cache_dir = cache::Cache::default_dir();
            server.load_durations(cache_dir.as_ref().map(|d| d.join("check-durations.json")));
            server.set_fix_store(cache_dir.clone().map(fixstate::Store::new));
            let shared = || cache::Cache::default_dir().and_then(cache::Cache::open);
            if !no_prs {
                let cadence = prs::Cadence::default();
                let gh = Arc::new(prs::GhSource::new(pr_query.clone()));
                let live: Arc<dyn prs::PrSource> = gh.clone();
                let listed: Arc<dyn prs::PrSource> = match shared() {
                    Some(cache) => Arc::new(prs::SharedSource::new(live, cache, &pr_query, cadence)),
                    None => live,
                };
                server.poll(server.with_followed(gh, listed));
            }
            if !no_runs {
                let cadence = runs::RunCadence::default();
                let live: Arc<dyn runs::RunSource> = Arc::new(runs::Fetcher::new(runs::GhApi::default(), chrono::Duration::minutes(30)));
                let source: Arc<dyn runs::RunSource> = match shared() {
                    Some(cache) => Arc::new(runs::SharedSource { inner: live, cache, cadence }),
                    None => live,
                };
                server.poll_runs(source);
            }
            if let Some(config) = capcom::session::config_dir() {
                server.watch_agents(Arc::new(web::ClaudeSessions { config: config.clone() }), capcom::notes::dir(), web::AGENTS_EVERY, std::time::Duration::from_secs(1));
                server.watch_sessions(config, cache_dir.map(|d| d.join("web-links.json")), std::time::Duration::from_secs(30), std::time::Duration::from_secs(1));
            }
            server.watch_pr_states(std::time::Duration::from_secs(1));
            print(&json!({ "url": format!("http://127.0.0.1:{port}") }))?;
            server.serve(listener);
            Ok(())
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
        Cmd::SetSession { key, id, skill, session, cwd } => mutate(root, &key, |b, _| {
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let cwd = match cwd.or_else(|| capcom::session::start_dir(&session)) {
                Some(c) => c,
                None => std::env::current_dir()?.to_string_lossy().to_string(),
            };
            ops::add_session(b, &id, &session, &skill, &cwd, &now)?;
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
