//! The command palette (Ctrl-P) and the right-click menu: every action in one list you can search.
//! The same box serves both; a menu has no search line and opens where you clicked.
use crate::app::{App, Focus, PrRow, Screen, Target};
use crate::{panel, theme};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Frame;

type Hits = Vec<(Rect, Target)>;

/// Something the palette can do. PR actions carry the PR's link, so they still mean the same PR if
/// the list reorders while the palette is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Reload,
    RefreshGithub,
    NextPanel,
    JumpToRed,
    ToggleDone,
    NewStory,
    ToggleAutoSync,
    ToggleDefaultAutoFix,
    ToggleDefaultAutoReview,
    ToggleSlowAway,
    ResetLayout,
    BackToList,
    SyncStory,
    Help,
    Quit,
    PlanTask,
    ImplementTask,
    FinishTask,
    TaskDetails,
    PrOpen(String),
    PrCopy(String),
    PrDetails(String),
    PrAgent(String),
    PrAutoFix(String),
    PrAutoReview(String),
    PrReady(String),
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub label: String,
    /// The key that does the same, shown at the right.
    pub hint: &'static str,
    pub action: Action,
}

fn entry(label: impl Into<String>, hint: &'static str, action: Action) -> Entry {
    Entry { label: label.into(), hint, action }
}

pub struct Palette {
    pub title: &'static str,
    pub query: String,
    /// A palette has a search line; a right-click menu does not.
    pub searchable: bool,
    pub sel: usize,
    pub entries: Vec<Entry>,
    /// Where a menu opens (the click); a palette is centred.
    pub anchor: Option<(u16, u16)>,
}

impl Palette {
    pub fn search(entries: Vec<Entry>) -> Palette {
        Palette { title: " Command palette ", query: String::new(), searchable: true, sel: 0, entries, anchor: None }
    }

    pub fn menu(title: &'static str, entries: Vec<Entry>, at: (u16, u16)) -> Palette {
        Palette { title, query: String::new(), searchable: false, sel: 0, entries, anchor: Some(at) }
    }

    /// The entries matching the search, as indices into `entries`: each word must appear in the label or key.
    pub fn visible(&self) -> Vec<usize> {
        let query = self.query.to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        (0..self.entries.len())
            .filter(|&i| {
                let text = format!("{} {}", self.entries[i].label, self.entries[i].hint).to_lowercase();
                words.iter().all(|w| text.contains(w))
            })
            .collect()
    }

    pub fn selected(&self) -> Option<&Entry> {
        let visible = self.visible();
        visible.get(self.sel.min(visible.len().saturating_sub(1))).map(|&i| &self.entries[i])
    }

    pub fn move_sel(&mut self, delta: i32) {
        let len = self.visible().len() as i32;
        if len > 0 {
            self.sel = (self.sel as i32 + delta).clamp(0, len - 1) as usize;
        }
    }

    pub fn type_char(&mut self, c: char) {
        self.query.push(c);
        self.sel = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.sel = 0;
    }
}

/// What can be done to one PR.
pub fn pr_entries(row: &PrRow) -> Vec<Entry> {
    let id = row.number.map_or(row.repo.clone(), |n| format!("{}#{n}", row.repo));
    let url = row.url.clone();
    let mut list = vec![
        entry(format!("Open {id} on GitHub"), "o", Action::PrOpen(url.clone())),
        entry(format!("Copy the link of {id}"), "c", Action::PrCopy(url.clone())),
        entry(format!("Details of {id}"), "Enter", Action::PrDetails(url.clone())),
    ];
    if row.has_agent {
        list.push(entry(format!("Go to the agent for {id}"), "a", Action::PrAgent(url.clone())));
    }
    if let Some(on) = row.auto {
        list.push(entry(format!("Turn {} auto-fix for {id}", if on { "off" } else { "on" }), "t", Action::PrAutoFix(url.clone())));
    }
    if let Some(on) = row.auto_review {
        list.push(entry(format!("Turn {} auto-review for {id}", if on { "off" } else { "on" }), "v", Action::PrAutoReview(url.clone())));
    }
    if row.live.is_some_and(|pr| pr.is_draft) {
        list.push(entry(format!("Mark {id} ready for review"), "m", Action::PrReady(url)));
    }
    list
}

