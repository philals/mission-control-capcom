//! The pull request panel (bottom of the story list and of a board) and the PR detail sheet.
use crate::app::{App, Focus, PrRow, Screen, Target};
use crate::prs::{check_seconds, duration_text, relative, Check, CheckState, PullRequest, Review};
use chrono::{DateTime, Utc};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use ratatui::Frame;
use storyboard::model::PrState;

type Hits = Vec<(Rect, Target)>;

const OPEN_BUTTON: &str = "[ o Open in browser ]";
const MAX_STAGE_LINES: usize = 8;
pub const ORANGE: Color = Color::Indexed(208);

/// Lines the PR rows need (rows plus the dividers between them).
pub fn content_height(app: &App) -> u16 {
    let rows = app.pr_rows();
    rows.iter().map(|r| row_height(r) + 1).sum::<u16>().saturating_sub(1)
}

/// First row to draw so that the selected row (heights include the divider) fits in `avail` lines.
pub fn window_offset(heights: &[u16], selected: usize, avail: u16) -> usize {
    let mut offset = 0;
    while offset < selected && heights[offset..=selected].iter().sum::<u16>() > avail + 1 {
        offset += 1;
    }
    offset
}

pub fn trunc(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Cut a run of spans to fit a width, ending with an ellipsis when something was dropped.
pub fn fit(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut used = 0;
    for span in spans {
        let len = span.content.chars().count();
        if used + len <= width {
            used += len;
            out.push(span);
        } else {
            let room = width.saturating_sub(used);
            if room > 0 {
                out.push(Span::styled(trunc(&span.content, room), span.style));
            }
            break;
        }
    }
    out
}

pub fn width_of(spans: &[Span<'static>]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

pub fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

fn review_span(review: Review) -> Option<Span<'static>> {
    match review {
        Review::Approved => Some(Span::styled("✓ approved", Style::new().fg(Color::Green))),
        Review::ChangesRequested => Some(Span::styled("✖ changes requested", Style::new().fg(Color::Red))),
        Review::Required => Some(Span::styled("● review required", Style::new().fg(Color::Yellow))),
        Review::None => None,
    }
}

/// `[READY]` (green) or `[DRAFT]` (grey) for a live PR; the board's own state for a recorded one.
fn badge(row: &PrRow) -> Span<'static> {
    let (text, color) = match (row.live, row.board_state) {
        (Some(pr), _) if pr.is_draft => ("[DRAFT] ", Color::DarkGray),
        (Some(_), _) => ("[READY] ", Color::Green),
        (None, Some(PrState::Draft)) => ("[DRAFT] ", Color::DarkGray),
        (None, Some(PrState::Ready)) => ("[READY] ", Color::Green),
        (None, Some(PrState::Merged)) => ("[MERGED]", Color::Magenta),
        (None, Some(PrState::Closed)) => ("[CLOSED]", Color::Red),
        (None, None) => ("[?]     ", Color::DarkGray),
    };
    Span::styled(text, Style::new().fg(color).add_modifier(Modifier::BOLD))
}

fn summary_line(pr: &PullRequest) -> Vec<Span<'static>> {
    if pr.checks.is_empty() {
        return vec![Span::styled("no checks", dim())];
    }
    let c = pr.counts();
    let mut spans = Vec::new();
    let mut push = |text: String, color: Color| {
        spans.push(Span::styled(text, Style::new().fg(color)));
        spans.push(Span::raw("  "));
    };
    if c.passed > 0 {
        push(format!("✓ {}", c.passed), Color::Green);
    }
    if c.failed > 0 {
        push(format!("✗ {}", c.failed), Color::Red);
    }
    if c.running > 0 {
        push(format!("◔ {}", c.running), Color::Yellow);
    }
    if c.queued > 0 {
        push(format!("● {}", c.queued), ORANGE);
    }
    if c.skipped > 0 {
        push(format!("⊘ {}", c.skipped), Color::DarkGray);
    }
    spans
}

fn stage_style(state: CheckState) -> (&'static str, Color, &'static str) {
    match state {
        CheckState::Running => ("◔", Color::Yellow, "running"),
        CheckState::Queued => ("●", ORANGE, "queued"),
        CheckState::Failed => ("✗", Color::Red, "failed"),
        CheckState::Passed => ("✓", Color::Green, "passed"),
        CheckState::Skipped => ("⊘", Color::DarkGray, "skipped"),
    }
}

/// The stages worth watching, one per line: running, then queued, then failed.
fn open_stages(pr: &PullRequest) -> Vec<&Check> {
    [CheckState::Running, CheckState::Queued, CheckState::Failed]
        .into_iter()
        .flat_map(|state| pr.checks_in(state))
        .collect()
}

fn stage_count(row: &PrRow) -> usize {
    row.live.map_or(0, |pr| open_stages(pr).len())
}

/// Lines a row takes (not counting the divider below it): header, CI summary, then the stages.
fn row_height(row: &PrRow) -> u16 {
    let stages = stage_count(row);
    let shown = stages.min(MAX_STAGE_LINES) + usize::from(stages > MAX_STAGE_LINES);
    (2 + shown) as u16
}

