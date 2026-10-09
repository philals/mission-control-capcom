//! The pull request panel (bottom of the story list and of a board) and the PR detail sheet.
use crate::theme;
use crate::app::{App, Focus, PrRow, Screen, Target};
use crate::prs::{check_seconds, duration_text, relative, Check, CheckState, PullRequest, Review, Waiting};
use chrono::{DateTime, Utc};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Wrap};
use ratatui::Frame;
use capcom::model::PrState;

type Hits = Vec<(Rect, Target)>;

const OPEN_BUTTON: &str = "[ o Open in browser ]";
const DETAILS_BUTTON: &str = "[ details ]";
const COPY_BUTTON: &str = "[ copy ]";
const AGENT_BUTTON: &str = "[ agent ]";
const CONFIRM_YES: &str = "[ y Mark ready ]";
const CONFIRM_NO: &str = "[ n Cancel ]";
pub const SHEET_STAGE_OPEN: &str = "[ open ]";

fn details_width() -> usize {
    DETAILS_BUTTON.chars().count()
}

fn agent_width(row: &PrRow) -> usize {
    if row.owner.is_some() {
        AGENT_BUTTON.chars().count() + 1
    } else {
        0
    }
}

fn copy_width() -> usize {
    COPY_BUTTON.chars().count()
}

/// Width of the `[DRAFT]` badge that marks a draft ready when clicked.
const BADGE_CLICK_WIDTH: u16 = 7;
const MAX_STAGE_LINES: usize = 8;
const MAX_TITLE_LINES: usize = 3;
const NOT_LISTED: &str = "(not in your open pull requests)";
pub const ORANGE: Color = theme::ORANGE;

/// Lines the PR rows need (rows plus the dividers between them).
pub fn content_height(app: &App, width: usize) -> u16 {
    let rows = app.pr_rows();
    rows.iter().map(|r| row_height(r, width) + 1).sum::<u16>().saturating_sub(1)
}

/// Break text over lines on word boundaries (a word longer than a line is split), cutting after
/// `max_lines` with an ellipsis only when it still does not fit.
pub fn wrap_text(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        while word.chars().count() > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            lines.push(word.chars().take(width).collect());
            word = word.chars().skip(width).collect();
        }
        if word.is_empty() {
            continue;
        }
        let needed = if current.is_empty() {
            word.chars().count()
        } else {
            current.chars().count() + 1 + word.chars().count()
        };
        if needed <= width {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(&word);
        } else {
            lines.push(std::mem::take(&mut current));
            current = word;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        return vec![String::new()];
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            if last.chars().count() >= width {
                last.pop();
            }
            last.push('…');
        }
    }
    lines
}

fn title_of(row: &PrRow) -> String {
    if row.live.is_none() && row.title.is_empty() {
        NOT_LISTED.to_string()
    } else {
        row.title.clone()
    }
}

/// The title, wrapped onto its own full-width lines (after the two-column marker).
fn title_lines(row: &PrRow, width: usize) -> Vec<String> {
    wrap_text(&title_of(row), width.saturating_sub(2), MAX_TITLE_LINES)
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
    Style::new().fg(theme::DIM)
}

fn review_span(review: Review) -> Option<Span<'static>> {
    match review {
        Review::Approved => Some(Span::styled("✓ approved", Style::new().fg(theme::GREEN))),
        Review::ChangesRequested => Some(Span::styled("✖ changes requested", Style::new().fg(theme::RED))),
        Review::Required => Some(Span::styled("● review required", Style::new().fg(theme::YELLOW))),
        Review::None => None,
    }
}

