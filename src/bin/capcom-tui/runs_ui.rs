//! The manual runs panel (beside or behind the PR panel) and the run detail sheet.
use crate::app::{App, Focus, Screen, Target};
use crate::panel::{dim, fit, trunc, width_of, window_offset, ORANGE, SHEET_STAGE_OPEN};
use crate::prs::{duration_text, relative};
use crate::runs::{Job, Run, RunState};
use chrono::{DateTime, Utc};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use ratatui::Frame;

type Hits = Vec<(Rect, Target)>;

const MAX_STAGE_LINES: usize = 8;
const DETAILS_LABEL: &str = "[ details ]";
const SHEET_BUTTON: &str = "[ o Open run ]";

fn style_of(state: RunState) -> (&'static str, Color, &'static str) {
    match state {
        RunState::Running => ("◔", Color::Yellow, "running"),
        RunState::Waiting => ("◑", Color::Magenta, "waiting"),
        RunState::Queued => ("●", ORANGE, "queued"),
        RunState::Failed => ("✗", Color::Red, "failed"),
        RunState::Success => ("✓", Color::Green, "success"),
        RunState::Cancelled => ("⊘", Color::Gray, "cancelled"),
        RunState::Skipped => ("⊘", Color::DarkGray, "skipped"),
    }
}

fn badge(state: RunState) -> Span<'static> {
    let (_, color, _) = style_of(state);
    let text = match state {
        RunState::Running => "[RUNNING]  ",
        RunState::Waiting => "[WAITING]  ",
        RunState::Queued => "[QUEUED]   ",
        RunState::Failed => "[FAILED]   ",
        RunState::Success => "[SUCCESS]  ",
        RunState::Cancelled => "[CANCELLED]",
        RunState::Skipped => "[SKIPPED]  ",
    };
    Span::styled(text, Style::new().fg(color).add_modifier(Modifier::BOLD))
}

fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc))
}

fn run_seconds(run: &Run, now: DateTime<Utc>) -> Option<i64> {
    let start = parse_time(run.started_at.as_deref().unwrap_or(&run.created_at))?;
    let end = if run.is_active() { now } else { parse_time(&run.updated_at)? };
    Some((end - start).num_seconds().max(0))
}

fn job_seconds(job: &Job, now: DateTime<Utc>) -> Option<i64> {
    let start = parse_time(job.started_at.as_deref()?)?;
    let end = match &job.completed_at {
        Some(done) => parse_time(done)?,
        None => now,
    };
    Some((end - start).num_seconds().max(0))
}

fn stage_count(run: &Run) -> usize {
    run.open_jobs().len()
}

/// Lines a run takes (not counting the divider below it): header, title line, then its open stages.
fn run_height(run: &Run) -> u16 {
    let stages = stage_count(run);
    (2 + stages.min(MAX_STAGE_LINES) + usize::from(stages > MAX_STAGE_LINES)) as u16
}

/// Lines the run rows need (rows plus the dividers between them).
pub fn content_height(app: &App) -> u16 {
    app.visible_runs().iter().map(|r| run_height(r) + 1).sum::<u16>().saturating_sub(1)
}

fn job_line(job: &Job, label_width: usize, marker: &Span<'static>, now: DateTime<Utc>) -> Vec<Span<'static>> {
    let (icon, color, word) = style_of(job.state);
    let took = job_seconds(job, now).map_or(String::new(), |s| format!(" {}", duration_text(s)));
    vec![
        marker.clone(),
        Span::raw("  "),
        Span::styled(format!("{icon} "), Style::new().fg(color)),
        Span::raw(format!("{:<label_width$}", job.name)),
        Span::styled(format!("   {word}{took}"), dim()),
    ]
}

/// The lines of one run, and how many of them are stage lines (they follow the first two).
fn run_lines(run: &Run, width: usize, selected: bool, now: DateTime<Utc>) -> Vec<Line<'static>> {
    let marker = Span::styled(if selected { "▌ " } else { "  " }, Style::new().fg(Color::Cyan));
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let left = vec![
        marker.clone(),
        badge(run.state),
        Span::raw(" "),
        Span::styled(format!("{}  ", run.repo), Style::new().fg(Color::Cyan)),
        Span::styled(run.name.clone(), bold),
    ];
    let first = fit(left, width);
    let (_, color, word) = style_of(run.state);
    let took = run_seconds(run, now).map_or(String::new(), |s| format!(" {}", duration_text(s)));
    let mut second = vec![marker.clone(), Span::raw("  "), Span::raw(run.title.clone())];
    if !run.branch.is_empty() {
        second.push(Span::styled(format!("  {}", run.branch), Style::new().fg(Color::Cyan)));
    }
    second.push(Span::styled(format!("  {}", relative(&run.created_at, now)), dim()));
    second.push(Span::styled(format!("  {word}{took}"), Style::new().fg(color)));
    let button = DETAILS_LABEL.chars().count();
    let mut second = fit(second, width.saturating_sub(button + 1));
    let gap = width.saturating_sub(width_of(&second) + button);
    second.push(Span::raw(" ".repeat(gap)));
    second.push(Span::styled(DETAILS_LABEL, Style::new().fg(Color::Cyan)));
    let style = if selected { Style::new().bg(Color::Indexed(237)) } else { Style::new() };
    let mut lines = vec![Line::from(first).style(style), Line::from(fit(second, width)).style(style)];
    let open = run.open_jobs();
    let label_width = open.iter().take(MAX_STAGE_LINES).map(|j| j.name.chars().count()).max().unwrap_or(0);
    for job in open.iter().take(MAX_STAGE_LINES) {
        lines.push(Line::from(fit(job_line(job, label_width, &marker, now), width)).style(style));
    }
    if open.len() > MAX_STAGE_LINES {
        let more = open.len() - MAX_STAGE_LINES;
        let line = vec![
            marker,
            Span::raw("  "),
            Span::styled(format!("… +{more} more (Enter shows every stage)"), dim()),
        ];
        lines.push(Line::from(fit(line, width)).style(style));
    }
    lines
}