fn stage_line(
    check: &Check,
    label_width: usize,
    marker: &Span<'static>,
    now: DateTime<Utc>,
) -> Vec<Span<'static>> {
    let (icon, color, word) = stage_style(check.state);
    let took = check_seconds(check, now).map_or(String::new(), |s| format!(" {}", duration_text(s)));
    vec![
        marker.clone(),
        Span::raw("  "),
        Span::styled(format!("{icon} "), Style::new().fg(color)),
        Span::raw(format!("{:<label_width$}", check.label())),
        Span::styled(format!("   {word}{took}"), dim()),
    ]
}

fn row_lines(row: &PrRow, width: usize, selected: bool, now: DateTime<Utc>) -> Vec<Line<'static>> {
    let marker = Span::styled(if selected { "▌ " } else { "  " }, Style::new().fg(Color::Cyan));
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let id = match row.number {
        Some(n) => format!("{}#{n}", row.repo),
        None => row.repo.clone(),
    };
    let mut right: Vec<Span<'static>> = Vec::new();
    let mut summary: Vec<Span<'static>> = vec![marker.clone(), Span::raw("  ")];
    let mut stages: Vec<Vec<Span<'static>>> = Vec::new();
    let mut title = row.title.clone();
    match row.live {
        Some(pr) => {
            if !pr.labels.is_empty() {
                let labels = pr.labels.iter().map(|l| format!("[{l}]")).collect::<Vec<_>>().join(" ");
                right.push(Span::styled(format!("{labels}  "), dim()));
            }
            if let Some(review) = review_span(pr.review) {
                right.push(review);
                right.push(Span::raw("  "));
            }
            if pr.comments > 0 {
                right.push(Span::styled(format!("✎ {}  ", pr.comments), dim()));
            }
            right.push(Span::styled(relative(&pr.updated_at, now), dim()));
            summary.extend(summary_line(pr));
            let open = open_stages(pr);
            let label_width = open
                .iter()
                .take(MAX_STAGE_LINES)
                .map(|c| c.label().chars().count())
                .max()
                .unwrap_or(0);
            for check in open.iter().take(MAX_STAGE_LINES) {
                stages.push(stage_line(check, label_width, &marker, now));
            }
            if open.len() > MAX_STAGE_LINES {
                let more = open.len() - MAX_STAGE_LINES;
                stages.push(vec![
                    marker.clone(),
                    Span::raw("  "),
                    Span::styled(format!("… +{more} more (Enter shows every check)"), dim()),
                ]);
            }
        }
        None => {
            if title.is_empty() {
                title = "(not in your open pull requests)".to_string();
            }
            summary.push(Span::styled("no live check data", dim()));
        }
    }
    let mut left: Vec<Span<'static>> = vec![marker, badge(row)];
    if let Some(tag) = &row.tag {
        left.push(Span::styled(format!(" {tag}  "), bold.fg(Color::Magenta)));
    } else {
        left.push(Span::raw(" "));
    }
    left.push(Span::styled(format!("{id}  "), Style::new().fg(Color::Cyan)));
    left.push(Span::styled(title, bold));
    let right_width = width_of(&right) + 1;
    let mut first = fit(left, width.saturating_sub(right_width));
    let pad = width.saturating_sub(width_of(&first) + width_of(&right));
    first.push(Span::raw(" ".repeat(pad)));
    first.extend(right);
    let style = if selected { Style::new().bg(Color::Indexed(237)) } else { Style::new() };
    let mut lines = vec![Line::from(first).style(style), Line::from(fit(summary, width)).style(style)];
    lines.extend(stages.into_iter().map(|s| Line::from(fit(s, width)).style(style)));
    lines
}