/// `[READY]` (green) or `[DRAFT]` (grey) for a live PR; the board's own state for a recorded one.
fn badge(row: &PrRow) -> Span<'static> {
    let (text, color) = match (row.live, row.board_state) {
        (Some(pr), _) if pr.is_draft => ("[DRAFT] ", theme::DIM),
        (Some(_), _) => ("[READY] ", theme::GREEN),
        (None, Some(PrState::Draft)) => ("[DRAFT] ", theme::DIM),
        (None, Some(PrState::Ready)) => ("[READY] ", theme::GREEN),
        (None, Some(PrState::Merged)) => ("[MERGED]", theme::MAGENTA),
        (None, Some(PrState::Closed)) => ("[CLOSED]", theme::RED),
        (None, None) => ("[?]     ", theme::DIM),
    };
    Span::styled(text, Style::new().fg(color).add_modifier(Modifier::BOLD))
}

/// The highlight for an all-green PR that is only waiting on someone, with its words and an icon.
pub fn waiting_label(waiting: Waiting) -> (&'static str, Color) {
    match waiting {
        Waiting::Reviewer => ("◉ AWAITING REVIEW", theme::CYAN),
        Waiting::Merge => ("✔ APPROVED · READY TO MERGE", theme::GREEN),
        Waiting::Nothing => ("✔ ALL GREEN", theme::GREEN),
    }
}

fn summary_line(pr: &PullRequest) -> Vec<Span<'static>> {
    if pr.checks.is_empty() {
        return vec![Span::styled("no checks", dim())];
    }
    let c = pr.counts();
    let mut spans = Vec::new();
    if let Some(waiting) = pr.waiting_for() {
        let (text, color) = waiting_label(waiting);
        spans.push(Span::styled(text, Style::new().fg(color).add_modifier(Modifier::BOLD)));
        spans.push(Span::raw("  "));
    }
    let mut push = |text: String, color: Color| {
        spans.push(Span::styled(text, Style::new().fg(color)));
        spans.push(Span::raw("  "));
    };
    if c.passed > 0 {
        push(format!("✓ {}", c.passed), theme::GREEN);
    }
    if c.failed > 0 {
        push(format!("✗ {}", c.failed), theme::RED);
    }
    if c.running > 0 {
        push(format!("◔ {}", c.running), theme::YELLOW);
    }
    if c.queued > 0 {
        push(format!("● {}", c.queued), ORANGE);
    }
    if c.skipped > 0 {
        push(format!("⊘ {}", c.skipped), theme::DIM);
    }
    spans
}

fn stage_style(state: CheckState) -> (&'static str, Color, &'static str) {
    match state {
        CheckState::Running => ("◔", theme::YELLOW, "running"),
        CheckState::Queued => ("●", ORANGE, "queued"),
        CheckState::Failed => ("✗", theme::RED, "failed"),
        CheckState::Passed => ("✓", theme::GREEN, "passed"),
        CheckState::Skipped => ("⊘", theme::DIM, "skipped"),
    }
}

fn stage_count(row: &PrRow) -> usize {
    row.live.map_or(0, |pr| pr.open_checks().len())
}