/// What can be done to the selected task of a board.
pub fn task_entries(id: &str) -> Vec<Entry> {
    vec![
        entry(format!("Details of {id}"), "Enter", Action::TaskDetails),
        entry(format!("Plan {id}"), "p", Action::PlanTask),
        entry(format!("Implement {id}"), "i", Action::ImplementTask),
        entry(format!("Finish {id} (checks its PRs)"), "x", Action::FinishTask),
    ]
}

/// Everything the palette offers right now: what you are on first, then what is always there.
pub fn entries_for(app: &App) -> Vec<Entry> {
    let mut list = Vec::new();
    if app.focus == Focus::Prs {
        if let Some(row) = app.pr_rows().get(app.pr_sel) {
            list.extend(pr_entries(row));
        }
    }
    if app.screen == Screen::Board && app.focus == Focus::Main {
        if let Some(task) = app.selected_task() {
            list.extend(task_entries(&task.id));
        }
        if app.selected_task().is_some() {
            list.push(entry("Sync this story's PR states from GitHub", "R", Action::SyncStory));
        }
    }
    list.push(entry("Jump to the first red PR", "", Action::JumpToRed));
    list.push(entry("Refresh GitHub now", "r", Action::RefreshGithub));
    list.push(entry("Reload the boards now", "r", Action::Reload));
    list.push(entry("Move to the next panel", "Tab", Action::NextPanel));
    if app.screen == Screen::List {
        list.push(entry("New story…", "n", Action::NewStory));
        list.push(entry(if app.hide_done { "Show completed stories" } else { "Hide completed stories" }, "d", Action::ToggleDone));
    } else {
        list.push(entry("Back to the story list", "Esc", Action::BackToList));
        list.push(entry(format!("Turn auto-sync {}", if app.auto_sync { "off" } else { "on" }), "S", Action::ToggleAutoSync));
    }
    list.push(entry(format!("Turn auto-fix by default {}", if app.autofix { "off" } else { "on" }), "F", Action::ToggleDefaultAutoFix));
    list.push(entry(format!("Turn auto-review by default {}", if app.autocopilot { "off" } else { "on" }), "C", Action::ToggleDefaultAutoReview));
    list.push(entry(
        format!("Turn slow polling when out of focus {}", if app.attention.slow_when_away() { "off" } else { "on" }),
        "W",
        Action::ToggleSlowAway,
    ));
    list.push(entry("Reset the panel sizes", "=", Action::ResetLayout));
    list.push(entry("Show the keys", "?", Action::Help));
    list.push(entry("Quit", "q", Action::Quit));
    list
}

/// The box's rectangle: centred for a palette, at the click (kept on screen) for a menu.
fn place(p: &Palette, area: Rect, rows: usize) -> Rect {
    let widest = p
        .entries
        .iter()
        .map(|e| e.label.chars().count() + e.hint.chars().count() + 6)
        .max()
        .unwrap_or(20);
    let width = if p.searchable { 72.min(area.width.saturating_sub(4)) } else { (widest as u16 + 4).min(area.width) }.max(24).min(area.width);
    let chrome = if p.searchable { 4 } else { 2 };
    let height = ((rows + chrome) as u16).min(area.height).max(3);
    match p.anchor {
        Some((x, y)) => {
            let x = x.min(area.x + area.width.saturating_sub(width));
            let y = y.min(area.y + area.height.saturating_sub(height));
            Rect::new(x, y, width, height)
        }
        None => Rect::new(area.x + (area.width - width) / 2, area.y + area.height.saturating_sub(height) / 4, width, height),
    }
}

