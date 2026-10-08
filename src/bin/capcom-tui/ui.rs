use crate::app::{App, BottomTab, Column, Focus, Geometry, Screen, StorySummary, Target, STATUS_ORDER};
use crate::{panel, runs_ui};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use ratatui::Frame;
use capcom::model::{Board, PrState, Status, StoryStatus, Task};
use capcom::rules;

type Hits = Vec<(Rect, Target)>;

const CARD_HEIGHT: u16 = 5;
const STORY_ROW_HEIGHT: u16 = 4;
const COMPACT_BELOW: u16 = 60;
const WIDE_FROM: u16 = 150;
const RUNS_SMALL_WIDTH: u16 = 30;
const BACK_LABEL: &str = "‹ Stories";

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let mut hits: Hits = Vec::new();
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)])
        .split(area);
    let wide = area.width >= WIDE_FROM;
    let panel_height = bottom_height(app, rows[1].height, wide, area.width);
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
    app.geometry.set(Geometry {
        body_y: rows[1].y,
        body_h: rows[1].height,
        bottom_x: panel_area.map_or(area.x, |b| b.x),
        bottom_w: panel_area.map_or(area.width, |b| b.width),
    });
    if let Some(bottom) = panel_area {
        if wide {
            let small = runs_small(app);
            let halves = if small {
                Layout::horizontal([Constraint::Min(20), Constraint::Length(RUNS_SMALL_WIDTH)]).split(bottom)
            } else {
                Layout::horizontal([Constraint::Percentage(app.split_pct), Constraint::Percentage(100 - app.split_pct)])
                    .split(bottom)
            };
            panel::draw_panel(f, halves[0], app, &mut hits);
            runs_ui::draw_panel(f, halves[1], app, &mut hits);
            hits.push((Rect::new(bottom.x, bottom.y, bottom.width, 1), Target::HeightHandle));
            let strip = Rect::new(halves[1].x.saturating_sub(1), halves[1].y + 1, 2, halves[1].height.saturating_sub(1));
            hits.push((strip, Target::SplitHandle));
        } else {
            let tab_row = Rect::new(bottom.x, bottom.y, bottom.width, 1);
            let rest = Rect::new(bottom.x, bottom.y + 1, bottom.width, bottom.height.saturating_sub(1));
            draw_tabs(f, tab_row, app, &mut hits);
            match app.tab {
                BottomTab::Prs => panel::draw_panel(f, rest, app, &mut hits),
                BottomTab::Runs => runs_ui::draw_panel(f, rest, app, &mut hits),
            }
        }
    }
    // the refresh labels sit on the border that the height handle also covers; they win
    let refresh: Vec<_> = hits.iter().filter(|(_, t)| *t == Target::Refresh).cloned().collect();
    hits.extend(refresh);
    draw_footer(f, rows[2], app);
    if app.help {
        draw_help(f, area, &mut hits);
    } else if app.pr_sheet {
        panel::draw_sheet(f, area, app, &mut hits);
    } else if app.run_sheet {
        runs_ui::draw_sheet(f, area, app, &mut hits);
    } else if app.detail && app.screen == Screen::Board {
        draw_detail(f, area, app, &mut hits);
    }
    if app.confirm.is_some() {
        panel::draw_confirm(f, area, app, &mut hits);
    }
    if app.new_story.is_some() {
        draw_new_story(f, area, app, &mut hits);
    }
    *app.hits.borrow_mut() = hits;
}

/// An empty (or switched off) manual runs panel shrinks to a narrow strip, unless it has an
/// error or a warning that needs room to be read.
fn runs_small(app: &App) -> bool {
    let quiet = !app.split_pinned && app.runs.error.is_none() && app.runs.warnings.is_empty();
    quiet && (app.runs.disabled || (app.runs.loaded && app.visible_runs().is_empty()))
}

/// Height of the bottom area for a body of the given height; 0 hides it.
fn bottom_height(app: &App, body: u16, wide: bool, width: u16) -> u16 {
    if let Some(pct) = app.bottom_pct {
        return if body >= 12 { (body * pct / 100).clamp(5, body - 4) } else { 0 };
    }
    match app.screen {
        Screen::List if body >= 16 => (body * 45 / 100).max(8),
        Screen::Board if body >= 18 => {
            let pr_width = if !wide {
                width
            } else if runs_small(app) {
                width.saturating_sub(RUNS_SMALL_WIDTH)
            } else {
                (u32::from(width) * u32::from(app.split_pct) / 100) as u16
            };
            let prs = panel::content_height(app, usize::from(pr_width.saturating_sub(2)));
            let runs = runs_ui::content_height(app);
            let content = if wide {
                prs.max(runs)
            } else if app.tab == BottomTab::Prs {
                prs
            } else {
                runs
            };
            let errors = u16::from(app.prs.error.is_some() || app.runs.error.is_some());
            let extra = 2 + errors + u16::from(!wide);
            (content.max(1) + extra).min(body * 40 / 100).max(5)
        }
        _ => 0,
    }
}