/// Lines a row takes (not counting the divider below it): the title, the badge and repo line,
/// the CI summary, then the stages.
fn row_height(row: &PrRow, width: usize) -> u16 {
    let stages = stage_count(row);
    let shown = stages.min(MAX_STAGE_LINES) + usize::from(stages > MAX_STAGE_LINES);
    (title_lines(row, width).len() + 2 + shown) as u16
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
    let marker = Span::styled(if selected { "▌ " } else { "  " }, Style::new().fg(theme::CYAN));
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let id = match row.number {
        Some(n) => format!("{}#{n}", row.repo),
        None => row.repo.clone(),
    };
    let mut right: Vec<Span<'static>> = Vec::new();
    let mut labels_text: Option<String> = None;
    let mut summary: Vec<Span<'static>> = vec![marker.clone(), Span::raw("  ")];
    let mut stages: Vec<Vec<Span<'static>>> = Vec::new();
    match row.live {
        Some(pr) => {
            if !pr.labels.is_empty() {
                labels_text = Some(pr.labels.iter().map(|l| format!("[{l}]")).collect::<Vec<_>>().join(" "));
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
            let open = pr.open_checks();
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
        None => summary.push(Span::styled("no live check data", dim())),
    }
    let mut left: Vec<Span<'static>> = vec![marker.clone(), badge(row)];
    if let Some(tag) = &row.tag {
        left.push(Span::styled(format!(" {tag}  "), bold.fg(theme::MAGENTA)));
    } else {
        left.push(Span::raw(" "));
    }
    left.push(Span::styled(id, Style::new().fg(theme::CYAN)));
    if let Some(labels) = labels_text {
        let room = width.saturating_sub(width_of(&left) + width_of(&right) + 3);
        if room >= 6 {
            right.insert(0, Span::styled(format!("{}  ", trunc(&labels, room)), dim()));
        }
    }
    let right_width = width_of(&right) + 1;
    let mut first = fit(left, width.saturating_sub(right_width));
    let pad = width.saturating_sub(width_of(&first) + width_of(&right));
    first.push(Span::raw(" ".repeat(pad)));
    first.extend(right);
    let style = if selected { Style::new().bg(theme::SELECT) } else { Style::new() };
    let button = details_width() + 1 + copy_width() + agent_width(row);
    let mut second = fit(summary, width.saturating_sub(button + 1));
    let gap = width.saturating_sub(width_of(&second) + button);
    second.push(Span::raw(" ".repeat(gap)));
    if row.owner.is_some() {
        second.push(Span::styled(AGENT_BUTTON, Style::new().fg(theme::CYAN)));
        second.push(Span::raw(" "));
    }
    second.push(Span::styled(COPY_BUTTON, Style::new().fg(theme::CYAN)));
    second.push(Span::raw(" "));
    second.push(Span::styled(DETAILS_BUTTON, Style::new().fg(theme::CYAN)));
    let mut lines: Vec<Line<'static>> = title_lines(row, width)
        .into_iter()
        .map(|t| Line::from(vec![marker.clone(), Span::styled(t, bold)]).style(style))
        .collect();
    lines.push(Line::from(first).style(style));
    lines.push(Line::from(second).style(style));
    lines.extend(stages.into_iter().map(|s| Line::from(fit(s, width)).style(style)));
    lines
}

/// `PULL REQUESTS · 3`, with `· 1 awaiting review` when some are all green and waiting for a reviewer.
fn title_text(name: &str, rows: &[PrRow]) -> String {
    let waiting = rows.iter().filter(|r| r.live.and_then(PullRequest::waiting_for) == Some(Waiting::Reviewer)).count();
    if waiting > 0 {
        format!(" {name} · {} · {waiting} awaiting review ", rows.len())
    } else {
        format!(" {name} · {} ", rows.len())
    }
}

pub fn draw_panel(f: &mut Frame, area: Rect, app: &App, hits: &mut Hits) {
    let rows = app.pr_rows();
    let focused = app.focus == Focus::Prs;
    let name = if app.screen == Screen::List { "PULL REQUESTS" } else { "STORY PULL REQUESTS" };
    let notice = app.current_notice().is_some();
    let status = if let Some(notice) = app.current_notice() {
        format!(" {notice} ")
    } else if app.prs.loading {
        " ↻ refreshing… ".to_string()
    } else {
        app.prs.updated.as_ref().map_or(String::new(), |u| format!(" ↻ updated {u} "))
    };
    let status_width = status.chars().count() as u16;
    let block = Block::bordered()
        .border_type(if focused { BorderType::Double } else { BorderType::Plain })
        .border_style(Style::new().fg(if focused { theme::BLUE } else { theme::BORDER }))
        .title(Span::styled(
            title_text(name, &rows),
            Style::new().fg(if focused { theme::BLUE } else { theme::FG }).add_modifier(Modifier::BOLD),
        ))
        .title(Line::from(Span::styled(status, dim())).right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);
    hits.push((area, Target::PrPanel));
    if !notice && status_width > 0 && status_width + 2 < area.width {
        hits.push((Rect::new(area.x + area.width - 1 - status_width, area.y, status_width, 1), Target::Refresh));
    }
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let mut top = inner.y;
    if let Some(err) = &app.prs.error {
        let line = Line::from(Span::styled(trunc(&format!("! {err}"), inner.width as usize), Style::new().fg(theme::RED)));
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
    let width = body.width as usize;
    let heights: Vec<u16> = rows.iter().map(|r| row_height(r, width) + 1).collect();
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
        let button = details_width() as u16;
        let copy = copy_width() as u16;
        let title_n = title_lines(row, width).len() as u16;
        let agent = agent_width(row) as u16;
        if body.width > button + copy + agent + 1 && h > title_n + 1 {
            let line = y + title_n + 1;
            if agent > 0 {
                hits.push((Rect::new(body.x + body.width - button - 1 - copy - agent, line, agent - 1, 1), Target::PrAgent(i)));
            }
            hits.push((Rect::new(body.x + body.width - button, line, button, 1), Target::PrDetails(i)));
            hits.push((Rect::new(body.x + body.width - button - 1 - copy, line, copy, 1), Target::PrCopy(i)));
        }
        if row.live.is_some_and(|pr| pr.is_draft) && h > title_n {
            hits.push((Rect::new(body.x + 2, y + title_n, BADGE_CLICK_WIDTH.min(body.width), 1), Target::PrBadge(i)));
        }
        for j in 0..stage_count(row).min(MAX_STAGE_LINES) {
            let line_y = y + title_n + 2 + j as u16;
            if line_y < y + h {
                hits.push((Rect::new(body.x, line_y, body.width, 1), Target::OpenCheck(i, j)));
            }
        }
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

/// One check line in the sheet; checks with a link get an `[ open ]` button at the right edge.
fn check_line(check: &Check, label_width: usize, inner_width: usize, now: DateTime<Utc>) -> Line<'static> {
    let (icon, color, word) = stage_style(check.state);
    let took = check_seconds(check, now).map_or(String::new(), |s| format!(" {}", duration_text(s)));
    let spans = vec![
        Span::styled(format!("  {icon} "), Style::new().fg(color)),
        Span::raw(format!("{:<label_width$}", check.label())),
        Span::styled(format!("   {word}{took}"), dim()),
    ];
    if check.url.is_none() {
        return Line::from(spans);
    }
    let button = SHEET_STAGE_OPEN.chars().count();
    let mut spans = fit(spans, inner_width.saturating_sub(button + 1));
    let gap = inner_width.saturating_sub(width_of(&spans) + button);
    spans.push(Span::raw(" ".repeat(gap)));
    spans.push(Span::styled(SHEET_STAGE_OPEN, Style::new().fg(theme::CYAN)));
    Line::from(spans)
}

/// The "mark ready for review?" confirmation over the whole screen.
pub fn draw_confirm(f: &mut Frame, area: Rect, app: &App, hits: &mut Hits) {
    let Some(confirm) = &app.confirm else {
        return;
    };
    let w = (area.width * 60 / 100).max(44).min(area.width);
    let h = 8.min(area.height);
    let rect = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
    let inner = w.saturating_sub(2) as usize;
    let mut lines = vec![
        Line::raw(""),
        Line::from(Span::styled(format!(" {}", confirm.id), Style::new().fg(theme::CYAN))),
    ];
    for t in wrap_text(&confirm.title, inner.saturating_sub(2), 2) {
        lines.push(Line::from(Span::styled(format!(" {t}"), Style::new().add_modifier(Modifier::BOLD))));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(" Reviewers are notified when it leaves draft.", dim())));
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ORANGE))
        .title(Span::styled(" Mark ready for review? ", Style::new().add_modifier(Modifier::BOLD)));
    theme::clear(f, rect);
    f.render_widget(Paragraph::new(lines).block(block), rect);
    let y = rect.y + rect.height.saturating_sub(2);
    let yes = Rect::new(rect.x + 2, y, CONFIRM_YES.chars().count() as u16, 1);
    let no = Rect::new(yes.x + yes.width + 2, y, CONFIRM_NO.chars().count() as u16, 1);
    f.render_widget(Paragraph::new(Span::styled(CONFIRM_YES, Style::new().fg(theme::GREEN).add_modifier(Modifier::BOLD))), yes);
    f.render_widget(Paragraph::new(Span::styled(CONFIRM_NO, Style::new().fg(theme::CYAN))), no);
    hits.push((rect, Target::Sheet));
    hits.push((yes, Target::ConfirmYes));
    hits.push((no, Target::ConfirmNo));
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
    let mut stage_hits: Hits = Vec::new();
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
            if let Some(waiting) = pr.waiting_for() {
                let words = match waiting {
                    Waiting::Reviewer => "all checks green: waiting for a reviewer",
                    Waiting::Merge => "all checks green and approved: ready to merge",
                    Waiting::Nothing => "all checks green (no review is required)",
                };
                lines.push(label_row("Waiting", words.into()));
            }
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
        lines.push(label_row("Task", tag.clone()));
    }
    lines.push(label_row("URL", row.url.clone()));
    if let Some(pr) = row.live {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(format!("Checks ({})", pr.checks.len()), Style::new().add_modifier(Modifier::BOLD))));
        if pr.checks.is_empty() {
            lines.push(Line::raw("  no checks"));
        }
        let label_width = pr.checks.iter().map(|c| c.label().chars().count()).max().unwrap_or(0);
        let inner_width = rect.width.saturating_sub(2) as usize;
        let first_stage = lines.len();
        for (k, check) in pr.ordered_checks().into_iter().enumerate() {
            lines.push(check_line(check, label_width, inner_width, now));
            let y = rect.y + 1 + (first_stage + k) as u16;
            if check.url.is_some() && y + 1 < rect.y + rect.height {
                let width = SHEET_STAGE_OPEN.chars().count() as u16;
                stage_hits.push((Rect::new(rect.x + 1 + inner_width as u16 - width, y, width, 1), Target::OpenStage(k)));
            }
        }
    }
    let title = if row.title.is_empty() { format!(" {id} ") } else { format!(" {id} · {} ", row.title) };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::BLUE))
        .title(Span::styled(title, Style::new().add_modifier(Modifier::BOLD)))
        .title_bottom(Line::from(vec![
            Span::raw(" "),
            Span::styled(OPEN_BUTTON, Style::new().fg(theme::CYAN)),
            Span::styled("  Esc or click outside to close ", dim()),
        ]));
    theme::clear(f, rect);
    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), rect);
    hits.push((rect, Target::Sheet));
    hits.extend(stage_hits);
    let button = Rect::new(rect.x + 2, rect.y + rect.height - 1, OPEN_BUTTON.chars().count() as u16, 1);
    hits.push((button, Target::OpenPr));
}

#[cfg(test)]
mod tests {
    use super::wrap_text;

    #[test]
    fn titles_wrap_on_word_boundaries() {
        assert_eq!(wrap_text("one two three four", 9, 3), vec!["one two", "three", "four"]);
        assert_eq!(wrap_text("short", 40, 3), vec!["short"]);
    }

    #[test]
    fn a_word_longer_than_the_line_is_broken_instead_of_hidden() {
        assert_eq!(wrap_text("abcdefghij", 4, 5), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn only_a_very_long_title_is_cut_and_the_cut_is_marked() {
        assert_eq!(wrap_text("aaa bbb ccc ddd eee", 7, 2), vec!["aaa bbb", "ccc dd…"]);
        assert_eq!(wrap_text("aaa bbb ccc ddd", 7, 2), vec!["aaa bbb", "ccc ddd"], "fits exactly: no mark");
    }

    #[test]
    fn empty_text_and_zero_width_do_not_panic() {
        assert_eq!(wrap_text("", 10, 3), vec![""]);
        assert_eq!(wrap_text("anything", 0, 3), vec![""]);
    }
}