pub fn draw_panel(f: &mut Frame, area: Rect, app: &App, hits: &mut Hits) {
    let rows = app.pr_rows();
    let focused = app.focus == Focus::Prs;
    let name = if app.screen == Screen::List { "PULL REQUESTS" } else { "STORY PULL REQUESTS" };
    let status = if app.prs.loading {
        " refreshing… ".to_string()
    } else {
        app.prs.updated.as_ref().map_or(String::new(), |u| format!(" updated {u} "))
    };
    let block = Block::bordered()
        .border_style(Style::new().fg(if focused { Color::Blue } else { Color::DarkGray }))
        .title(Span::styled(
            format!(" {name} · {} ", rows.len()),
            Style::new().fg(if focused { Color::Blue } else { Color::White }).add_modifier(Modifier::BOLD),
        ))
        .title(Line::from(Span::styled(status, dim())).right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);
    hits.push((area, Target::PrPanel));
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let mut top = inner.y;
    if let Some(err) = &app.prs.error {
        let line = Line::from(Span::styled(trunc(&format!("! {err}"), inner.width as usize), Style::new().fg(Color::Red)));
        f.render_widget(Paragraph::new(line), Rect::new(inner.x, top, inner.width, 1));
        top += 1;
    }
    let body = Rect::new(inner.x, top, inner.width, (inner.y + inner.height).saturating_sub(top));
    if rows.is_empty() {
        let message = if app.screen == Screen::Board {
            Some("No pull requests recorded on this story's tasks yet.")
        } else if app.prs.disabled {
            Some("Pull requests are off (started with --no-prs).")
        } else if app.prs.error.is_some() {
            None
        } else if app.prs.loaded {
            Some("No open pull requests match the query.")
        } else {
            Some("Loading pull requests…")
        };
        if let (Some(message), true) = (message, body.height > 0) {
            f.render_widget(Paragraph::new(Span::styled(message, dim())), body);
        }
        return;
    }
    let heights: Vec<u16> = rows.iter().map(|r| row_height(r) + 1).collect();
    let selected = app.pr_sel.min(rows.len() - 1);
    let offset = window_offset(&heights, selected, body.height);
    let now = Utc::now();
    let mut y = body.y;
    let bottom = body.y + body.height;
    for (i, row) in rows.iter().enumerate().skip(offset) {
        if y >= bottom {
            break;
        }
        let h = (heights[i] - 1).min(bottom - y);
        let rect = Rect::new(body.x, y, body.width, h);
        let lines = row_lines(row, body.width as usize, focused && i == selected, now);
        f.render_widget(Paragraph::new(lines), rect);
        hits.push((rect, Target::Pr(i)));
        y += heights[i] - 1;
        if i + 1 < rows.len() && y < bottom {
            let rule = "─".repeat(body.width as usize);
            f.render_widget(Paragraph::new(Span::styled(rule, dim())), Rect::new(body.x, y, body.width, 1));
            y += 1;
        }
    }
}

fn label_row(label: &str, value: String) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{label:<12}"), dim()), Span::raw(value)])
}

fn check_line(check: &Check, label_width: usize, now: DateTime<Utc>) -> Line<'static> {
    let (icon, color, word) = stage_style(check.state);
    let took = check_seconds(check, now).map_or(String::new(), |s| format!(" {}", duration_text(s)));
    Line::from(vec![
        Span::styled(format!("  {icon} "), Style::new().fg(color)),
        Span::raw(format!("{:<label_width$}", check.label())),
        Span::styled(format!("   {word}{took}"), dim()),
    ])
}

pub fn draw_sheet(f: &mut Frame, area: Rect, app: &App, hits: &mut Hits) {
    let rows = app.pr_rows();
    let Some(row) = rows.get(app.pr_sel) else {
        return;
    };
    let w = (area.width * 80 / 100).max(50).min(area.width);
    let h = (area.height * 80 / 100).max(12).min(area.height);
    let rect = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
    let now = Utc::now();
    let id = row.number.map_or(row.repo.clone(), |n| format!("{}#{n}", row.repo));
    let mut lines = Vec::new();
    match row.live {
        Some(pr) => {
            let state = if pr.is_draft { "Draft (not ready for review)" } else { "Ready for review" };
            lines.push(label_row("State", state.into()));
            let review = match pr.review {
                Review::Approved => "approved",
                Review::ChangesRequested => "changes requested",
                Review::Required => "review required",
                Review::None => "none",
            };
            lines.push(label_row("Review", review.into()));
            let labels = if pr.labels.is_empty() { "none".to_string() } else { pr.labels.join(", ") };
            lines.push(label_row("Labels", labels));
            lines.push(label_row("Comments", pr.comments.to_string()));
            lines.push(label_row("Updated", format!("{} ({})", relative(&pr.updated_at, now), pr.updated_at)));
        }
        None => {
            let state = row.board_state.map_or("unknown", PrState::as_str);
            lines.push(label_row("On the board", state.into()));
            lines.push(Line::from(Span::styled("Not in your open pull requests, so no live check data.", dim())));
        }
    }
    if let Some(tag) = &row.tag {
        lines.push(label_row("Storyboard", tag.clone()));
    }
    lines.push(label_row("URL", row.url.clone()));
    if let Some(pr) = row.live {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(format!("Checks ({})", pr.checks.len()), Style::new().add_modifier(Modifier::BOLD))));
        if pr.checks.is_empty() {
            lines.push(Line::raw("  no checks"));
        }
        let label_width = pr.checks.iter().map(|c| c.label().chars().count()).max().unwrap_or(0);
        for state in [CheckState::Running, CheckState::Queued, CheckState::Failed, CheckState::Passed, CheckState::Skipped] {
            for check in pr.checks_in(state) {
                lines.push(check_line(check, label_width, now));
            }
        }
    }
    let title = if row.title.is_empty() { format!(" {id} ") } else { format!(" {id} · {} ", row.title) };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Blue))
        .title(Span::styled(title, Style::new().add_modifier(Modifier::BOLD)))
        .title_bottom(Line::from(vec![
            Span::raw(" "),
            Span::styled(OPEN_BUTTON, Style::new().fg(Color::Cyan)),
            Span::styled("  Esc or click outside to close ", dim()),
        ]));
    f.render_widget(Clear, rect);
    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), rect);
    hits.push((rect, Target::Sheet));
    let button = Rect::new(rect.x + 2, rect.y + rect.height - 1, OPEN_BUTTON.chars().count() as u16, 1);
    hits.push((button, Target::OpenPr));
}