fn draw_tabs(f: &mut Frame, area: Rect, app: &App, hits: &mut Hits) {
    let runs = app.visible_runs();
    let active = runs.iter().filter(|r| r.is_active()).count();
    let prs = format!("[ Pull requests · {} ]", app.pr_rows().len());
    let runs_label = if active > 0 {
        format!("[ Manual runs · {} ({active} active) ]", runs.len())
    } else {
        format!("[ Manual runs · {} ]", runs.len())
    };
    let style = |tab: BottomTab| {
        if app.tab == tab {
            let color = if (tab == BottomTab::Prs && app.focus == Focus::Prs) || (tab == BottomTab::Runs && app.focus == Focus::Runs) {
                Color::Blue
            } else {
                Color::White
            };
            Style::new().fg(color).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(Color::DarkGray)
        }
    };
    let prs_width = prs.chars().count() as u16;
    let runs_width = runs_label.chars().count() as u16;
    hits.push((Rect::new(area.x, area.y, prs_width.min(area.width), 1), Target::Tab(BottomTab::Prs)));
    let runs_x = area.x + prs_width + 2;
    if runs_x < area.x + area.width {
        let width = runs_width.min(area.x + area.width - runs_x);
        hits.push((Rect::new(runs_x, area.y, width, 1), Target::Tab(BottomTab::Runs)));
    }
    let used = prs_width + 2 + runs_width;
    if area.width > used + 2 {
        let free = Rect::new(area.x + used + 2, area.y, area.width - used - 2, 1);
        hits.push((free, Target::HeightHandle));
    }
    let line = Line::from(vec![
        Span::styled(prs, style(BottomTab::Prs)),
        Span::raw("  "),
        Span::styled(runs_label, style(BottomTab::Runs)),
    ]);
    f.render_widget(Paragraph::new(line), area);
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

/// The brand, the mission clock and the go/no-go light (no-go while any open PR has a failed check).
fn brand(app: &App) -> Vec<Span<'static>> {
    let secs = app.started.elapsed().as_secs();
    let mut spans = vec![
        Span::styled("🚀 CAPCOM", Style::new().fg(panel::ORANGE).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("  T+{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60),
            Style::new().fg(Color::DarkGray),
        ),
    ];
    if app.prs.loaded && !app.prs.disabled {
        let failing = app.prs.items.iter().any(|p| p.counts().failed > 0);
        let (text, color) = if failing { ("NO-GO", Color::Red) } else { ("GO", Color::Green) };
        spans.push(Span::raw("  "));
        spans.push(Span::styled(format!("● {text}"), Style::new().fg(color).add_modifier(Modifier::BOLD)));
    }
    spans
}

const NEW_STORY_BUTTON: &str = "[ n + new story ]";

fn draw_list_header(f: &mut Frame, area: Rect, app: &App, hits: &mut Hits) {
    let dim = Style::new().fg(Color::DarkGray);
    let mut left = brand(app);
    left.push(Span::styled("  STORIES", Style::new().add_modifier(Modifier::BOLD)));
    left.push(Span::raw("  "));
    let new_x = area.x + Line::from(left.clone()).width() as u16;
    left.push(Span::styled(NEW_STORY_BUTTON, Style::new().fg(Color::Cyan)));
    let new_w = NEW_STORY_BUTTON.chars().count() as u16;
    if new_x + new_w < area.x + area.width {
        hits.push((Rect::new(new_x, area.y, new_w, 1), Target::NewStory));
    }
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
    ];
    left.extend(brand(app));
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
            "Create one with: capcom init PROJ-123 --title \"...\"",
            Style::new().fg(Color::DarkGray),
        )),
    ];
    if let Some(err) = &app.error {
        lines.push(Line::from(Span::styled(err.clone(), Style::new().fg(Color::Red))));
    }
    f.render_widget(Paragraph::new(lines).centered(), area);
}

fn draw_new_story(f: &mut Frame, area: Rect, app: &App, hits: &mut Hits) {
    let Some(text) = &app.new_story else {
        return;
    };
    let w = (area.width * 60 / 100).max(46).min(area.width);
    let h = 9.min(area.height);
    let rect = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
    let room = (w as usize).saturating_sub(6);
    let shown: String = text.chars().rev().take(room).collect::<Vec<_>>().into_iter().rev().collect();
    let dim = Style::new().fg(Color::DarkGray);
    let mut lines = vec![
        Line::raw(""),
        Line::from(Span::raw(" Paste a Jira key or link, then press Enter:")),
        Line::from(vec![
            Span::raw(" "),
            Span::styled(format!("{shown}▌"), Style::new().add_modifier(Modifier::BOLD)),
        ]),
        Line::raw(""),
    ];
    if app.herdr.is_some() {
        lines.push(Line::from(Span::styled(" Starts /story-break-down in a new Herdr workspace.", dim)));
    } else {
        lines.push(Line::from(Span::styled(" Not running inside Herdr: nothing can be started.", Style::new().fg(Color::Red))));
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(panel::ORANGE))
        .title(Span::styled(" New story ", Style::new().add_modifier(Modifier::BOLD)));
    f.render_widget(Clear, rect);
    f.render_widget(Paragraph::new(lines).block(block), rect);
    let y = rect.y + rect.height.saturating_sub(2);
    let start = Rect::new(rect.x + 2, y, START_BUTTON.chars().count() as u16, 1);
    let cancel = Rect::new(start.x + start.width + 2, y, CANCEL_BUTTON.chars().count() as u16, 1);
    f.render_widget(Paragraph::new(Span::styled(START_BUTTON, Style::new().fg(Color::Green).add_modifier(Modifier::BOLD))), start);
    f.render_widget(Paragraph::new(Span::styled(CANCEL_BUTTON, Style::new().fg(Color::Cyan))), cancel);
    hits.push((rect, Target::Sheet));
    hits.push((start, Target::NewStoryStart));
    hits.push((cancel, Target::NewStoryCancel));
}

