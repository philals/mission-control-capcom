mod app;
mod cache;
mod demo;
mod finish;
mod herdr;
mod panel;
mod pr_state;
mod prs;
mod runs;
mod runs_ui;
mod settings;
mod svg;
mod theme;
mod ui;

use anyhow::{bail, Result};
use app::App;
use clap::Parser;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event, KeyEventKind, KeyModifiers, MouseButton,
    MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::SetTitle;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Parser)]
#[command(name = "capcom-tui", about = "Live kanban view of capcom story boards, with your open PRs, CI and manual runs (keyboard and mouse)")]
struct Cli {
    /// Open this story's board straight away, for example PROJ-123 (default: the story list)
    key: Option<String>,
    /// Directory holding one folder per story (required: set it here or in CAPCOM_ROOT)
    #[arg(long, env = "CAPCOM_ROOT")]
    root: Option<PathBuf>,
    /// GitHub search used for the pull request panel (@me is resolved by GitHub)
    #[arg(long, env = "CAPCOM_PR_QUERY", default_value = prs::DEFAULT_QUERY)]
    pr_query: String,
    /// Do not fetch pull requests from GitHub
    #[arg(long)]
    no_prs: bool,
    /// Extra repos (owner/name, comma separated) to look for your manual workflow runs in
    #[arg(long, env = "CAPCOM_DEPLOY_REPOS", value_delimiter = ',')]
    deploy_repos: Vec<String>,
    /// Do not fetch your manual workflow runs from GitHub
    #[arg(long)]
    no_runs: bool,
    /// Print the pane (from `herdr pane list` JSON on stdin) already running capcom-tui; used by the plugin
    #[arg(long, hide = true)]
    find_pane: bool,
    /// Folder new Herdr workspaces and tabs start in (default: the current folder)
    #[arg(long, env = "CAPCOM_WORKDIR")]
    workdir: Option<PathBuf>,
    /// Try it with invented stories, PRs and runs: needs no stories folder, GitHub or Herdr
    #[arg(long)]
    demo: bool,
    /// Draw the demo screen to an SVG file and exit (used for the README pictures)
    #[arg(long, value_name = "FILE", hide = true)]
    screenshot: Option<PathBuf>,
    /// Which demo screen to draw: list or board
    #[arg(long, default_value = "board", hide = true)]
    screen: String,
    /// Size of the drawn screen in columns x rows
    #[arg(long, default_value = "150x42", hide = true)]
    size: String,
}

fn screenshot(path: &std::path::Path, screen: &str, size: &str) -> Result<()> {
    let (w, h) = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u16>().ok()?, h.parse::<u16>().ok()?)))
        .filter(|(w, h)| *w >= 40 && *h >= 12)
        .ok_or_else(|| anyhow::anyhow!("--size must look like 150x42"))?;
    let root = demo::make_root();
    let mut app = demo::app(&root);
    if screen == "board" {
        app.open_story("DEMO-101");
        app.col = 3;
        app.row[3] = 0;
    }
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h))?;
    terminal.draw(|frame| ui::draw(frame, &app))?;
    let (title, description) = match screen {
        "board" => (
            "capcom story board",
            "A kanban board of one demo story with its tasks in TODO, PLANNING, PLANNED, IMPLEMENTING and DONE columns, above panels of open pull requests with their CI stages and of manual workflow runs.",
        ),
        _ => (
            "capcom story list",
            "A list of demo stories with progress bars, above panels of open pull requests with their CI stages and of manual workflow runs.",
        ),
    };
    std::fs::write(path, svg::render(terminal.backend().buffer(), title, description))?;
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

fn run_demo(key: Option<String>) -> Result<()> {
    let root = demo::make_root();
    let mut app = demo::app(&root);
    if let Some(key) = key {
        app.open_story(&key);
    }
    let result = run_terminal(&mut app);
    let _ = std::fs::remove_dir_all(&root);
    result
}

