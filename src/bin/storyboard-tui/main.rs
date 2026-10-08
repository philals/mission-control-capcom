mod app;
mod panel;
mod prs;
mod ui;

use anyhow::{bail, Result};
use app::App;
use clap::Parser;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, KeyModifiers, MouseButton,
    MouseEventKind,
};
use ratatui::crossterm::execute;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "storyboard-tui", about = "Live, read-only kanban view of storyboard boards (keyboard and mouse)")]
struct Cli {
    /// Open this story's board straight away, for example PROJ-123 (default: the story list)
    key: Option<String>,
    /// Directory holding one folder per story (required: set it here or in STORYBOARD_ROOT)
    #[arg(long, env = "STORYBOARD_ROOT")]
    root: Option<PathBuf>,
    /// GitHub search used for the pull request panel (@me is resolved by GitHub)
    #[arg(long, env = "STORYBOARD_PR_QUERY", default_value = prs::DEFAULT_QUERY)]
    pr_query: String,
    /// Do not fetch pull requests from GitHub
    #[arg(long)]
    no_prs: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let Some(root) = cli.root else {
        bail!("no stories folder: set STORYBOARD_ROOT or pass --root DIR");
    };
    let mut app = App::new(root, cli.key);
    if cli.no_prs {
        app.prs.disabled = true;
    } else {
        let source = Arc::new(prs::GhSource { query: cli.pr_query });
        let (rx, wake) = prs::spawn(source, prs::Cadence::default());
        app.attach_feed(rx, wake);
    }
    let mut terminal = ratatui::init();
    execute!(std::io::stdout(), EnableMouseCapture)?;
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
        previous_hook(info);
    }));
    let result = run(&mut terminal, &mut app);
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
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
                    MouseEventKind::ScrollUp => app.on_scroll(mouse.column, mouse.row, -1),
                    MouseEventKind::ScrollDown => app.on_scroll(mouse.column, mouse.row, 1),
                    _ => {}
                },
                _ => {}
            }
        }
        app.poll_feed();
        if app.needs_reload() {
            app.reload();
        }
    }
}