const START_BUTTON: &str = "[ Enter Start ]";
const CANCEL_BUTTON: &str = "[ Esc Cancel ]";

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    if let Some(err) = &app.error {
        let text = trunc(&format!("! {err}"), area.width as usize);
        f.render_widget(Paragraph::new(Span::styled(text, Style::new().fg(Color::Red))), area);
        return;
    }
    let dim = Style::new().fg(Color::DarkGray);
    let keys: &[&str] = match app.screen {
        Screen::List => &["↑↓ story", "⏎ open", "n new story", "d show/hide done", "Tab PRs/runs", "? help", "q quit"],
        Screen::Board => &[
            "←→ column", "↑↓ card", "[ ] story", "⏎ detail", "p plan", "i implement", "Tab PRs/runs",
            "Esc stories", "? help", "q quit",
        ],
    };
    let mut left = Vec::new();
    for key in keys {
        left.push(Span::styled(format!("[ {key} ]"), dim));
        left.push(Span::raw(" "));
    }
    let right = match app.current_notice() {
        Some(notice) => vec![Span::styled(notice.to_string(), Style::new().fg(panel::ORANGE).add_modifier(Modifier::BOLD))],
        None => vec![Span::styled(format!("updated {}", app.updated), dim)],
    };
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
        draw_column(f, area, &cols[i], i, true, false, app.row[i], board, Some(title), hits);
        return;
    }
    let rects = Layout::horizontal((0..cols.len()).map(|i| Constraint::Fill(app.col_weights[i]))).split(area);
    for (i, col) in cols.iter().enumerate() {
        hits.push((rects[i], Target::Column(i)));
        draw_column(f, rects[i], col, i, i == app.col, app.drop_column() == Some(i), app.row[i], board, None, hits);
    }
    for i in 0..cols.len().saturating_sub(1) {
        let strip = Rect::new(rects[i + 1].x.saturating_sub(1), rects[i + 1].y + 1, 2, rects[i + 1].height.saturating_sub(1));
        hits.push((strip, Target::ColumnHandle(i, rects[i].x, rects[i + 1].x + rects[i + 1].width)));
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_column(
    f: &mut Frame,
    area: Rect,
    col: &Column,
    index: usize,
    selected: bool,
    drop: bool,
    sel_row: usize,
    board: &Board,
    title: Option<String>,
    hits: &mut Hits,
) {
    let accent = if drop {
        panel::ORANGE
    } else if selected {
        Color::Blue
    } else {
        Color::DarkGray
    };
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
        row("Tab", "focus: board or list, then PRs, then manual runs".into()),
        row("o", "open the selected pull request or run in the browser".into()),
        row("n", "new story: paste a Jira key or link to start its breakdown in Herdr".into()),
        row("p / i", "start planning / implementing the selected task in Herdr (or drag the card)".into()),
        row("c", "copy the selected pull request's link".into()),
        row("m", "mark the selected draft ready for review (asks first)".into()),
        row("d", "show or hide completed stories (list)".into()),
        row("Esc  b", "back to the list, or close a sheet".into()),
        row("r", "reload now (it also reloads by itself)".into()),
        row("< >", "make the PR panel narrower or wider (or drag the divider)".into()),
        row("+ -", "make the bottom panels taller or shorter (or drag their top edge)".into()),
        row(", .", "make the selected kanban column narrower or wider (or drag a column border)".into()),
        row("=", "reset all panel sizes".into()),
        row("?", "toggle this help".into()),
        row("q  Ctrl-C", "quit".into()),
        Line::raw(""),
        row("Mouse", "click a story, column or card; a PR or run opens on GitHub, [ details ] opens its sheet, [ open ] a stage; wheel scrolls".into()),
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
    use crate::app::{App, BottomTab, Focus, Screen};
    use ratatui::crossterm::event::KeyCode;
    use crate::runs::{Batch, Job, Run, RunState};
    use crate::prs::{Check, CheckState, PullRequest, Review};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use capcom::model::{Status, TaskType};
    use capcom::{ops, rules, store};
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
            ops::add_pr(b, "T4", "api", "https://github.com/o/api/pull/7", capcom::model::PrState::Draft)?;
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
            rules::story_transition(b, capcom::model::StoryStatus::InReview)?;
            rules::story_transition(b, capcom::model::StoryStatus::Done)
        })
        .unwrap();
        let app = App::new(root.path().to_path_buf(), None);
        (root, app)
    }

    #[test]
    fn the_story_list_shows_rows_counts_and_the_done_toggle() {
        let (_root, mut app) = two_stories();
        let out = render(&app, 120, 30);
        for want in ["STORIES", "PROJ-2", "Story PROJ-2", "[ in_progress ]", "0/2 done", "2 todo", "[ Show done ]", "1 done hidden", "q quit"] {
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
        assert!(!app.detail, "a press might be the start of a drag, so the sheet opens on release");
        app.on_release();
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
            url: (name != "docs").then(|| format!("https://github.com/acme/widgets/actions/runs/1/job/{name}")),
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
            ops::add_pr(b, "T1", "widgets", "https://github.com/acme/widgets/pull/12", capcom::model::PrState::Draft)
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
            "acme/api#99", "Unrelated chore", "approved", "✎ 3", " ago", "[bug]", "[ details ]",
            "✓ 3", "✗ 1", "◔ 2", "● 1", "◔ CI / build", "◔ CI / test", "● CI / deploy",
            "✗ CI / unit", "no checks", "[READY]", "[DRAFT]",
        ] {
            assert!(out.contains(want), "missing {want:?} in:\n{out}");
        }
        assert!(out.contains("Tab PRs/runs"), "{out}");
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
        let ready = line_of(&out, "acme/widgets#12");
        let draft = line_of(&out, "acme/api#99");
        assert!(ready.contains("[READY]") && !ready.contains("[DRAFT]"), "{ready}");
        assert!(draft.contains("[DRAFT]") && !draft.contains("[READY]"), "{draft}");
    }

    #[test]
    fn a_divider_separates_one_pull_request_from_the_next() {
        let (_root, app) = with_prs();
        let out = render(&app, 170, 44);
        let first = find(&out, "Add notices").1 as usize;
        let second = find(&out, "Unrelated chore").1 as usize;
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
        let out = render(&app, 140, 36);
        assert!(out.contains("CI / job07"), "{out}");
        assert!(!out.contains("CI / job08"), "{out}");
        assert!(out.contains("… +4 more"), "{out}");
        app.focus = Focus::Prs;
        app.pr_sel = 2;
        let out = render(&app, 140, 36);
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
            ops::add_pr(b, "T2", "widgets", "https://github.com/acme/widgets/pull/5", capcom::model::PrState::Merged)
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
    fn clicking_a_pr_opens_it_in_the_browser_and_selects_it() {
        let (_root, mut app) = with_prs();
        let log = recorder(&mut app);
        let out = render(&app, 170, 44);
        let (x, y) = find(&out, "Unrelated chore");
        app.on_click(x, y);
        assert_eq!(*log.borrow(), vec!["https://github.com/acme/api/pull/99".to_string()]);
        assert_eq!((app.focus, app.pr_sel, app.pr_sheet), (Focus::Prs, 1, false));
        render(&app, 170, 44);
        app.on_click(x, y);
        assert_eq!(log.borrow().len(), 2, "every click opens it again, the sheet stays closed");
        assert!(!app.pr_sheet);
    }

    #[test]
    fn the_details_button_opens_the_pr_sheet_without_opening_the_browser() {
        let (_root, mut app) = with_prs();
        let log = recorder(&mut app);
        let out = render(&app, 170, 44);
        assert_eq!(out.matches("[ details ]").count(), 3, "one button per pull request:\n{out}");
        let (x, y) = find(&out, "[ details ]");
        app.on_click(x + 2, y);
        assert!(app.pr_sheet && app.focus == Focus::Prs);
        assert_eq!(app.pr_sel, 0);
        assert!(log.borrow().is_empty(), "details does not open the browser");
        let out = render(&app, 170, 44);
        assert!(out.contains("Ready for review"), "{out}");
        let (bx, by) = find(&out, "[ o Open in browser ]");
        app.on_click(bx + 3, by);
        assert_eq!(*log.borrow(), vec!["https://github.com/acme/widgets/pull/12".to_string()]);
        assert!(app.pr_sheet, "opening the browser keeps the sheet");
        app.on_click(0, 0);
        assert!(!app.pr_sheet);
    }

    #[test]
    fn the_details_button_of_a_later_pr_opens_that_prs_sheet() {
        let (_root, mut app) = with_prs();
        let out = render(&app, 170, 44);
        let y_second = find(&out, "Unrelated chore").1 + 2;
        let x = out.lines().nth(y_second as usize).unwrap().find("[ details ]").expect("button on the CI line");
        let x = out.lines().nth(y_second as usize).unwrap()[..x].chars().count() as u16;
        app.on_click(x + 1, y_second);
        assert!(app.pr_sheet);
        assert_eq!(app.pr_sel, 1);
        let out = render(&app, 170, 44);
        assert!(out.contains("Draft (not ready for review)"), "{out}");
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

    fn copies(app: &mut App) -> std::rc::Rc<std::cell::RefCell<Vec<String>>> {
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let sink = log.clone();
        app.copier = Box::new(move |text| sink.borrow_mut().push(text.to_string()));
        log
    }

    fn ready_calls(app: &mut App) -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let sink = log.clone();
        app.readier = std::sync::Arc::new(move |url| {
            sink.lock().unwrap().push(url.to_string());
            Ok(())
        });
        log
    }

    #[test]
    fn the_copy_button_copies_the_pr_link_without_opening_the_browser() {
        let (_root, mut app) = with_prs();
        let opened = recorder(&mut app);
        let copied = copies(&mut app);
        let out = render(&app, 170, 44);
        assert_eq!(out.matches("[ copy ]").count(), 3, "one per pull request:\n{out}");
        let (x, y) = find(&out, "[ copy ]");
        app.on_click(x + 2, y);
        assert_eq!(*copied.borrow(), vec!["https://github.com/acme/widgets/pull/12".to_string()]);
        assert!(opened.borrow().is_empty());
        assert!(render(&app, 170, 44).contains("copied acme/widgets#12"));
    }

    #[test]
    fn the_c_key_copies_the_selected_pr() {
        let (_root, mut app) = with_prs();
        let copied = copies(&mut app);
        app.focus = Focus::Prs;
        app.pr_sel = 1;
        app.on_key(KeyCode::Char('c'), false);
        assert_eq!(copied.borrow().len(), 1);
        assert!(copied.borrow()[0].contains("/pull/"));
    }

    #[test]
    fn clicking_draft_asks_first_and_only_yes_marks_it_ready() {
        let (_root, mut app) = with_prs();
        let calls = ready_calls(&mut app);
        let out = render(&app, 170, 44);
        let (x, y) = find(&out, "[DRAFT]");
        app.on_click(x + 2, y);
        assert!(app.confirm.is_some());
        assert!(calls.lock().unwrap().is_empty(), "nothing happens before the answer");
        let out = render(&app, 170, 44);
        assert!(out.contains("Mark ready for review?") && out.contains("Unrelated chore"), "{out}");
        let (nx, ny) = find(&out, "[ n Cancel ]");
        app.on_click(nx + 2, ny);
        assert!(app.confirm.is_none() && calls.lock().unwrap().is_empty());
        let (x, y) = find(&render(&app, 170, 44), "[DRAFT]");
        app.on_click(x + 2, y);
        let out = render(&app, 170, 44);
        let (yx, yy) = find(&out, "[ y Mark ready ]");
        app.on_click(yx + 2, yy);
        assert!(app.confirm.is_none());
        for _ in 0..100 {
            app.poll_ready();
            if !calls.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn keys_answer_the_confirmation_and_ready_prs_have_no_clickable_badge() {
        let (_root, mut app) = with_prs();
        let calls = ready_calls(&mut app);
        app.focus = Focus::Prs;
        app.pr_sel = 0;
        app.on_key(KeyCode::Char('m'), false);
        assert!(app.confirm.is_none(), "a ready pull request is not asked about");
        let out = render(&app, 170, 44);
        let (x, y) = find(&out, "[READY]");
        app.on_click(x + 2, y);
        assert!(app.confirm.is_none());
        app.pr_sel = 1;
        app.on_key(KeyCode::Char('m'), false);
        assert!(app.confirm.is_some());
        app.on_key(KeyCode::Esc, false);
        assert!(app.confirm.is_none() && calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_failed_mark_ready_is_reported() {
        let (_root, mut app) = with_prs();
        app.readier = std::sync::Arc::new(|_| Err("not permitted".into()));
        app.pr_sel = 1;
        app.ask_mark_ready(1);
        app.confirm_ready();
        for _ in 0..100 {
            app.poll_ready();
            if app.current_notice().is_some_and(|n| n.contains("could not")) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.current_notice().unwrap().contains("not permitted"));
    }

    #[test]
    fn the_header_has_the_brand_a_mission_clock_and_a_go_no_go_light() {
        let (_root, mut app) = two_stories();
        let out = render(&app, 120, 30);
        assert!(out.contains("🚀") && out.contains("CAPCOM") && out.contains("T+00:00:"), "{out}");
        assert!(!out.contains("GO"), "no light before the pull requests have loaded:\n{out}");
        app.apply_prs(Ok(vec![]));
        assert!(render(&app, 120, 30).contains("● GO"));
        app.apply_prs(Ok(feed()));
        let out = render(&app, 120, 30);
        assert!(out.contains("● NO-GO"), "a failed check is a no-go:\n{out}");
        app.on_key(KeyCode::Enter, false);
        assert!(render(&app, 120, 30).contains("CAPCOM"), "the board header has it too");
    }

    #[test]
    fn the_updated_label_of_each_panel_is_a_refresh_button() {
        let (_root, mut app) = with_prs();
        let (_tx, rx) = std::sync::mpsc::channel();
        let (wake_tx, wake_rx) = std::sync::mpsc::channel();
        app.attach_feed(rx, wake_tx);
        let out = render(&app, 170, 44);
        assert_eq!(out.matches("↻ updated").count(), 1, "the PR panel has it:\n{out}");
        let (x, y) = find(&out, "↻ updated");
        app.on_click(x + 3, y);
        assert!(wake_rx.try_recv().is_ok(), "clicking the label refreshes at once");
        assert!(app.pr_sheet == false && app.focus == Focus::Main, "and does nothing else");
        app.apply_runs(Ok(runs_feed()));
        let out = render(&app, 170, 44);
        assert_eq!(out.matches("↻ updated").count(), 2, "the runs panel has it too:\n{out}");
        let line = out.lines().nth(y as usize).unwrap();
        let second = line.rmatches("↻ updated").next().map(|_| line.rfind("↻ updated").unwrap()).unwrap();
        let x2 = line[..second].chars().count() as u16;
        app.on_click(x2 + 3, y);
        assert!(wake_rx.try_recv().is_ok());
    }

    struct FakeHerdr {
        launches: std::sync::Mutex<Vec<crate::herdr::Launch>>,
    }

    impl crate::herdr::Herdr for FakeHerdr {
        fn launch(&self, l: &crate::herdr::Launch) -> Result<crate::herdr::Outcome, String> {
            self.launches.lock().unwrap().push(l.clone());
            Ok(crate::herdr::Outcome::Started)
        }
    }

    fn with_herdr(app: &mut App) -> std::sync::Arc<FakeHerdr> {
        let fake = std::sync::Arc::new(FakeHerdr { launches: std::sync::Mutex::new(Vec::new()) });
        app.herdr = Some(fake.clone());
        fake
    }

    fn launches_after(app: &mut App, fake: &FakeHerdr, want: usize) -> Vec<crate::herdr::Launch> {
        for _ in 0..200 {
            app.poll_ready();
            if fake.launches.lock().unwrap().len() >= want {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        fake.launches.lock().unwrap().clone()
    }

    fn drag_card(app: &mut App, card: &str, column: &str) {
        let out = render(app, 200, 30);
        let (x, y) = find(&out, card);
        let (cx, cy) = find(&out, column);
        app.on_click(x, y);
        app.on_drag(cx + 4, cy + 1);
        app.on_drag(cx + 5, cy + 6);
        app.on_release();
    }

    #[test]
    fn dragging_a_todo_card_onto_planning_starts_the_plan_skill_in_the_storys_workspace() {
        let (_root, mut app) = sample();
        let fake = with_herdr(&mut app);
        drag_card(&mut app, "T3 Spike it", "PLANNING");
        let launches = launches_after(&mut app, &fake, 1);
        assert_eq!(
            launches,
            vec![crate::herdr::Launch {
                workspace: "PROJ-1".into(),
                tab: "T3 plan".into(),
                agent: "proj-1-t3-plan".into(),
                prompt: "/story-plan-task PROJ-1 T3".into(),
            }]
        );
        assert_eq!(app.selected_task().unwrap().status, Status::Todo, "the skill moves the card, not the drag");
        assert!(!app.detail);
    }

    #[test]
    fn dragging_a_ready_planned_card_onto_implementing_starts_the_implement_skill() {
        let (_root, mut app) = sample();
        let fake = with_herdr(&mut app);
        drag_card(&mut app, "T1 Add endpoint", "IMPLEMENTING");
        let launches = launches_after(&mut app, &fake, 1);
        assert_eq!(launches.len(), 1);
        assert_eq!(launches[0].prompt, "/story-implement-task PROJ-1 T1");
        assert_eq!((launches[0].tab.as_str(), launches[0].agent.as_str()), ("T1 implement", "proj-1-t1-impl"));
    }

    #[test]
    fn a_card_whose_dependencies_are_not_done_is_not_started() {
        let (_root, mut app) = sample();
        let fake = with_herdr(&mut app);
        drag_card(&mut app, "T2 Wire UI", "IMPLEMENTING");
        assert!(launches_after(&mut app, &fake, 1).is_empty());
        assert!(app.current_notice().unwrap().contains("waits for T1"), "{:?}", app.current_notice());
    }

    #[test]
    fn other_drops_start_nothing_and_say_why() {
        let (_root, mut app) = sample();
        let fake = with_herdr(&mut app);
        drag_card(&mut app, "T3 Spike it", "DONE");
        assert!(launches_after(&mut app, &fake, 1).is_empty());
        assert!(app.current_notice().unwrap().contains("drop a TODO card on PLANNING"));
    }

    #[test]
    fn dropping_a_card_back_on_its_own_column_starts_nothing_and_a_selected_card_still_opens() {
        let (_root, mut app) = sample();
        let fake = with_herdr(&mut app);
        let out = render(&app, 200, 30);
        let (x, y) = find(&out, "T3 Spike it");
        assert_eq!(app.selected_task().unwrap().id, "T3", "already selected");
        app.on_click(x, y);
        app.on_drag(x + 1, y);
        app.on_release();
        assert!(app.detail, "pressing the selected card and not leaving its column opens the detail");
        assert!(launches_after(&mut app, &fake, 1).is_empty());
    }

    #[test]
    fn p_and_i_start_the_selected_task_and_outside_herdr_nothing_starts() {
        let (_root, mut app) = sample();
        let fake = with_herdr(&mut app);
        app.col = 0;
        app.row[0] = 0;
        app.on_key(KeyCode::Char('p'), false);
        app.col = 2;
        app.row[2] = 0;
        app.on_key(KeyCode::Char('i'), false);
        let prompts: Vec<String> = launches_after(&mut app, &fake, 2).into_iter().map(|l| l.prompt).collect();
        assert!(prompts.contains(&"/story-plan-task PROJ-1 T3".to_string()), "{prompts:?}");
        assert!(prompts.contains(&"/story-implement-task PROJ-1 T1".to_string()), "{prompts:?}");
        let (_root, mut plain) = sample();
        plain.col = 0;
        plain.on_key(KeyCode::Char('p'), false);
        assert!(plain.current_notice().unwrap().contains("not running inside Herdr"));
    }

    #[test]
    fn the_drop_column_lights_up_while_a_card_is_held_over_it() {
        let (_root, mut app) = sample();
        let out = render(&app, 200, 30);
        let (x, y) = find(&out, "T3 Spike it");
        let (cx, cy) = find(&out, "PLANNING");
        app.on_click(x, y);
        app.on_drag(cx + 4, cy + 3);
        assert_eq!(app.drop_column(), Some(1));
        app.on_drag(x, y);
        assert_eq!(app.drop_column(), None, "over its own column nothing is a target");
    }

    #[test]
    fn pasting_a_jira_link_into_the_new_story_box_starts_the_breakdown() {
        let (_root, mut app) = two_stories();
        let fake = with_herdr(&mut app);
        let out = render(&app, 120, 30);
        assert!(out.contains("[ n + new story ]"), "{out}");
        let (x, y) = find(&out, "[ n + new story ]");
        app.on_click(x + 3, y);
        assert!(app.new_story.is_some());
        app.on_paste("https://acme.atlassian.net/browse/NEW-5?focusedId=1\n");
        let out = render(&app, 120, 30);
        assert!(out.contains("New story") && out.contains("NEW-5?focusedId=1"), "{out}");
        let (sx, sy) = find(&out, "[ Enter Start ]");
        app.on_click(sx + 2, sy);
        assert!(app.new_story.is_none());
        let launches = launches_after(&mut app, &fake, 1);
        assert_eq!(
            launches,
            vec![crate::herdr::Launch {
                workspace: "NEW-5".into(),
                tab: "break down".into(),
                agent: "new-5-breakdown".into(),
                prompt: "/story-break-down NEW-5".into(),
            }]
        );
    }

    #[test]
    fn the_new_story_box_takes_keys_and_rejects_text_without_a_key_or_a_known_story() {
        let (_root, mut app) = two_stories();
        let fake = with_herdr(&mut app);
        app.on_key(KeyCode::Char('n'), false);
        for c in "hello".chars() {
            app.on_key(KeyCode::Char(c), false);
        }
        app.on_key(KeyCode::Backspace, false);
        assert_eq!(app.new_story.as_deref(), Some("hell"), "q and other keys are text while the box is open");
        app.on_key(KeyCode::Enter, false);
        assert!(app.new_story.is_some(), "the box stays open so the text can be fixed");
        assert!(app.current_notice().unwrap().contains("no Jira key"));
        app.on_key(KeyCode::Esc, false);
        assert!(app.new_story.is_none());
        app.on_key(KeyCode::Char('n'), false);
        app.on_paste("PROJ-2");
        app.on_key(KeyCode::Enter, false);
        assert!(app.current_notice().unwrap().contains("already on the board"));
        assert!(launches_after(&mut app, &fake, 1).is_empty());
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
    fn job(name: &str, state: RunState, run: u64, id: u64) -> Job {
        Job {
            name: name.into(),
            state,
            url: format!("https://github.com/acme/widgets/actions/runs/{run}/job/{id}"),
            started_at: None,
            completed_at: None,
        }
    }

    fn run(repo: &str, id: u64, title: &str, state: RunState, jobs: Vec<Job>) -> Run {
        Run {
            repo: repo.into(),
            id,
            name: "Deploy nonprod".into(),
            title: title.into(),
            branch: "feat/x".into(),
            url: format!("https://github.com/{repo}/actions/runs/{id}"),
            state,
            created_at: "2026-10-08T03:00:00Z".into(),
            started_at: None,
            updated_at: "2026-10-08T03:01:00Z".into(),
            jobs,
        }
    }

    fn runs_feed() -> Batch {
        Batch {
            runs: vec![
                run(
                    "acme/widgets",
                    101,
                    "Deploy to nonprod",
                    RunState::Running,
                    vec![
                        job("compile", RunState::Success, 101, 1),
                        job("lint", RunState::Failed, 101, 5),
                        job("smoke", RunState::Queued, 101, 3),
                        job("deploy", RunState::Running, 101, 2),
                        job("approve", RunState::Waiting, 101, 4),
                    ],
                ),
                run("acme/widgets", 100, "Earlier deploy", RunState::Success, vec![]),
                run("acme/api", 55, "Release", RunState::Failed, vec![job("publish", RunState::Failed, 55, 9)]),
            ],
            warnings: vec![],
        }
    }

    fn with_runs() -> (TempDir, App) {
        let (root, mut app) = with_prs();
        app.apply_runs(Ok(runs_feed()));
        (root, app)
    }

    fn recorder(app: &mut App) -> std::rc::Rc<std::cell::RefCell<Vec<String>>> {
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let sink = log.clone();
        app.opener = Box::new(move |url| sink.borrow_mut().push(url.to_string()));
        log
    }

    #[test]
    fn a_wide_terminal_shows_pull_requests_and_manual_runs_side_by_side() {
        let (_root, app) = with_runs();
        let out = render(&app, 170, 44);
        for want in [
            "PULL REQUESTS · 3", "MANUAL RUNS · 3", "[RUNNING]", "[SUCCESS]", "[FAILED]", "acme/widgets",
            "Deploy nonprod", "feat/x", "[ details ]", "Deploy to nonprod", "Release",
        ] {
            assert!(out.contains(want), "missing {want:?} in:\n{out}");
        }
        assert!(line_of(&out, "PULL REQUESTS").contains("MANUAL RUNS"), "same row, side by side:\n{out}");
        assert_eq!(out.matches("[ details ]").count(), 6, "a details button on each PR and each run:\n{out}");
        assert!(!out.contains("[ open ]"), "runs no longer have a separate open button:\n{out}");
        assert!(!out.contains("compile"), "passed stages are not listed:\n{out}");
    }

    #[test]
    fn run_stages_are_listed_vertically_running_then_waiting_then_queued_then_failed() {
        let (_root, app) = with_runs();
        let out = render(&app, 170, 44);
        let (xd, yd) = find(&out, "◔ deploy");
        let (xa, ya) = find(&out, "◑ approve");
        let (xs, ys) = find(&out, "● smoke");
        let (xl, yl) = find(&out, "✗ lint");
        assert!(xd == xa && xa == xs && xs == xl, "one column:\n{out}");
        assert_eq!((ya, ys, yl), (yd + 1, yd + 2, yd + 3), "{out}");
    }

    #[test]
    fn runs_are_separated_by_dividers() {
        let (_root, app) = with_runs();
        let out = render(&app, 170, 44);
        let col = find(&out, "MANUAL RUNS").0 as usize;
        let first = find(&out, "Deploy to nonprod").1 as usize;
        let second = find(&out, "Earlier deploy").1 as usize;
        let between: Vec<String> = out.lines().skip(first + 1).take(second - first - 1).map(|l| l.chars().skip(col).collect()).collect();
        assert!(between.iter().any(|l| l.contains("────────")), "{out}");
    }

    #[test]
    fn a_narrow_terminal_uses_tabs_with_live_counts_and_a_click_switches_tab() {
        let (_root, mut app) = with_runs();
        let out = render(&app, 120, 44);
        assert!(out.contains("[ Pull requests · 3 ]"), "{out}");
        assert!(out.contains("[ Manual runs · 3 (1 active) ]"), "{out}");
        assert!(out.contains("acme/api#99") && !out.contains("[RUNNING]"), "{out}");
        let (x, y) = find(&out, "[ Manual runs");
        app.on_click(x + 2, y);
        assert_eq!((app.tab, app.focus), (BottomTab::Runs, Focus::Runs));
        let out = render(&app, 120, 44);
        assert!(out.contains("[RUNNING]") && out.contains("MANUAL RUNS · 3"), "{out}");
        assert!(!out.contains("acme/api#99"), "{out}");
    }

    fn rightmost(out: &str, y: u16, needle: &str) -> u16 {
        let line = out.lines().nth(y as usize).unwrap();
        line[..line.rfind(needle).unwrap_or_else(|| panic!("{needle:?} not on line {y}:\n{out}"))].chars().count() as u16
    }

    #[test]
    fn clicking_a_run_opens_it_in_the_browser_and_a_stage_opens_that_stage() {
        let (_root, mut app) = with_runs();
        let log = recorder(&mut app);
        let out = render(&app, 170, 44);
        let (x, y) = find(&out, "Deploy to nonprod");
        app.on_click(x, y);
        assert_eq!(*log.borrow(), vec!["https://github.com/acme/widgets/actions/runs/101".to_string()]);
        assert_eq!((app.focus, app.run_sel, app.run_sheet), (Focus::Runs, 0, false));
        let (x, y) = find(&out, "◔ deploy");
        app.on_click(x + 2, y);
        assert_eq!(log.borrow()[1], "https://github.com/acme/widgets/actions/runs/101/job/2");
        assert!(!app.run_sheet, "click-through does not open the sheet");
    }

    #[test]
    fn the_details_button_opens_the_run_sheet_without_opening_the_browser() {
        let (_root, mut app) = with_runs();
        let log = recorder(&mut app);
        let out = render(&app, 170, 44);
        let y = find(&out, "Release").1;
        let x = rightmost(&out, y, "[ details ]");
        app.on_click(x + 2, y);
        assert!(app.run_sheet);
        assert_eq!((app.focus, app.run_sel), (Focus::Runs, 2));
        assert!(log.borrow().is_empty(), "details does not open the browser");
        let out = render(&app, 170, 44);
        for want in ["acme/api", "Release", "Stages (1)", "✗ publish", "failed"] {
            assert!(out.contains(want), "missing {want:?} in:\n{out}");
        }
        let (bx, by) = find(&out, "[ o Open run ]");
        app.on_click(bx + 3, by);
        assert_eq!(*log.borrow(), vec!["https://github.com/acme/api/actions/runs/55".to_string()]);
        assert!(app.run_sheet);
        app.on_click(0, 0);
        assert!(!app.run_sheet);
    }

    #[test]
    fn every_stage_in_the_run_sheet_has_its_own_open_button() {
        let (_root, mut app) = with_runs();
        let log = recorder(&mut app);
        app.focus = Focus::Runs;
        app.run_sel = 0;
        app.run_sheet = true;
        let out = render(&app, 170, 44);
        assert_eq!(out.matches("[ open ]").count(), 5, "five stages, five buttons:\n{out}");
        let (y, line) = out.lines().enumerate().find(|(_, l)| l.contains("approve") && l.contains("[ open ]")).expect("approve line");
        let x = line[..line.rfind("[ open ]").unwrap()].chars().count() as u16;
        app.on_click(x + 2, y as u16);
        assert_eq!(*log.borrow(), vec!["https://github.com/acme/widgets/actions/runs/101/job/4".to_string()]);
        assert!(app.run_sheet, "opening a stage keeps the sheet");
    }

    #[test]
    fn every_check_with_a_link_in_the_pr_sheet_has_its_own_open_button() {
        let (_root, mut app) = with_prs();
        let log = recorder(&mut app);
        app.focus = Focus::Prs;
        app.pr_sel = 0;
        app.pr_sheet = true;
        let out = render(&app, 170, 50);
        assert_eq!(out.matches("[ open ]").count(), 6, "7 checks, one without a link:\n{out}");
        let (y, line) = out.lines().enumerate().find(|(_, l)| l.contains("CI / unit") && l.contains("[ open ]")).expect("unit line");
        let x = line[..line.rfind("[ open ]").unwrap()].chars().count() as u16;
        app.on_click(x + 2, y as u16);
        assert_eq!(*log.borrow(), vec!["https://github.com/acme/widgets/actions/runs/1/job/unit".to_string()]);
        assert!(app.pr_sheet);
        assert!(!out.lines().any(|l| l.contains("CI / docs") && l.contains("[ open ]")), "no link, no button");
    }

    #[test]
    fn clicking_a_ci_stage_in_the_pr_panel_opens_that_check() {
        let (_root, mut app) = with_prs();
        let log = recorder(&mut app);
        let out = render(&app, 170, 44);
        let (x, y) = find(&out, "◔ CI / build");
        app.on_click(x + 2, y);
        assert_eq!(*log.borrow(), vec!["https://github.com/acme/widgets/actions/runs/1/job/build".to_string()]);
        assert_eq!((app.focus, app.pr_sel, app.pr_sheet), (Focus::Prs, 0, false));
    }

    #[test]
    fn the_story_board_shows_only_runs_for_the_storys_repos() {
        let (_root, mut app) = with_runs();
        app.open_story("PROJ-2");
        let out = render(&app, 170, 44);
        assert!(out.contains("STORY MANUAL RUNS · 2"), "{out}");
        assert!(!out.contains("Release"), "{out}");
    }

    #[test]
    fn the_runs_panel_explains_loading_empty_error_and_warning_states() {
        let (_root, mut app) = two_stories();
        app.focus = Focus::Runs;
        let out = render(&app, 170, 40);
        assert!(out.contains("Loading manual runs"), "{out}");
        app.apply_runs(Ok(Batch::default()));
        let out = render(&app, 170, 40);
        assert!(out.contains("No manual runs"), "{out}");
        app.apply_runs(Ok(Batch { runs: vec![], warnings: vec!["acme/api: HTTP 403".into()] }));
        let out = render(&app, 170, 40);
        assert!(out.contains("acme/api: HTTP 403"), "{out}");
        assert!(out.contains("No manual runs in the last 3 hours"), "with room, the full message shows:\n{out}");
        app.apply_runs(Err("gh failed: not logged in".into()));
        let out = render(&app, 170, 40);
        assert!(out.contains("gh failed: not logged in"), "{out}");
    }

    #[test]
    fn scrolling_over_the_runs_panel_moves_the_run_selection() {
        let (_root, mut app) = with_runs();
        let out = render(&app, 170, 44);
        let (x, y) = find(&out, "Earlier deploy");
        app.on_scroll(x, y, 1);
        assert_eq!((app.focus, app.run_sel), (Focus::Runs, 1));
        app.on_scroll(x, y, -1);
        assert_eq!(app.run_sel, 0);
        assert_eq!((app.list_sel, app.pr_sel), (0, 0));
    }
    fn titled(title: &str) -> PullRequest {
        let mut pr = feed().remove(0);
        pr.title = title.to_string();
        pr
    }

    #[test]
    fn the_pr_title_has_its_own_line_and_is_never_cut_off() {
        let (_root, mut app) = with_prs();
        let long = "Add a very long descriptive title that explains the whole change in great detail so that it has to wrap";
        app.apply_prs(Ok(vec![titled(long)]));
        for width in [170u16, 120, 90] {
            let out = render(&app, width, 44);
            for word in long.split_whitespace() {
                assert!(out.contains(word), "width {width}: {word:?} is missing:\n{out}");
            }
            assert!(!line_of(&out, "Add a very long").contains("[READY]"), "the title has a line of its own:\n{out}");
            let pr_cols = if width >= 150 { usize::from(width) * 58 / 100 } else { usize::from(width) };
            let cut = out.lines().filter(|l| l.chars().take(pr_cols).any(|c| c == '…')).count();
            assert_eq!(cut, 0, "width {width}: nothing in the PR panel is truncated:\n{out}");
        }
        let out = render(&app, 170, 44);
        let title_y = find(&out, "Add a very long").1;
        let meta_y = find(&out, "acme/widgets#12").1;
        assert!(meta_y > title_y, "the title comes first, then the badge and repo line:\n{out}");
    }

    #[test]
    fn a_title_that_fits_takes_one_line_and_clicking_it_still_opens_the_pr() {
        let (_root, mut app) = with_prs();
        let log = recorder(&mut app);
        let out = render(&app, 170, 44);
        let (x, y) = find(&out, "Add notices");
        assert_eq!(find(&out, "acme/widgets#12").1, y + 1, "the repo line directly follows a one-line title");
        app.on_click(x, y);
        assert_eq!(*log.borrow(), vec!["https://github.com/acme/widgets/pull/12".to_string()]);
    }

    fn runs_x(out: &str) -> u16 {
        find(out, "MANUAL RUNS").0
    }

    #[test]
    fn an_empty_manual_runs_panel_is_narrow_and_leaves_the_room_to_the_prs() {
        let (_root, mut app) = with_prs();
        assert!(runs_x(&render(&app, 170, 44)) < 110, "still loading: normal width, no jumping around");
        app.apply_runs(Ok(Batch::default()));
        let out = render(&app, 170, 44);
        assert!(runs_x(&out) >= 170 - 36, "empty runs panel is narrow:\n{out}");
        assert!(out.contains("No manual runs"), "{out}");
        app.apply_runs(Ok(runs_feed()));
        assert!(runs_x(&render(&app, 170, 44)) < 110, "runs back: wide again");
        app.apply_runs(Ok(Batch::default()));
        app.apply_runs(Err("gh failed: not logged in".into()));
        assert!(runs_x(&render(&app, 170, 44)) < 110, "an error needs room to be read");
        app.apply_runs(Ok(Batch { runs: vec![], warnings: vec!["acme/api: HTTP 403".into()] }));
        assert!(runs_x(&render(&app, 170, 44)) < 110, "a warning needs room to be read");
    }

    #[test]
    fn keys_resize_the_panels_within_limits_and_equals_resets() {
        let (_root, mut app) = with_runs();
        assert_eq!((app.split_pct, app.bottom_pct), (58, None));
        app.on_key(KeyCode::Char('>'), false);
        assert_eq!(app.split_pct, 63);
        app.on_key(KeyCode::Char('<'), false);
        app.on_key(KeyCode::Char('<'), false);
        assert_eq!(app.split_pct, 53);
        for _ in 0..20 {
            app.on_key(KeyCode::Char('<'), false);
        }
        assert_eq!(app.split_pct, 25);
        for _ in 0..30 {
            app.on_key(KeyCode::Char('>'), false);
        }
        assert_eq!(app.split_pct, 80);
        app.on_key(KeyCode::Char('+'), false);
        assert_eq!(app.bottom_pct, Some(50), "from the list default of 45");
        for _ in 0..20 {
            app.on_key(KeyCode::Char('+'), false);
        }
        assert_eq!(app.bottom_pct, Some(80));
        for _ in 0..30 {
            app.on_key(KeyCode::Char('-'), false);
        }
        assert_eq!(app.bottom_pct, Some(15));
        app.on_key(KeyCode::Char('='), false);
        assert_eq!((app.split_pct, app.bottom_pct), (58, None));
    }

    #[test]
    fn dragging_the_divider_resizes_the_two_panels() {
        let (_root, mut app) = with_runs();
        let out = render(&app, 170, 44);
        let (tx, ty) = find(&out, "MANUAL RUNS");
        app.on_click(tx - 2, ty + 3);
        assert!(!app.run_sheet && app.focus == Focus::Main, "grabbing the divider is not a click on a run");
        app.on_drag(120, ty + 3);
        app.on_release();
        assert_eq!(app.split_pct, 70);
        let out = render(&app, 170, 44);
        let x = runs_x(&out);
        assert!((117..=123).contains(&x), "the runs panel now starts near column 120, not {x}:\n{out}");
        let (tx2, _) = find(&out, "MANUAL RUNS");
        app.on_click(tx2 - 2, ty + 3);
        app.on_drag(2, ty + 3);
        assert_eq!(app.split_pct, 25, "dragged to the far left it stops at the limit");
        app.on_release();
        app.on_drag(150, ty + 3);
        assert_eq!(app.split_pct, 25, "after releasing, moving the pointer changes nothing");
    }

    #[test]
    fn dragging_the_top_edge_resizes_the_bottom_area() {
        let (_root, mut app) = with_runs();
        let out = render(&app, 170, 44);
        let (_, y) = find(&out, "PULL REQUESTS");
        app.on_click(60, y);
        app.on_drag(60, y - 6);
        app.on_release();
        assert!(app.bottom_pct.is_some_and(|p| p > 45));
        let out = render(&app, 170, 44);
        let new_y = find(&out, "PULL REQUESTS").1;
        assert!((4..=7).contains(&(y - new_y)), "the panels moved up by about 6 rows ({y} -> {new_y}):\n{out}");
        app.on_click(60, new_y);
        app.on_drag(60, 40);
        app.on_release();
        assert_eq!(app.bottom_pct, Some(15), "dragged almost to the bottom it stops at the smallest size");
    }

    #[test]
    fn the_height_setting_also_applies_inside_a_story_and_keeps_the_board_usable() {
        let (_root, mut app) = with_runs();
        app.open_story("PROJ-2");
        let auto_y = find(&render(&app, 170, 44), "STORY PULL REQUESTS").1;
        app.bottom_pct = Some(70);
        let out = render(&app, 170, 44);
        let y = find(&out, "STORY PULL REQUESTS").1;
        assert!(y < auto_y, "a taller bottom area starts higher ({auto_y} -> {y})");
        assert!(out.contains("TODO") && out.contains("PLANNED"), "the board is still drawn:\n{out}");
    }

    #[test]
    fn narrow_terminals_can_still_resize_the_height_but_have_no_divider() {
        let (_root, mut app) = with_runs();
        let out = render(&app, 120, 44);
        let y = find(&out, "[ Pull requests").1;
        app.on_click(100, y);
        app.on_drag(100, y.saturating_sub(4));
        app.on_release();
        assert!(app.bottom_pct.is_some_and(|p| p > 45));
        let hits = app.hits.borrow();
        assert!(!hits.iter().any(|(_, t)| *t == crate::app::Target::SplitHandle), "tabs have no divider");
    }
    #[test]
    fn dragging_a_column_border_resizes_the_two_neighbouring_columns() {
        let (_root, mut app) = sample();
        let out = render(&app, 200, 30);
        let (planning_x, _) = find(&out, "PLANNING");
        let (planned_x, _) = find(&out, "PLANNED");
        let widths = |out: &str| (find(out, "PLANNING").0, find(out, "PLANNED").0);
        assert_eq!(widths(&out), (planning_x, planned_x));
        app.on_click(planning_x - 2, 6);
        app.on_drag(planning_x - 22, 6);
        app.on_release();
        let out = render(&app, 200, 30);
        let (new_planning_x, new_planned_x) = widths(&out);
        assert!(new_planning_x + 18 <= planning_x, "the border moved left by about 20 ({planning_x} -> {new_planning_x}):\n{out}");
        let before = planned_x - planning_x;
        let after = new_planned_x - new_planning_x;
        assert!(after > before, "the column to the right of the border grew: {before} -> {after}");
        assert!(app.col_weights[0] < 100 && app.col_weights[1] > 100, "{:?}", app.col_weights);
    }

    #[test]
    fn the_shrunken_runs_strip_can_be_dragged_wider_and_then_stays_that_wide() {
        let (_root, mut app) = with_prs();
        app.apply_runs(Ok(Batch::default()));
        let out = render(&app, 170, 44);
        let (tx, ty) = find(&out, "MANUAL RUNS");
        assert!(tx >= 134, "starts as a narrow strip");
        app.on_click(tx - 2, ty + 3);
        app.on_drag(100, ty + 3);
        app.on_release();
        assert!(app.split_pinned);
        let out = render(&app, 170, 44);
        let x = runs_x(&out);
        assert!((97..=103).contains(&x), "now about 70 columns wide, starting near 100, not {x}:\n{out}");
        app.apply_runs(Ok(Batch::default()));
        assert!(runs_x(&render(&app, 170, 44)) < 110, "a size you chose is respected even when empty");
        app.on_key(KeyCode::Char('='), false);
        assert!(runs_x(&render(&app, 170, 44)) >= 134, "reset: automatic again");
    }
}