pub fn draw_panel(f: &mut Frame, area: Rect, app: &App, hits: &mut Hits) {
    let runs = app.visible_runs();
    let focused = app.focus == Focus::Runs;
    let name = if app.screen == Screen::List { "MANUAL RUNS" } else { "STORY MANUAL RUNS" };
    let compact = area.width < 40;
    let status = if compact {
        String::new()
    } else if app.runs.loading {
        " refreshing… ".to_string()
    } else {
        app.runs.updated.as_ref().map_or(String::new(), |u| format!(" updated {u} "))
    };
    let block = Block::bordered()
        .border_style(Style::new().fg(if focused { Color::Blue } else { Color::DarkGray }))
        .title(Span::styled(
            format!(" {name} · {} ", runs.len()),
            Style::new().fg(if focused { Color::Blue } else { Color::White }).add_modifier(Modifier::BOLD),
        ))
        .title(Line::from(Span::styled(status, dim())).right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);
    hits.push((area, Target::RunPanel));
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let mut top = inner.y;
    let mut note = |text: String, color: Color, top: &mut u16| {
        if *top < inner.y + inner.height {
            let line = Line::from(Span::styled(trunc(&text, inner.width as usize), Style::new().fg(color)));
            f.render_widget(Paragraph::new(line), Rect::new(inner.x, *top, inner.width, 1));
            *top += 1;
        }
    };
    if let Some(err) = &app.runs.error {
        note(format!("! {err}"), Color::Red, &mut top);
    }
    for warning in app.runs.warnings.iter().take(2) {
        note(format!("! {warning}"), Color::Yellow, &mut top);
    }
    let body = Rect::new(inner.x, top, inner.width, (inner.y + inner.height).saturating_sub(top));
    if runs.is_empty() {
        let message = if compact && app.runs.disabled {
            Some("Manual runs off".to_string())
        } else if compact && app.runs.loaded && app.runs.error.is_none() {
            Some("No manual runs".to_string())
        } else if app.runs.disabled {
            Some("Manual runs are off (started with --no-runs).".to_string())
        } else if app.screen == Screen::Board {
            Some("No manual runs in this story's repos in the last 3 hours.".to_string())
        } else if app.runs.error.is_some() {
            None
        } else if app.runs.loaded {
            Some("No manual runs in the last 3 hours.".to_string())
        } else {
            Some("Loading manual runs…".to_string())
        };
        let mut lines: Vec<Line<'static>> = message.into_iter().map(|m| Line::from(Span::styled(m, dim()))).collect();
        if !compact && app.runs.loaded && !app.runs.disabled && app.deploy_repos().is_empty() {
            lines.push(Line::from(Span::styled(
                "No repos to check yet: they come from your open PRs, story PRs and CAPCOM_DEPLOY_REPOS.",
                dim(),
            )));
        }
        if body.height > 0 {
            f.render_widget(Paragraph::new(lines), body);
        }
        return;
    }
    let heights: Vec<u16> = runs.iter().map(|r| run_height(r) + 1).collect();
    let selected = app.run_sel.min(runs.len() - 1);
    let offset = window_offset(&heights, selected, body.height);
    let now = Utc::now();
    let mut y = body.y;
    let bottom = body.y + body.height;
    for (i, run) in runs.iter().enumerate().skip(offset) {
        if y >= bottom {
            break;
        }
        let h = (heights[i] - 1).min(bottom - y);
        let rect = Rect::new(body.x, y, body.width, h);
        let lines = run_lines(run, body.width as usize, focused && i == selected, now);
        f.render_widget(Paragraph::new(lines), rect);
        hits.push((rect, Target::Run(i)));
        let button = DETAILS_LABEL.chars().count() as u16;
        if body.width > button && h >= 2 {
            hits.push((Rect::new(body.x + body.width - button, y + 1, button, 1), Target::RunDetails(i)));
        }
        for j in 0..stage_count(run).min(MAX_STAGE_LINES) {
            let line_y = y + 2 + j as u16;
            if line_y < y + h {
                hits.push((Rect::new(body.x, line_y, body.width, 1), Target::OpenJob(i, j)));
            }
        }
        y += heights[i] - 1;
        if i + 1 < runs.len() && y < bottom {
            let rule = "─".repeat(body.width as usize);
            f.render_widget(Paragraph::new(Span::styled(rule, dim())), Rect::new(body.x, y, body.width, 1));
            y += 1;
        }
    }
}