fn shared_cache() -> Option<cache::Cache> {
    cache::Cache::default_dir().and_then(cache::Cache::open)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.find_pane {
        let mut input = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
        if let Some(found) = herdr::find_pane(&input) {
            println!("{found}");
        }
        return Ok(());
    }
    if let Some(path) = &cli.screenshot {
        return screenshot(path, &cli.screen, &cli.size);
    }
    if cli.demo {
        return run_demo(cli.key);
    }
    let Some(root) = cli.root else {
        bail!("no stories folder: set CAPCOM_ROOT or pass --root DIR");
    };
    let mut app = App::new(root, cli.key);
    if cli.no_prs {
        app.prs.disabled = true;
    } else {
        let cadence = prs::Cadence::default();
        let live: Arc<dyn prs::PrSource> = Arc::new(prs::GhSource { query: cli.pr_query.clone() });
        let source: Arc<dyn prs::PrSource> = match shared_cache() {
            Some(cache) => Arc::new(prs::SharedSource::new(live, cache, &cli.pr_query, cadence)),
            None => live,
        };
        let (rx, wake) = prs::spawn(source, cadence);
        app.attach_feed(rx, wake);
    }
    if herdr::inside_herdr() {
        let cwd = cli.workdir.or_else(|| std::env::current_dir().ok()).unwrap_or_default();
        app.herdr = Some(Arc::new(herdr::Cli::new(cwd)));
    }
    app.settings_path = settings::default_path();
    app.load_settings();
    app.extra_repos = cli.deploy_repos;
    let (checker_tx, checker_rx) =
        pr_state::spawn(Arc::new(capcom::refresh::GhLookup), shared_cache(), Duration::from_secs(60));
    app.attach_checker(checker_tx, checker_rx);
    if cli.no_runs {
        app.runs.disabled = true;
    } else {
        let repos = Arc::new(Mutex::new(Vec::new()));
        let cadence = runs::RunCadence::default();
        let live: Arc<dyn runs::RunSource> =
            Arc::new(runs::Fetcher::new(runs::GhApi::default(), chrono::Duration::hours(3)));
        let source: Arc<dyn runs::RunSource> = match shared_cache() {
            Some(cache) => Arc::new(runs::SharedSource { inner: live, cache, cadence }),
            None => live,
        };
        let (rx, wake) = runs::spawn(source, repos.clone(), cadence);
        app.attach_runs(rx, wake, repos);
    }
    run_terminal(&mut app)
}

fn run_terminal(app: &mut App) -> Result<()> {
    let mut terminal = ratatui::init();
    execute!(std::io::stdout(), EnableMouseCapture, EnableBracketedPaste, SetTitle(herdr::PANE_TITLE))?;
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(std::io::stdout(), DisableMouseCapture, DisableBracketedPaste);
        previous_hook(info);
    }));
    let result = run(&mut terminal, app);
    let _ = execute!(std::io::stdout(), DisableMouseCapture, DisableBracketedPaste);
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;
        if event::poll(Duration::from_millis(250))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if app.on_key(key.code, key.modifiers.contains(KeyModifiers::CONTROL)) {
                        return Ok(());
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => app.on_click(mouse.column, mouse.row),
                    MouseEventKind::Drag(MouseButton::Left) => app.on_drag(mouse.column, mouse.row),
                    MouseEventKind::Up(MouseButton::Left) => app.on_release(),
                    MouseEventKind::ScrollUp => app.on_scroll(mouse.column, mouse.row, -1),
                    MouseEventKind::ScrollDown => app.on_scroll(mouse.column, mouse.row, 1),
                    _ => {}
                },
                Event::Paste(text) => app.on_paste(&text),
                _ => {}
            }
        }
        app.poll_feed();
        app.poll_runs();
        app.poll_ready();
        if app.needs_reload() {
            app.reload();
        }
    }
}
