use crate::app::{App, Column, Screen, StorySummary, Target, STATUS_ORDER};
use crate::panel;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use ratatui::Frame;
use storyboard::model::{Board, PrState, Status, StoryStatus, Task};
use storyboard::rules;

type Hits = Vec<(Rect, Target)>;

const CARD_HEIGHT: u16 = 5;
const STORY_ROW_HEIGHT: u16 = 4;
const COMPACT_BELOW: u16 = 60;
const BACK_LABEL: &str = "‹ Stories";

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let mut hits: Hits = Vec::new();
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)])
        .split(area);
    let panel_height = panel::height(app, rows[1].height);
    let (main, panel_area) = if panel_height > 0 {
        let parts = Layout::vertical([Constraint::Min(3), Constraint::Length(panel_height)]).split(rows[1]);
        (parts[0], Some(parts[1]))
    } else {
        (rows[1], None)
    };
    match app.screen {
        Screen::List => {
            draw_list_header(f, rows[0], app, &mut hits);
            draw_list(f, main, app, &mut hits);
        }
        Screen::Board => {
            draw_board_header(f, rows[0], app, &mut hits);
            if app.board.is_some() {
                draw_board(f, main, app, &mut hits);
            } else {
                draw_empty(f, main, app);
            }
        }
    }
    if let Some(panel_area) = panel_area {
        panel::draw_panel(f, panel_area, app, &mut hits);
    }
    draw_footer(f, rows[2], app);
    if app.help {
        draw_help(f, area, &mut hits);
    } else if app.pr_sheet {
        panel::draw_sheet(f, area, app, &mut hits);
    } else if app.detail && app.screen == Screen::Board {
        draw_detail(f, area, app, &mut hits);
    }
    *app.hits.borrow_mut() = hits;
}

fn label(status: Status) -> &'static str {
    match status {
        Status::Todo => "TODO",
        Status::Planning => "PLANNING",
        Status::Planned => "PLANNED",
        Status::Implementing => "IMPLEMENTING",
        Status::Done => "DONE",
        Status::Dropped => "DROPPED",
    }
}

fn story_color(status: StoryStatus) -> Color {
    match status {
        StoryStatus::InProgress => Color::Cyan,
        StoryStatus::InReview => Color::Magenta,
        StoryStatus::Done => Color::Green,
    }
}

fn trunc(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(1);
    let mut out: String = text.chars().take(keep).collect();
    out.push('…');
    out
}

fn padded(left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: u16) -> Line<'static> {
    let used = Line::from(left.clone()).width() + Line::from(right.clone()).width();
    let pad = (width as usize).saturating_sub(used);
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(pad)));
    spans.extend(right);
    Line::from(spans)
}