fn label_row(label: &str, value: String) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{label:<12}"), dim()), Span::raw(value)])
}

pub fn draw_sheet(f: &mut Frame, area: Rect, app: &App, hits: &mut Hits) {
    let runs = app.visible_runs();
    let Some(run) = runs.get(app.run_sel) else {
        return;
    };
    let w = (area.width * 80 / 100).max(50).min(area.width);
    let h = (area.height * 80 / 100).max(12).min(area.height);
    let rect = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
    let now = Utc::now();
    let (_, _, word) = style_of(run.state);
    let took = run_seconds(run, now).map_or(String::new(), |s| format!(" ({})", duration_text(s)));
    let mut lines = vec![
        label_row("State", format!("{word}{took}")),
        label_row("Repo", run.repo.clone()),
        label_row("Workflow", run.name.clone()),
        label_row("Title", run.title.clone()),
        label_row("Branch", run.branch.clone()),
        label_row("Started", format!("{} ({})", relative(&run.created_at, now), run.created_at)),
        label_row("URL", run.url.clone()),
        Line::raw(""),
        Line::from(Span::styled(format!("Stages ({})", run.jobs.len()), Style::new().add_modifier(Modifier::BOLD))),
    ];
    if run.jobs.is_empty() {
        lines.push(Line::raw("  none loaded (only running, queued or failed runs load their stages)"));
    }
    let label_width = run.jobs.iter().map(|j| j.name.chars().count()).max().unwrap_or(0);
    let inner_width = rect.width.saturating_sub(2) as usize;
    let first_stage = lines.len();
    let mut stage_hits: Hits = Vec::new();
    for (k, job) in run.ordered_jobs().into_iter().enumerate() {
        let (icon, color, word) = style_of(job.state);
        let took = job_seconds(job, now).map_or(String::new(), |s| format!(" {}", duration_text(s)));
        let spans = vec![
            Span::styled(format!("  {icon} "), Style::new().fg(color)),
            Span::raw(format!("{:<label_width$}", job.name)),
            Span::styled(format!("   {word}{took}"), dim()),
        ];
        let button = SHEET_STAGE_OPEN.chars().count();
        let mut spans = fit(spans, inner_width.saturating_sub(button + 1));
        let gap = inner_width.saturating_sub(width_of(&spans) + button);
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::styled(SHEET_STAGE_OPEN, Style::new().fg(Color::Cyan)));
        lines.push(Line::from(spans));
        let y = rect.y + 1 + (first_stage + k) as u16;
        if y + 1 < rect.y + rect.height {
            stage_hits.push((Rect::new(rect.x + 1 + (inner_width - button) as u16, y, button as u16, 1), Target::OpenStage(k)));
        }
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Blue))
        .title(Span::styled(
            format!(" {} · {} #{} ", run.repo, run.name, run.id),
            Style::new().add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(vec![
            Span::raw(" "),
            Span::styled(SHEET_BUTTON, Style::new().fg(Color::Cyan)),
            Span::styled("  Esc or click outside to close ", dim()),
        ]));
    f.render_widget(Clear, rect);
    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), rect);
    hits.push((rect, Target::Sheet));
    hits.extend(stage_hits);
    let button = Rect::new(rect.x + 2, rect.y + rect.height - 1, SHEET_BUTTON.chars().count() as u16, 1);
    hits.push((button, Target::OpenSelectedRun));
}