pub fn draw(f: &mut Frame, area: Rect, p: &Palette, hits: &mut Hits) {
    let visible = p.visible();
    let chrome = if p.searchable { 4 } else { 2 };
    let room = (area.height as usize).saturating_sub(chrome + 2).clamp(1, 18);
    let rows = visible.len().clamp(1, room);
    let rect = place(p, area, rows);
    theme::clear(f, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::BLUE))
        .title(Span::styled(p.title, Style::new().add_modifier(Modifier::BOLD)));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    hits.push((rect, Target::Sheet));
    let mut y = inner.y;
    if p.searchable {
        let line = Line::from(vec![Span::styled("▸ ", Style::new().fg(theme::ORANGE)), Span::raw(p.query.clone()), Span::styled("▏", Style::new().fg(theme::ORANGE))]);
        f.render_widget(Paragraph::new(line), Rect::new(inner.x, y, inner.width, 1));
        y += 1;
    }
    let list_rows = inner.height.saturating_sub(if p.searchable { 2 } else { 0 }) as usize;
    let selected = p.sel.min(visible.len().saturating_sub(1));
    let offset = if selected >= list_rows { selected + 1 - list_rows } else { 0 };
    if visible.is_empty() {
        f.render_widget(Paragraph::new(Span::styled("No matching action", panel::dim())), Rect::new(inner.x, y, inner.width, 1));
    }
    for (n, &index) in visible.iter().enumerate().skip(offset).take(list_rows) {
        let entry = &p.entries[index];
        let line_rect = Rect::new(inner.x, y, inner.width, 1);
        let on = n == selected;
        let width = inner.width as usize;
        let hint = if entry.hint.is_empty() { String::new() } else { format!("[ {} ]", entry.hint) };
        let label_room = width.saturating_sub(hint.chars().count() + 4);
        let label = panel::trunc(&entry.label, label_room);
        let pad = width.saturating_sub(2 + label.chars().count() + hint.chars().count());
        let marker = if on { Span::styled("▌ ", Style::new().fg(theme::CYAN)) } else { Span::raw("  ") };
        let spans = vec![marker, Span::styled(label, Style::new().fg(theme::FG)), Span::raw(" ".repeat(pad)), Span::styled(hint, panel::dim())];
        let line = Line::from(spans);
        f.render_widget(Paragraph::new(if on { line.style(Style::new().bg(theme::SELECT)) } else { line }), line_rect);
        hits.push((line_rect, Target::PaletteItem(n)));
        y += 1;
    }
    if p.searchable && inner.height >= 3 {
        let help = "↑↓ choose · Enter run · Esc close";
        f.render_widget(Paragraph::new(Span::styled(help, panel::dim())), Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Palette {
        Palette::search(vec![
            entry("Open acme/api#1 on GitHub", "o", Action::PrOpen("u".into())),
            entry("Copy the link of acme/api#1", "c", Action::PrCopy("u".into())),
            entry("Quit", "q", Action::Quit),
        ])
    }

    #[test]
    fn every_word_of_the_search_must_appear_and_the_order_is_kept() {
        let mut p = sample();
        assert_eq!(p.visible(), vec![0, 1, 2]);
        for c in "api open".chars() {
            p.type_char(c);
        }
        assert_eq!(p.visible(), vec![0], "words in any order, in the label");
        p.backspace();
        p.backspace();
        p.backspace();
        p.backspace();
        p.backspace();
        assert_eq!(p.visible(), vec![0, 1], "the key counts too");
        p.query = "Q".into();
        assert_eq!(p.visible(), vec![2], "case does not matter and the hint is searched");
        p.query = "zzz".into();
        assert!(p.visible().is_empty() && p.selected().is_none());
    }

    #[test]
    fn the_selection_moves_within_the_matches_and_resets_when_the_search_changes() {
        let mut p = sample();
        p.move_sel(1);
        assert_eq!(p.selected().map(|e| e.action.clone()), Some(Action::PrCopy("u".into())));
        p.move_sel(5);
        assert_eq!(p.selected().map(|e| e.action.clone()), Some(Action::Quit), "stops at the last");
        p.move_sel(-9);
        assert_eq!(p.sel, 0);
        p.move_sel(2);
        p.type_char('o');
        assert_eq!(p.sel, 0, "a new search starts at the top");
    }
}