fn draw_list_header(f: &mut Frame, area: Rect, app: &App, hits: &mut Hits) {
    let dim = Style::new().fg(Color::DarkGray);
    let left = vec![
        Span::styled("◇ storyboard", Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled("  Stories", Style::new().add_modifier(Modifier::BOLD)),
    ];
    let shown = app.visible_stories().len();
    let hidden = app.hidden_done_count();
    let mut counts = format!("{shown} shown");
    if hidden > 0 {
        counts.push_str(&format!(" · {hidden} done hidden"));
    }
    let toggle = if app.hide_done { "[ Show done ]" } else { "[ Hide done ]" };
    let right = vec![
        Span::styled(counts, dim),
        Span::raw("  "),
        Span::styled(toggle, Style::new().fg(Color::Cyan)),
    ];
    let toggle_width = toggle.chars().count() as u16;
    if area.width > toggle_width {
        let rect = Rect::new(area.x + area.width - toggle_width, area.y, toggle_width, 1);
        hits.push((rect, Target::ToggleDone));
    }
    f.render_widget(Paragraph::new(padded(left, right, area.width)), area);
}

fn counts_text(s: &StorySummary) -> String {
    let names = ["todo", "planning", "planned", "implementing", "done", "dropped"];
    STATUS_ORDER
        .iter()
        .enumerate()
        .filter(|(i, _)| s.counts[*i] > 0)
        .map(|(i, _)| format!("{} {}", s.counts[i], names[i]))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn progress_bar(done: usize, total: usize) -> String {
    let cells = 10;
    let filled = if total == 0 { 0 } else { done * cells / total };
    format!("{}{}", "█".repeat(filled), "░".repeat(cells - filled))
}

fn draw_list(f: &mut Frame, area: Rect, app: &App, hits: &mut Hits) {
    if app.stories.is_empty() {
        draw_empty(f, area, app);
        return;
    }
    let visible = app.visible_stories();
    if visible.is_empty() {
        let hidden = app.hidden_done_count();
        let noun = if hidden == 1 { "completed story is" } else { "completed stories are" };
        let lines = vec![
            Line::from("No stories to show."),
            Line::from(Span::styled(
                format!("{hidden} {noun} hidden - press d (or click Show done) to show them."),
                Style::new().fg(Color::DarkGray),
            )),
        ];
        f.render_widget(Paragraph::new(lines).centered(), area);
        return;
    }
    let capacity = ((area.height / STORY_ROW_HEIGHT) as usize).max(1);
    let offset = if app.list_sel >= capacity { app.list_sel + 1 - capacity } else { 0 };
    for (n, story) in visible.iter().enumerate().skip(offset).take(capacity) {
        let y = area.y + (n - offset) as u16 * STORY_ROW_HEIGHT;
        if y + STORY_ROW_HEIGHT > area.y + area.height {
            break;
        }
        let rect = Rect::new(area.x, y, area.width, STORY_ROW_HEIGHT);
        draw_story_row(f, rect, story, n == app.list_sel);
        hits.push((rect, Target::Story(n)));
    }
}

fn draw_story_row(f: &mut Frame, area: Rect, story: &StorySummary, selected: bool) {
    let border = if selected {
        Style::new().fg(Color::White).add_modifier(Modifier::BOLD)
    } else if story.error.is_some() {
        Style::new().fg(Color::Red)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    let block = Block::bordered()
        .border_type(if selected { BorderType::Thick } else { BorderType::Plain })
        .border_style(border);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let width = inner.width as usize;
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let dim = Style::new().fg(Color::DarkGray);
    if let Some(error) = &story.error {
        let lines = vec![
            Line::from(Span::styled(story.key.clone(), bold)),
            Line::from(Span::styled(trunc(&format!("! {error}"), width), Style::new().fg(Color::Red))),
        ];
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }
    let badge = format!("[ {} ]", story.status.as_str());
    let title_room = width.saturating_sub(story.key.len() + 2 + badge.chars().count() + 1);
    let first = padded(
        vec![
            Span::styled(story.key.clone(), bold),
            Span::raw("  "),
            Span::raw(trunc(&story.title, title_room)),
        ],
        vec![Span::styled(badge, Style::new().fg(story_color(story.status)))],
        inner.width,
    );
    let second = Line::from(vec![
        Span::styled(format!("{}/{} done  ", story.done, story.total), Style::new().fg(Color::Green)),
        Span::styled(progress_bar(story.done, story.total), Style::new().fg(Color::Green)),
        Span::raw("  "),
        Span::styled(trunc(&counts_text(story), width.saturating_sub(24)), dim),
    ]);
    f.render_widget(Paragraph::new(vec![first, second]), inner);
}

fn draw_board_header(f: &mut Frame, area: Rect, app: &App, hits: &mut Hits) {
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let dim = Style::new().fg(Color::DarkGray);
    let back_width = BACK_LABEL.chars().count() as u16;
    hits.push((Rect::new(area.x, area.y, back_width.min(area.width), 1), Target::Back));
    let mut left = vec![
        Span::styled(BACK_LABEL, Style::new().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled("◇ storyboard", bold.fg(Color::Cyan)),
    ];
    let mut right = Vec::new();
    if let Some((pos, count)) = app.story_position() {
        if count > 1 {
            right.push(Span::styled(format!("‹ {pos}/{count} ›  "), dim));
        }
    }
    if let Some(board) = &app.board {
        let live = board.tasks.iter().filter(|t| t.status != Status::Dropped).count();
        let done = board.tasks.iter().filter(|t| t.status == Status::Done).count();
        left.push(Span::raw("  "));
        left.push(Span::styled(board.story.key.clone(), bold));
        left.push(Span::styled(format!("  {}", board.story.title), dim));
        left.push(Span::styled(format!("  {done}/{live} done"), Style::new().fg(Color::Green)));
        right.push(Span::styled(
            format!("[ {} ]", board.story.status.as_str()),
            Style::new().fg(story_color(board.story.status)),
        ));
    }
    f.render_widget(Paragraph::new(padded(left, right, area.width)), area);
}

fn draw_empty(f: &mut Frame, area: Rect, app: &App) {
    let mut lines = vec![
        Line::from(format!("No stories found in {}", app.root.display())),
        Line::from(Span::styled(
            "Create one with: storyboard init PROJ-123 --title \"...\"",
            Style::new().fg(Color::DarkGray),
        )),
    ];
    if let Some(err) = &app.error {
        lines.push(Line::from(Span::styled(err.clone(), Style::new().fg(Color::Red))));
    }
    f.render_widget(Paragraph::new(lines).centered(), area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    if let Some(err) = &app.error {
        let text = trunc(&format!("! {err}"), area.width as usize);
        f.render_widget(Paragraph::new(Span::styled(text, Style::new().fg(Color::Red))), area);
        return;
    }
    let dim = Style::new().fg(Color::DarkGray);
    let keys: &[&str] = match app.screen {
        Screen::List => &["↑↓ story", "⏎ open", "d show/hide done", "Tab PRs", "? help", "q quit"],
        Screen::Board => &[
            "←→ column", "↑↓ card", "[ ] story", "⏎ detail", "Tab PRs", "Esc stories", "? help",
            "q quit",
        ],
    };
    let mut left = Vec::new();
    for key in keys {
        left.push(Span::styled(format!("[ {key} ]"), dim));
        left.push(Span::raw(" "));
    }
    let right = vec![Span::styled(format!("updated {}", app.updated), dim)];
    f.render_widget(Paragraph::new(padded(left, right, area.width)), area);
}

fn draw_board(f: &mut Frame, area: Rect, app: &App, hits: &mut Hits) {
    let Some(board) = &app.board else { return };
    let cols = app.columns();
    if cols.is_empty() {
        return;
    }
    if area.width < COMPACT_BELOW {
        let i = app.col.min(cols.len() - 1);
        let title = format!("‹ [ ⇄ {} {}/{} ] ›", label(cols[i].status), i + 1, cols.len());
        hits.push((area, Target::Column(i)));
        let half = area.width / 2;
        hits.push((Rect::new(area.x, area.y, half, 1), Target::PrevColumn));
        hits.push((Rect::new(area.x + half, area.y, area.width - half, 1), Target::NextColumn));
        draw_column(f, area, &cols[i], i, true, app.row[i], board, Some(title), hits);
        return;
    }
    let rects = Layout::horizontal(vec![Constraint::Ratio(1, cols.len() as u32); cols.len()])
        .split(area);
    for (i, col) in cols.iter().enumerate() {
        hits.push((rects[i], Target::Column(i)));
        draw_column(f, rects[i], col, i, i == app.col, app.row[i], board, None, hits);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_column(
    f: &mut Frame,
    area: Rect,
    col: &Column,
    index: usize,
    selected: bool,
    sel_row: usize,
    board: &Board,
    title: Option<String>,
    hits: &mut Hits,
) {
    let accent = if selected { Color::Blue } else { Color::DarkGray };
    let title = title.unwrap_or_else(|| format!("{} · {}", label(col.status), col.tasks.len()));
    let inner_height = area.height.saturating_sub(2);
    let visible = ((inner_height / CARD_HEIGHT) as usize).max(1);
    let offset = if sel_row >= visible { sel_row + 1 - visible } else { 0 };
    let more_above = if offset > 0 { " ▲" } else { "" };
    let more_below = if offset + visible < col.tasks.len() { " ▼" } else { "" };
    let block = Block::bordered()
        .border_style(Style::new().fg(accent))
        .title(Span::styled(
            format!(" {title}{more_above}{more_below} "),
            Style::new().fg(if selected { Color::Blue } else { Color::White }).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    for (n, task) in col.tasks.iter().enumerate().skip(offset).take(visible) {
        let y = inner.y + (n - offset) as u16 * CARD_HEIGHT;
        if y + CARD_HEIGHT > inner.y + inner.height {
            break;
        }
        let rect = Rect::new(inner.x, y, inner.width, CARD_HEIGHT);
        draw_card(f, rect, task, board, selected && n == sel_row);
        hits.push((rect, Target::Card { col: index, row: n }));
    }
}

fn unfinished_deps<'a>(task: &'a Task, board: &Board) -> Vec<&'a str> {
    task.depends_on
        .iter()
        .filter(|d| board.task(d).map_or(true, |t| t.status != Status::Done))
        .map(String::as_str)
        .collect()
}

fn status_line(task: &Task, board: &Board) -> (String, Color) {
    if let Some(b) = &task.blocked {
        return (format!("✖ blocked: {}", b.reason), Color::Red);
    }
    let waiting = unfinished_deps(task, board);
    match task.status {
        Status::Todo if waiting.is_empty() => ("· todo".into(), Color::Gray),
        Status::Todo => (format!("· todo · after {}", waiting.join(", ")), Color::Gray),
        Status::Planning => match &task.agent {
            Some(a) => (format!("◌ planning · {}", a.pane), Color::Yellow),
            None => ("◌ planning".into(), Color::Yellow),
        },
        Status::Planned if waiting.is_empty() => ("▶ ready to implement".into(), Color::Green),
        Status::Planned => (format!("◷ waits {}", waiting.join(", ")), Color::Yellow),
        Status::Implementing if task.prs.is_empty() => ("● implementing".into(), Color::Cyan),
        Status::Implementing => {
            let merged = task.prs.iter().filter(|p| p.state == PrState::Merged).count();
            (format!("● implementing · PRs {merged}/{}", task.prs.len()), Color::Cyan)
        }
        Status::Done => ("✓ done".into(), Color::Green),
        Status::Dropped => ("✕ dropped".into(), Color::DarkGray),
    }
}

fn draw_card(f: &mut Frame, area: Rect, task: &Task, board: &Board, selected: bool) {
    let (status, tone) = status_line(task, board);
    let ready_planned = task.status == Status::Planned && rules::is_ready(board, task);
    let border = if selected {
        Style::new().fg(Color::White).add_modifier(Modifier::BOLD)
    } else if task.blocked.is_some() {
        Style::new().fg(Color::Red)
    } else if task.status == Status::Done {
        Style::new().fg(Color::Green)
    } else if ready_planned {
        Style::new().fg(Color::Yellow)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    let block = Block::bordered()
        .border_type(if selected { BorderType::Thick } else { BorderType::Plain })
        .border_style(border);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let width = inner.width as usize;
    let mut meta = format!(
        "{} · {}",
        task.kind.as_str(),
        if task.repos.is_empty() { "no repos".to_string() } else { task.repos.join(", ") }
    );
    if let Some(sub) = &task.jira_subtask {
        meta.push_str(&format!(" · {sub}"));
    }
    let lines = vec![
        Line::from(vec![
            Span::styled(format!("{} ", task.id), Style::new().add_modifier(Modifier::BOLD)),
            Span::raw(trunc(&task.title, width.saturating_sub(task.id.len() + 1))),
        ]),
        Line::from(Span::styled(trunc(&status, width), Style::new().fg(tone))),
        Line::from(Span::styled(trunc(&meta, width), Style::new().fg(Color::DarkGray))),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn centered(area: Rect, pct_x: u16, pct_y: u16) -> Rect {
    let w = (area.width * pct_x / 100).max(40).min(area.width);
    let h = (area.height * pct_y / 100).max(10).min(area.height);
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

fn row(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<12}"), Style::new().fg(Color::DarkGray)),
        Span::raw(value),
    ])
}

fn draw_detail(f: &mut Frame, area: Rect, app: &App, hits: &mut Hits) {
    let (Some(task), Some(board)) = (app.selected_task(), &app.board) else {
        return;
    };
    let rect = centered(area, 70, 70);
    let deps: Vec<String> = task
        .depends_on
        .iter()
        .map(|d| match board.task(d) {
            Some(t) => format!("{d} ({})", t.status.as_str()),
            None => format!("{d} (missing)"),
        })
        .collect();
    let or_none = |v: Vec<String>| if v.is_empty() { "none".to_string() } else { v.join(", ") };
    let mut lines = vec![
        row("Status", task.status.as_str().to_string()),
        row("Type", task.kind.as_str().to_string()),
    ];
    if let Some(b) = &task.blocked {
        lines.push(Line::from(vec![
            Span::styled(format!("{:<12}", "Blocked"), Style::new().fg(Color::DarkGray)),
            Span::styled(b.reason.clone(), Style::new().fg(Color::Red)),
        ]));
    }
    lines.push(row("Depends on", or_none(deps)));
    lines.push(row("Repos", or_none(task.repos.clone())));
    lines.push(row("Jira", task.jira_subtask.clone().unwrap_or_else(|| "none".into())));
    if let Some(a) = &task.agent {
        lines.push(row("Agent", format!("{} · {} · since {}", a.pane, a.skill, a.started_at)));
    }
    lines.push(row("File", task.file.clone()));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled("PRs", Style::new().fg(Color::DarkGray))));
    if task.prs.is_empty() {
        lines.push(Line::raw("  none"));
    }
    for pr in &task.prs {
        lines.push(Line::raw(format!("  {:<8} {}  {}", pr.state.as_str(), pr.repo, pr.url)));
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Blue))
        .title(Span::styled(
            format!(" {} · {} ", task.id, task.title),
            Style::new().add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(" Esc or click outside to close ", Style::new().fg(Color::DarkGray)));
    f.render_widget(Clear, rect);
    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), rect);
    hits.push((rect, Target::Sheet));
}

fn draw_help(f: &mut Frame, area: Rect, hits: &mut Hits) {
    let rect = centered(area, 60, 60);
    let lines = vec![
        row("↑ ↓  j k", "move between stories or cards".into()),
        row("← →  h l", "move between columns (open a story from the list)".into()),
        row("Enter", "open a story, or open a card's detail".into()),
        row("[ ]", "switch story on the board".into()),
        row("Tab", "move focus between the board or list and the PR panel".into()),
        row("o", "open the selected pull request in the browser (PR panel)".into()),
        row("d", "show or hide completed stories (list)".into()),
        row("Esc  b", "back to the list, or close a sheet".into()),
        row("r", "reload now (it also reloads by itself)".into()),
        row("?", "toggle this help".into()),
        row("q  Ctrl-C", "quit".into()),
        Line::raw(""),
        row("Mouse", "click a story, column, card, PR or button; wheel scrolls".into()),
    ];
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Blue))
        .title(Span::styled(" Keys ", Style::new().add_modifier(Modifier::BOLD)));
    f.render_widget(Clear, rect);
    f.render_widget(Paragraph::new(lines).block(block), rect);
    hits.push((rect, Target::Sheet));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Focus, Screen};
    use crate::prs::{Check, CheckState, PullRequest, Review};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use storyboard::model::{Status, TaskType};
    use storyboard::{ops, rules, store};
    use tempfile::TempDir;

    fn sample() -> (TempDir, App) {
        let root = TempDir::new().unwrap();
        ops::init_story(root.path(), "PROJ-1", "Notices", Some("https://j/PROJ-1")).unwrap();
        store::update(root.path(), "PROJ-1", |b| {
            let dir = root.path().join("PROJ-1");
            ops::add_task(&dir, b, "Add endpoint", TaskType::Pr, vec![], vec!["api".into()])?;
            ops::add_task(&dir, b, "Wire UI", TaskType::Pr, vec!["T1".into()], vec!["ui".into()])?;
            ops::add_task(&dir, b, "Spike it", TaskType::Spike, vec![], vec![])?;
            ops::add_task(&dir, b, "Fix schema", TaskType::Pr, vec![], vec!["api".into()])?;
            for id in ["T1", "T2", "T4"] {
                rules::transition(b, id, Status::Planning)?;
                rules::transition(b, id, Status::Planned)?;
            }
            rules::transition(b, "T4", Status::Implementing)?;
            ops::add_pr(b, "T4", "api", "https://github.com/o/api/pull/7", storyboard::model::PrState::Draft)?;
            ops::block(b, "T4", "waiting on design")?;
            Ok(())
        })
        .unwrap();
        let app = App::new(root.path().to_path_buf(), Some("PROJ-1".into()));
        (root, app)
    }

    fn render(app: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_wide_board_shows_header_columns_cards_and_key_rail() {
        let (_root, app) = sample();
        let out = render(&app, 200, 30);
        for want in [
            "PROJ-1", "Notices", "0/4 done", "[ in_progress ]",
            "TODO · 1", "PLANNING · 0", "PLANNED · 2", "IMPLEMENTING · 1", "DONE · 0",
            "T1 Add endpoint", "ready to implement", "T2 Wire UI", "waits T1",
            "T3 Spike it", "T4 Fix schema", "blocked: waiting on design",
            "q quit", "updated",
        ] {
            assert!(out.contains(want), "missing {want:?} in:\n{out}");
        }
        assert!(!out.contains("DROPPED"), "{out}");
    }

    #[test]
    fn a_narrow_terminal_shows_one_column_with_a_selector() {
        let (_root, app) = sample();
        let out = render(&app, 50, 24);
        assert!(out.contains("⇄"), "{out}");
        assert!(out.contains("TODO"), "{out}");
        assert!(out.contains("T3 Spike it"), "{out}");
        assert!(!out.contains("IMPLEMENTING"), "{out}");
    }

    #[test]
    fn the_detail_sheet_lists_dependencies_prs_and_the_task_file() {
        let (_root, mut app) = sample();
        app.col = 2;
        app.row[2] = 1;
        app.detail = true;
        let out = render(&app, 120, 40);
        assert!(out.contains("T2 · Wire UI"), "{out}");
        assert!(out.contains("Depends on"), "{out}");
        assert!(out.contains("T1 (planned)"), "{out}");
        assert!(out.contains("tasks/T2-wire-ui.md"), "{out}");

        app.col = 3;
        app.row[3] = 0;
        let out = render(&app, 120, 40);
        assert!(out.contains("https://github.com/o/api/pull/7"), "{out}");
        assert!(out.contains("waiting on design"), "{out}");
    }

    #[test]
    fn the_help_sheet_lists_the_keys() {
        let (_root, mut app) = sample();
        app.help = true;
        let out = render(&app, 120, 40);
        assert!(out.contains("Keys"), "{out}");
        assert!(out.contains("switch story"), "{out}");
    }

    #[test]
    fn an_empty_root_says_so_and_a_reload_error_is_shown_in_the_footer() {
        let root = TempDir::new().unwrap();
        let mut app = App::new(root.path().to_path_buf(), None);
        let out = render(&app, 100, 20);
        assert!(out.contains("No stories found"), "{out}");
        app.error = Some("parsing board.json failed".into());
        let out = render(&app, 100, 20);
        assert!(out.contains("parsing board.json failed"), "{out}");
    }
    fn find(out: &str, needle: &str) -> (u16, u16) {
        for (y, line) in out.lines().enumerate() {
            if let Some(byte) = line.find(needle) {
                return (line[..byte].chars().count() as u16, y as u16);
            }
        }
        panic!("{needle:?} not found in:\n{out}");
    }

    fn two_stories() -> (TempDir, App) {
        let root = TempDir::new().unwrap();
        for key in ["PROJ-1", "PROJ-2"] {
            ops::init_story(root.path(), key, &format!("Story {key}"), None).unwrap();
            store::update(root.path(), key, |b| {
                let dir = root.path().join(key);
                ops::add_task(&dir, b, "First", TaskType::Spike, vec![], vec![])?;
                ops::add_task(&dir, b, "Second", TaskType::Pr, vec![], vec![])
                    .map(|_| ())
            })
            .unwrap();
        }
        store::update(root.path(), "PROJ-1", |b| {
            for id in ["T1", "T2"] {
                for s in [Status::Planning, Status::Planned, Status::Implementing] {
                    rules::transition(b, id, s)?;
                }
            }
            b.tasks[0].prs.clear();
            rules::transition(b, "T1", Status::Done)?;
            b.tasks[1].kind = TaskType::Spike;
            rules::transition(b, "T2", Status::Done)?;
            rules::story_transition(b, storyboard::model::StoryStatus::InReview)?;
            rules::story_transition(b, storyboard::model::StoryStatus::Done)
        })
        .unwrap();
        let app = App::new(root.path().to_path_buf(), None);
        (root, app)
    }

    #[test]
    fn the_story_list_shows_rows_counts_and_the_done_toggle() {
        let (_root, mut app) = two_stories();
        let out = render(&app, 120, 30);
        for want in ["Stories", "PROJ-2", "Story PROJ-2", "[ in_progress ]", "0/2 done", "2 todo", "[ Show done ]", "1 done hidden", "q quit"] {
            assert!(out.contains(want), "missing {want:?} in:\n{out}");
        }
        assert!(!out.contains("PROJ-1"), "{out}");
        app.hide_done = false;
        let out = render(&app, 120, 30);
        assert!(out.contains("PROJ-1"), "{out}");
        assert!(out.contains("[ done ]"), "{out}");
        assert!(out.contains("2/2 done"), "{out}");
        assert!(out.contains("[ Hide done ]"), "{out}");
    }

    #[test]
    fn a_list_with_only_hidden_stories_explains_how_to_show_them() {
        let (_root, mut app) = two_stories();
        app.stories.retain(|s| s.key == "PROJ-1");
        let out = render(&app, 120, 20);
        assert!(out.contains("No stories to show"), "{out}");
        assert!(out.contains("press d"), "{out}");
    }

    #[test]
    fn clicking_a_story_row_opens_its_board() {
        let (_root, mut app) = two_stories();
        let out = render(&app, 120, 30);
        let (x, y) = find(&out, "Story PROJ-2");
        app.on_click(x, y);
        assert_eq!(app.screen, Screen::Board);
        assert_eq!(app.board.as_ref().unwrap().story.key, "PROJ-2");
    }

    #[test]
    fn clicking_the_toggle_shows_and_hides_done_stories() {
        let (_root, mut app) = two_stories();
        let out = render(&app, 120, 30);
        let (x, y) = find(&out, "[ Show done ]");
        app.on_click(x + 2, y);
        assert!(!app.hide_done);
        let out = render(&app, 120, 30);
        assert!(out.contains("PROJ-1"));
        let (x, y) = find(&out, "[ Hide done ]");
        app.on_click(x, y);
        assert!(app.hide_done);
    }

    #[test]
    fn clicking_back_returns_to_the_list() {
        let (_root, mut app) = sample();
        let out = render(&app, 200, 30);
        let (x, y) = find(&out, "‹ Stories");
        app.on_click(x + 1, y);
        assert_eq!(app.screen, Screen::List);
    }

    #[test]
    fn clicking_a_card_selects_it_then_opens_it_and_clicking_outside_closes_it() {
        let (_root, mut app) = sample();
        let out = render(&app, 200, 30);
        let (x, y) = find(&out, "T2 Wire UI");
        app.on_click(x, y);
        assert_eq!(app.selected_task().unwrap().id, "T2");
        assert!(!app.detail);
        render(&app, 200, 30);
        app.on_click(x, y);
        assert!(app.detail);
        let out = render(&app, 200, 30);
        assert!(out.contains("Depends on"), "{out}");
        app.on_click(x, y + 1);
        assert!(app.detail, "a click inside the sheet must not close it");
        app.on_click(0, 0);
        assert!(!app.detail);
    }

    #[test]
    fn clicking_a_column_title_selects_that_column() {
        let (_root, mut app) = sample();
        let out = render(&app, 200, 30);
        let (x, y) = find(&out, "DONE · 0");
        app.on_click(x, y);
        assert_eq!(app.col, 4);
    }

    #[test]
    fn scrolling_moves_the_selection_in_the_column_under_the_pointer() {
        let (_root, mut app) = sample();
        let out = render(&app, 200, 30);
        let (x, y) = find(&out, "T1 Add endpoint");
        app.on_scroll(x, y, 1);
        assert_eq!(app.selected_task().unwrap().id, "T2");
        app.on_scroll(x, y, -1);
        assert_eq!(app.selected_task().unwrap().id, "T1");
    }

    #[test]
    fn the_compact_title_arrows_move_between_columns() {
        let (_root, mut app) = sample();
        let out = render(&app, 50, 24);
        let (_, y) = find(&out, "⇄");
        app.on_click(46, y);
        assert_eq!(app.col, 1);
        render(&app, 50, 24);
        app.on_click(3, y);
        assert_eq!(app.col, 0);
    }

    #[test]
    fn the_board_header_has_a_back_link_and_the_story_counter_counts_visible_stories() {
        let (_root, mut app) = two_stories();
        app.open_story("PROJ-2");
        let out = render(&app, 140, 20);
        assert!(out.contains("‹ Stories"), "{out}");
        assert!(!out.contains("1/2"), "only one story is visible: {out}");
        app.hide_done = false;
        let out = render(&app, 140, 20);
        assert!(out.contains("‹ 2/2 ›"), "{out}");
    }
    fn check(name: &str, state: CheckState) -> Check {
        Check {
            name: name.into(),
            workflow: Some("CI".into()),
            state,
            started_at: None,
            completed_at: None,
        }
    }

    fn feed() -> Vec<PullRequest> {
        let checks = vec![
            check("lint", CheckState::Passed),
            check("unit", CheckState::Failed),
            check("build", CheckState::Running),
            check("test", CheckState::Running),
            check("deploy", CheckState::Queued),
            check("docs", CheckState::Passed),
            check("types", CheckState::Passed),
        ];
        let pr = |repo: &str, number: u64, title: &str, checks: Vec<Check>| PullRequest {
            repo: repo.into(),
            number,
            title: title.into(),
            url: format!("https://github.com/{repo}/pull/{number}"),
            is_draft: number == 99,
            labels: vec!["bug".into()],
            review: Review::Approved,
            comments: 3,
            updated_at: "2026-10-08T01:00:00Z".into(),
            checks,
        };
        vec![
            pr("acme/widgets", 12, "Add notices", checks),
            pr("acme/api", 99, "Unrelated chore", vec![]),
            pr("acme/web", 4, "Third thing", vec![]),
        ]
    }

    fn with_prs() -> (TempDir, App) {
        let (root, mut app) = two_stories();
        store::update(root.path(), "PROJ-2", |b| {
            ops::add_pr(b, "T1", "widgets", "https://github.com/acme/widgets/pull/12", storyboard::model::PrState::Draft)
        })
        .unwrap();
        app.reload();
        app.apply_prs(Ok(feed()));
        (root, app)
    }

    #[test]
    fn the_main_screen_has_a_bottom_panel_with_every_open_pr_and_its_ci_stages() {
        let (_root, app) = with_prs();
        let out = render(&app, 170, 44);
        for want in [
            "PULL REQUESTS · 3", "updated", "acme/widgets#12", "Add notices", "PROJ-2 · T1",
            "acme/api#99", "Unrelated chore", "approved", "✎ 3", " ago", "[bug]",
            "✓ 3", "✗ 1", "◔ 2", "● 1", "◔ CI / build", "◔ CI / test", "● CI / deploy",
            "✗ CI / unit", "no checks", "[READY]", "[DRAFT]",
        ] {
            assert!(out.contains(want), "missing {want:?} in:\n{out}");
        }
        assert!(out.contains("Tab PRs"), "{out}");
        for gone in ["running: ", "queued: ", "failed: ", "CI / lint"] {
            assert!(!out.contains(gone), "{gone:?} should not appear in:\n{out}");
        }
    }

    fn line_of<'a>(out: &'a str, needle: &str) -> &'a str {
        out.lines().find(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle:?} not in:\n{out}"))
    }

    #[test]
    fn ci_stages_are_listed_vertically_one_per_line_in_one_column() {
        let (_root, app) = with_prs();
        let out = render(&app, 170, 44);
        let (xb, yb) = find(&out, "◔ CI / build");
        let (xt, yt) = find(&out, "◔ CI / test");
        let (xd, yd) = find(&out, "● CI / deploy");
        let (xu, yu) = find(&out, "✗ CI / unit");
        assert!(xb == xt && xt == xd && xd == xu, "stages share a column:\n{out}");
        assert_eq!((yt, yd, yu), (yb + 1, yb + 2, yb + 3), "running, queued, failed in order:\n{out}");
    }

    #[test]
    fn pending_stages_use_a_filled_orange_circle() {
        use ratatui::style::Color;
        let (_root, app) = with_prs();
        let mut term = Terminal::new(TestBackend::new(170, 44)).unwrap();
        term.draw(|f| draw(f, &app)).unwrap();
        let buf = term.backend().buffer().clone();
        let text = render(&app, 170, 44);
        let (x, y) = find(&text, "● CI / deploy");
        assert_eq!(buf[(x, y)].symbol(), "●");
        assert_eq!(buf[(x, y)].fg, Color::Indexed(208), "pending is orange");
        let (sx, sy) = find(&text, "● 1");
        assert_eq!(buf[(sx, sy)].fg, Color::Indexed(208), "the summary count matches");
        assert!(!text.contains("○"), "no hollow circle is left:\n{text}");
    }

    #[test]
    fn draft_and_ready_pull_requests_are_clearly_different() {
        let (_root, app) = with_prs();
        let out = render(&app, 170, 44);
        let ready = line_of(&out, "Add notices");
        let draft = line_of(&out, "Unrelated chore");
        assert!(ready.contains("[READY]") && !ready.contains("[DRAFT]"), "{ready}");
        assert!(draft.contains("[DRAFT]") && !draft.contains("[READY]"), "{draft}");
    }

    #[test]
    fn a_divider_separates_one_pull_request_from_the_next() {
        let (_root, app) = with_prs();
        let out = render(&app, 170, 44);
        let first = find(&out, "acme/widgets#12").1 as usize;
        let second = find(&out, "acme/api#99").1 as usize;
        let between: Vec<&str> = out.lines().skip(first + 1).take(second - first - 1).collect();
        assert!(between.iter().any(|l| l.contains("────────")), "{out}");
        assert!(
            !between.is_empty() && between.last().unwrap().contains("────────"),
            "the divider sits directly above the next PR:\n{out}"
        );
    }

    fn busy_pr(number: u64, stages: usize) -> PullRequest {
        let mut pr = feed().remove(1);
        pr.number = number;
        pr.repo = format!("acme/svc{number}");
        pr.url = format!("https://github.com/acme/svc{number}/pull/{number}");
        pr.title = format!("Busy {number}");
        pr.checks = (0..stages).map(|n| check(&format!("job{n:02}"), CheckState::Running)).collect();
        pr
    }

    #[test]
    fn a_long_stage_list_is_capped_and_the_selected_pr_stays_fully_visible() {
        let (_root, mut app) = two_stories();
        app.apply_prs(Ok(vec![busy_pr(1, 12), busy_pr(2, 3), busy_pr(3, 3)]));
        let out = render(&app, 140, 34);
        assert!(out.contains("CI / job07"), "{out}");
        assert!(!out.contains("CI / job08"), "{out}");
        assert!(out.contains("… +4 more"), "{out}");
        app.focus = Focus::Prs;
        app.pr_sel = 2;
        let out = render(&app, 140, 34);
        assert!(out.contains("acme/svc3#3"), "{out}");
        assert!(out.contains("CI / job02"), "the whole selected row is visible:\n{out}");
        assert!(!out.contains("acme/svc1#1"), "{out}");
    }

    #[test]
    fn the_story_board_has_a_panel_with_only_that_storys_prs() {
        let (_root, mut app) = with_prs();
        app.open_story("PROJ-2");
        let out = render(&app, 170, 44);
        assert!(out.contains("STORY PULL REQUESTS · 1"), "{out}");
        assert!(out.contains("acme/widgets#12"), "{out}");
        assert!(out.contains("◔ CI / build"), "{out}");
        assert!(!out.contains("acme/api#99"), "{out}");
        app.open_story("PROJ-1");
        let out = render(&app, 170, 44);
        assert!(out.contains("No pull requests recorded"), "{out}");
    }

    #[test]
    fn a_recorded_pr_that_github_does_not_list_still_shows_from_the_board() {
        let (root, mut app) = with_prs();
        store::update(root.path(), "PROJ-2", |b| {
            ops::add_pr(b, "T2", "widgets", "https://github.com/acme/widgets/pull/5", storyboard::model::PrState::Merged)
        })
        .unwrap();
        app.reload();
        app.open_story("PROJ-2");
        let out = render(&app, 170, 44);
        assert!(out.contains("STORY PULL REQUESTS · 2"), "{out}");
        assert!(out.contains("acme/widgets#5"), "{out}");
        assert!(out.contains("[MERGED]"), "{out}");
    }

    #[test]
    fn the_panel_explains_loading_empty_and_error_states() {
        let (_root, mut app) = two_stories();
        let out = render(&app, 140, 40);
        assert!(out.contains("Loading pull requests"), "{out}");
        app.apply_prs(Err("gh failed: not logged in".into()));
        let out = render(&app, 140, 40);
        assert!(out.contains("gh failed: not logged in"), "{out}");
        app.apply_prs(Ok(vec![]));
        let out = render(&app, 140, 40);
        assert!(out.contains("No open pull requests match"), "{out}");
    }

    #[test]
    fn a_short_terminal_hides_the_panel() {
        let (_root, app) = with_prs();
        let out = render(&app, 100, 14);
        assert!(!out.contains("PULL REQUESTS"), "{out}");
    }

    #[test]
    fn clicking_a_pr_row_selects_it_then_opens_the_sheet_and_the_button_opens_the_browser() {
        let (_root, mut app) = with_prs();
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let sink = log.clone();
        app.opener = Box::new(move |url| sink.borrow_mut().push(url.to_string()));
        let out = render(&app, 170, 44);
        let (x, y) = find(&out, "Unrelated chore");
        app.on_click(x, y);
        assert_eq!((app.focus, app.pr_sel, app.pr_sheet), (Focus::Prs, 1, false));
        render(&app, 170, 44);
        app.on_click(x, y);
        assert!(app.pr_sheet);
        let out = render(&app, 170, 44);
        let (bx, by) = find(&out, "[ o Open in browser ]");
        app.on_click(bx + 3, by);
        assert_eq!(*log.borrow(), vec!["https://github.com/acme/api/pull/99".to_string()]);
        assert!(app.pr_sheet, "opening the browser keeps the sheet");
        app.on_click(0, 0);
        assert!(!app.pr_sheet);
    }

    #[test]
    fn the_pr_sheet_lists_checks_running_first_then_queued_failed_and_passed() {
        let (_root, mut app) = with_prs();
        app.focus = Focus::Prs;
        app.pr_sel = 0;
        app.pr_sheet = true;
        let out = render(&app, 150, 50);
        let y = |s: &str| find(&out, s).1;
        assert!(y("CI / build") < y("CI / deploy"), "{out}");
        assert!(y("CI / deploy") < y("CI / unit"), "{out}");
        assert!(y("CI / unit") < y("CI / lint"), "{out}");
        for want in ["acme/widgets#12", "Add notices", "Checks (7)", "https://github.com/acme/widgets/pull/12", "PROJ-2 · T1", "Ready for review"] {
            assert!(out.contains(want), "missing {want:?} in:\n{out}");
        }
    }

    #[test]
    fn the_pr_sheet_says_when_a_pull_request_is_a_draft() {
        let (_root, mut app) = with_prs();
        app.focus = Focus::Prs;
        app.pr_sel = 1;
        app.pr_sheet = true;
        let out = render(&app, 150, 50);
        assert!(out.contains("Draft (not ready for review)"), "{out}");
    }

    #[test]
    fn scrolling_over_the_pr_panel_moves_the_pr_selection() {
        let (_root, mut app) = with_prs();
        let out = render(&app, 170, 44);
        let (x, y) = find(&out, "acme/widgets#12");
        app.on_scroll(x, y, 1);
        assert_eq!((app.focus, app.pr_sel), (Focus::Prs, 1));
        app.on_scroll(x, y, -1);
        assert_eq!(app.pr_sel, 0);
        assert_eq!(app.list_sel, 0, "the story list is untouched");
    }
}
