use crate::herdr::{self, Herdr, Launch, Outcome};
use crate::prs::{PrMsg, PullRequest};
use crate::runs::{valid_repo, Batch, Run, RunMsg};
use crate::settings::{self, Settings, COLUMNS, DEFAULT_COLUMN_WEIGHT, DEFAULT_SPLIT, MIN_COLUMN_WEIGHT};
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use capcom::model::{Board, PrState, Status, StoryStatus, Task};
use capcom::rules;
use capcom::store;

const MAIN_COLUMNS: [Status; 5] = [
    Status::Todo,
    Status::Planning,
    Status::Planned,
    Status::Implementing,
    Status::Done,
];
const MAX_COLUMNS: usize = 6;
pub const STATUS_ORDER: [Status; 6] = [
    Status::Todo,
    Status::Planning,
    Status::Planned,
    Status::Implementing,
    Status::Done,
    Status::Dropped,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    List,
    Board,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Main,
    Prs,
    Runs,
}

/// Which bottom panel shows when the terminal is too narrow for both side by side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BottomTab {
    Prs,
    Runs,
}

/// A resize handle being dragged with the mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drag {
    Split,
    Height,
    Column { index: usize, x0: u16, x1: u16 },
}

/// Where the body and the bottom panels were last drawn, so a drag can turn a position into a size.
#[derive(Clone, Copy, Debug, Default)]
pub struct Geometry {
    pub body_y: u16,
    pub body_h: u16,
    pub bottom_x: u16,
    pub bottom_w: u16,
}

pub const SPLIT_RANGE: (u16, u16) = (25, 80);
pub const HEIGHT_RANGE: (u16, u16) = (15, 80);

/// What a mouse click on a rectangle does. Rectangles are recorded while drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Story(usize),
    ToggleDone,
    Back,
    Column(usize),
    Card { col: usize, row: usize },
    PrevColumn,
    NextColumn,
    Pr(usize),
    PrDetails(usize),
    PrCopy(usize),
    /// The `[DRAFT]` badge of a live draft pull request.
    PrBadge(usize),
    ConfirmYes,
    ConfirmNo,
    PrPanel,
    /// The "updated" label in a panel title: click to refresh now.
    Refresh,
    OpenPr,
    OpenCheck(usize, usize),
    Run(usize),
    RunDetails(usize),
    RunPanel,
    OpenJob(usize, usize),
    OpenStage(usize),
    OpenSelectedRun,
    Tab(BottomTab),
    NewStory,
    NewStoryStart,
    NewStoryCancel,
    SplitHandle,
    HeightHandle,
    /// The border between kanban column `i` and `i + 1`; carries the span of both columns.
    ColumnHandle(usize, u16, u16),
    Sheet,
}

pub struct Column<'a> {
    pub status: Status,
    pub tasks: Vec<&'a Task>,
}

pub struct StorySummary {
    pub key: String,
    pub title: String,
    pub status: StoryStatus,
    pub total: usize,
    pub done: usize,
    pub counts: [usize; 6],
    /// (pull request url, task id) for every PR recorded on the story's tasks.
    pub prs: Vec<(String, String)>,
    pub error: Option<String>,
}

fn summarize(root: &Path, key: &str) -> StorySummary {
    match store::load(root, key) {
        Ok(board) => {
            let mut counts = [0; 6];
            for task in &board.tasks {
                if let Some(i) = STATUS_ORDER.iter().position(|s| *s == task.status) {
                    counts[i] += 1;
                }
            }
            StorySummary {
                key: key.to_string(),
                title: board.story.title.clone(),
                status: board.story.status,
                total: board.tasks.len() - counts[5],
                done: counts[4],
                counts,
                prs: board
                    .tasks
                    .iter()
                    .flat_map(|t| t.prs.iter().map(|p| (p.url.clone(), t.id.clone())))
                    .collect(),
                error: None,
            }
        }
        Err(e) => StorySummary {
            key: key.to_string(),
            title: String::new(),
            status: StoryStatus::InProgress,
            total: 0,
            done: 0,
            counts: [0; 6],
            prs: Vec::new(),
            error: Some(format!("{e:#}")),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Signature(Vec<(String, u64)>);

pub fn signature(root: &Path) -> Signature {
    let mut entries = Vec::new();
    for key in store::list_keys(root).unwrap_or_default() {
        let bytes = std::fs::read(root.join(&key).join("board.json")).unwrap_or_default();
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        entries.push((key, hasher.finish()));
    }
    Signature(entries)
}

fn now() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

fn id_number(task: &Task) -> u32 {
    task.id.trim_start_matches('T').parse().unwrap_or(u32::MAX)
}

fn contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

#[derive(Default)]
pub struct PrData {
    pub items: Vec<PullRequest>,
    pub error: Option<String>,
    pub updated: Option<String>,
    pub loaded: bool,
    pub loading: bool,
    pub disabled: bool,
}

#[derive(Default)]
pub struct RunData {
    pub items: Vec<Run>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
    pub updated: Option<String>,
    pub loaded: bool,
    pub loading: bool,
    pub disabled: bool,
}

/// One line of the pull request panel: a live GitHub PR, or a PR known only from the board.
pub struct PrRow<'a> {
    pub live: Option<&'a PullRequest>,
    pub url: String,
    pub repo: String,
    pub number: Option<u64>,
    pub title: String,
    pub tag: Option<String>,
    pub board_state: Option<PrState>,
}

fn pr_id(row: &PrRow) -> String {
    row.number.map_or(row.repo.clone(), |n| format!("{}#{n}", row.repo))
}

fn same_url(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

fn url_parts(url: &str) -> Option<(String, u64)> {
    let parts: Vec<&str> = url.trim_end_matches('/').split('/').collect();
    if parts.len() >= 7 && parts[5] == "pull" {
        Some((format!("{}/{}", parts[3], parts[4]), parts[6].parse().ok()?))
    } else {
        None
    }
}

fn open_in_browser(url: &str) {
    if !url.starts_with("https://") {
        return;
    }
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let child = Command::new(program)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// A kanban card being pressed: dropping it on another column may start an agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardDrag {
    pub col: usize,
    pub row: usize,
    /// The press was on the already selected card, so a plain click opens its detail on release.
    pub open: bool,
    pub over: Option<usize>,
}

/// A pull request waiting for "mark ready for review" to be confirmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    pub url: String,
    pub id: String,
    pub title: String,
}

const NOTICE_SECONDS: u64 = 8;

pub type Readier = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Put text on the clipboard: the terminal's own clipboard (works over ssh) and, when present,
/// the desktop tool.
fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let _ = out.flush();
    let tools: &[(&str, &[&str])] = &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("pbcopy", &[]),
    ];
    for (program, args) in tools {
        let child = Command::new(program)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            break;
        }
    }
}

fn gh_mark_ready(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("not a pull request link".into());
    }
    let mut cmd = Command::new("gh");
    cmd.args(["pr", "ready", url]);
    match capcom::refresh::run_with_timeout(cmd, std::time::Duration::from_secs(30)) {
        Ok(Some(out)) if out.status.success() => Ok(()),
        Ok(Some(out)) => Err(String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("gh failed").to_string()),
        Ok(None) => Err("gh timed out".into()),
        Err(e) => Err(e.to_string()),
    }
}

pub struct App {
    pub root: PathBuf,
    pub keys: Vec<String>,
    pub stories: Vec<StorySummary>,
    pub screen: Screen,
    pub hide_done: bool,
    pub list_sel: usize,
    pub story: usize,
    pub board: Option<Board>,
    pub error: Option<String>,
    pub col: usize,
    pub row: [usize; MAX_COLUMNS],
    pub detail: bool,
    pub help: bool,
    pub updated: String,
    pub hits: RefCell<Vec<(Rect, Target)>>,
    pub focus: Focus,
    pub pr_sel: usize,
    pub pr_sheet: bool,
    pub prs: PrData,
    pub runs: RunData,
    pub run_sel: usize,
    pub run_sheet: bool,
    pub tab: BottomTab,
    pub extra_repos: Vec<String>,
    /// Width of the PR panel, as a percentage of the bottom area, when both panels are side by side.
    pub split_pct: u16,
    /// Height of the bottom area as a percentage of the body; None lets the layout decide.
    pub bottom_pct: Option<u16>,
    /// Once you size the panels yourself, an empty runs panel no longer shrinks on its own.
    pub split_pinned: bool,
    /// Relative widths of the kanban columns.
    pub col_weights: [u16; COLUMNS],
    /// Where the panel sizes are remembered; None keeps them for this session only.
    pub settings_path: Option<PathBuf>,
    pub geometry: Cell<Geometry>,
    drag: Option<Drag>,
    pub opener: Box<dyn Fn(&str)>,
    pub copier: Box<dyn Fn(&str)>,
    pub readier: Readier,
    pub confirm: Option<Confirm>,
    pub herdr: Option<Arc<dyn Herdr>>,
    /// The text typed or pasted into the "new story" box, while it is open.
    pub new_story: Option<String>,
    pub card_drag: Option<CardDrag>,
    launch_done: (Sender<String>, Receiver<String>),
    pub started: std::time::Instant,
    notice: Option<(String, std::time::Instant)>,
    ready_done: (Sender<Result<String, String>>, Receiver<Result<String, String>>),
    feed: Option<(Receiver<PrMsg>, Sender<()>)>,
    run_feed: Option<(Receiver<RunMsg>, Sender<()>)>,
    run_repos: Option<Arc<Mutex<Vec<String>>>>,
    sig: Signature,
}

impl App {
    pub fn new(root: PathBuf, preferred: Option<String>) -> App {
        let keys = store::list_keys(&root).unwrap_or_default();
        let preferred_pos = preferred.and_then(|k| keys.iter().position(|x| *x == k));
        let stories = keys.iter().map(|k| summarize(&root, k)).collect();
        let mut app = App {
            sig: signature(&root),
            root,
            keys,
            stories,
            screen: if preferred_pos.is_some() { Screen::Board } else { Screen::List },
            hide_done: true,
            list_sel: 0,
            story: preferred_pos.unwrap_or(0),
            board: None,
            error: None,
            col: 0,
            row: [0; MAX_COLUMNS],
            detail: false,
            help: false,
            updated: now(),
            hits: RefCell::new(Vec::new()),
            focus: Focus::Main,
            pr_sel: 0,
            pr_sheet: false,
            prs: PrData::default(),
            runs: RunData::default(),
            run_sel: 0,
            run_sheet: false,
            tab: BottomTab::Prs,
            extra_repos: Vec::new(),
            split_pct: DEFAULT_SPLIT,
            bottom_pct: None,
            split_pinned: false,
            col_weights: [DEFAULT_COLUMN_WEIGHT; COLUMNS],
            settings_path: None,
            geometry: Cell::new(Geometry::default()),
            drag: None,
            opener: Box::new(open_in_browser),
            copier: Box::new(copy_to_clipboard),
            readier: Arc::new(gh_mark_ready),
            confirm: None,
            herdr: None,
            new_story: None,
            card_drag: None,
            launch_done: std::sync::mpsc::channel(),
            started: std::time::Instant::now(),
            notice: None,
            ready_done: std::sync::mpsc::channel(),
            feed: None,
            run_feed: None,
            run_repos: None,
        };
        app.load_current();
        app.focus_first_column();
        if preferred_pos.is_some() {
            let key = app.keys[app.story].clone();
            app.reselect(Some(key));
        }
        app
    }

    fn load_current(&mut self) {
        let Some(key) = self.keys.get(self.story).cloned() else {
            self.board = None;
            self.error = None;
            return;
        };
        if self.board.as_ref().map(|b| b.story.key.as_str()) != Some(key.as_str()) {
            self.board = None;
        }
        match store::load(&self.root, &key) {
            Ok(board) => {
                self.board = Some(board);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("{e:#}")),
        }
    }

    fn focus_first_column(&mut self) {
        self.row = [0; MAX_COLUMNS];
        self.col = self
            .columns()
            .iter()
            .position(|c| !c.tasks.is_empty())
            .unwrap_or(0);
    }

    pub fn needs_reload(&self) -> bool {
        signature(&self.root) != self.sig
    }

    pub fn reload(&mut self) {
        let selected = self.selected_task().map(|t| t.id.clone());
        let list_key = self.selected_story_key();
        let current = self.keys.get(self.story).cloned();
        self.keys = store::list_keys(&self.root).unwrap_or_default();
        self.stories = self.keys.iter().map(|k| summarize(&self.root, k)).collect();
        self.story = current
            .and_then(|k| self.keys.iter().position(|x| *x == k))
            .unwrap_or(0);
        self.load_current();
        self.sig = signature(&self.root);
        self.updated = now();
        self.reselect(list_key);
        if let Some(id) = selected {
            self.select_task(&id);
        }
        self.clamp();
        self.sync_run_repos();
    }

    pub fn visible_stories(&self) -> Vec<&StorySummary> {
        self.stories
            .iter()
            .filter(|s| !self.hide_done || s.status != StoryStatus::Done)
            .collect()
    }

    pub fn hidden_done_count(&self) -> usize {
        if !self.hide_done {
            return 0;
        }
        self.stories.iter().filter(|s| s.status == StoryStatus::Done).count()
    }

    fn selected_story_key(&self) -> Option<String> {
        self.visible_stories().get(self.list_sel).map(|s| s.key.clone())
    }

    fn reselect(&mut self, key: Option<String>) {
        let (pos, len) = {
            let visible = self.visible_stories();
            let pos = key.and_then(|k| visible.iter().position(|s| s.key == k));
            (pos, visible.len())
        };
        self.list_sel = pos.unwrap_or(self.list_sel).min(len.saturating_sub(1));
    }

    pub fn toggle_hide_done(&mut self) {
        let key = self.selected_story_key();
        self.hide_done = !self.hide_done;
        self.reselect(key);
    }

    pub fn list_move(&mut self, delta: i32) {
        let len = self.visible_stories().len() as i32;
        if len > 0 {
            self.list_sel = (self.list_sel as i32 + delta).clamp(0, len - 1) as usize;
        }
    }

    pub fn open_selected(&mut self) {
        if let Some(key) = self.selected_story_key() {
            self.open_story(&key);
        }
    }

    pub fn open_story(&mut self, key: &str) {
        let Some(i) = self.keys.iter().position(|k| k == key) else {
            return;
        };
        self.story = i;
        self.load_current();
        self.focus_first_column();
        self.screen = Screen::Board;
        self.detail = false;
        self.help = false;
        self.reset_pr_view();
        self.reselect(Some(key.to_string()));
    }

    pub fn back(&mut self) {
        self.screen = Screen::List;
        self.detail = false;
        self.help = false;
        self.reset_pr_view();
        let key = self.keys.get(self.story).cloned();
        self.reselect(key);
    }

    /// Position of the open story among the visible stories, as (1-based position, count).
    pub fn story_position(&self) -> Option<(usize, usize)> {
        let key = self.keys.get(self.story)?;
        let visible = self.visible_stories();
        let pos = visible.iter().position(|s| &s.key == key)?;
        Some((pos + 1, visible.len()))
    }

    fn select_task(&mut self, id: &str) {
        let found = self.columns().iter().enumerate().find_map(|(c, col)| {
            col.tasks.iter().position(|t| t.id == id).map(|r| (c, r))
        });
        if let Some((c, r)) = found {
            self.col = c;
            self.row[c] = r;
        }
    }

    fn clamp(&mut self) {
        let lens: Vec<usize> = self.columns().iter().map(|c| c.tasks.len()).collect();
        self.col = self.col.min(lens.len().saturating_sub(1));
        for (i, len) in lens.iter().enumerate() {
            self.row[i] = self.row[i].min(len.saturating_sub(1));
        }
        self.pr_clamp();
        self.run_clamp();
    }

    fn run_clamp(&mut self) {
        let len = self.visible_runs().len();
        self.run_sel = self.run_sel.min(len.saturating_sub(1));
    }

    fn pr_clamp(&mut self) {
        let len = self.pr_rows().len();
        self.pr_sel = self.pr_sel.min(len.saturating_sub(1));
    }

    pub fn attach_feed(&mut self, rx: Receiver<PrMsg>, wake: Sender<()>) {
        self.feed = Some((rx, wake));
    }

    pub fn poll_feed(&mut self) {
        let mut messages = Vec::new();
        if let Some((rx, _)) = &self.feed {
            while let Ok(msg) = rx.try_recv() {
                messages.push(msg);
            }
        }
        for msg in messages {
            match msg {
                PrMsg::Started => self.apply_started(),
                PrMsg::Result(result) => self.apply_prs(result),
            }
        }
    }

    fn refresh_prs(&self) {
        if let Some((_, wake)) = &self.feed {
            let _ = wake.send(());
        }
        if let Some((_, wake)) = &self.run_feed {
            let _ = wake.send(());
        }
    }

    pub fn apply_started(&mut self) {
        self.prs.loading = true;
    }

    pub fn apply_prs(&mut self, result: Result<Vec<PullRequest>, String>) {
        self.prs.loading = false;
        self.prs.loaded = true;
        match result {
            Ok(items) => {
                self.prs.items = items;
                self.prs.error = None;
                self.prs.updated = Some(now());
            }
            Err(e) => self.prs.error = Some(e),
        }
        self.pr_clamp();
        self.sync_run_repos();
    }

    pub fn attach_runs(&mut self, rx: Receiver<RunMsg>, wake: Sender<()>, repos: Arc<Mutex<Vec<String>>>) {
        self.run_feed = Some((rx, wake));
        self.run_repos = Some(repos);
        self.sync_run_repos();
    }

    pub fn poll_runs(&mut self) {
        let mut messages = Vec::new();
        if let Some((rx, _)) = &self.run_feed {
            while let Ok(msg) = rx.try_recv() {
                messages.push(msg);
            }
        }
        for msg in messages {
            match msg {
                RunMsg::Started => self.apply_run_started(),
                RunMsg::Result(result) => self.apply_runs(result),
            }
        }
    }

    pub fn apply_run_started(&mut self) {
        self.runs.loading = true;
    }

    pub fn apply_runs(&mut self, result: Result<Batch, String>) {
        self.runs.loading = false;
        self.runs.loaded = true;
        match result {
            Ok(batch) => {
                self.runs.items = batch.runs;
                self.runs.warnings = batch.warnings;
                self.runs.error = None;
                self.runs.updated = Some(now());
            }
            Err(e) => self.runs.error = Some(e),
        }
        self.run_clamp();
    }

    /// Repos to look for manual runs in: your open PRs, the PRs recorded on stories, and the extra list.
    pub fn deploy_repos(&self) -> Vec<String> {
        let mut repos: Vec<String> = self.prs.items.iter().map(|p| p.repo.clone()).collect();
        for story in &self.stories {
            repos.extend(story.prs.iter().filter_map(|(url, _)| url_parts(url).map(|(repo, _)| repo)));
        }
        repos.extend(self.extra_repos.iter().cloned());
        repos.retain(|r| valid_repo(r));
        repos.sort();
        repos.dedup();
        repos
    }

    fn sync_run_repos(&self) {
        let (Some(shared), Some((_, wake))) = (&self.run_repos, &self.run_feed) else {
            return;
        };
        let wanted = self.deploy_repos();
        let mut current = shared.lock().expect("repo list lock");
        if *current != wanted {
            *current = wanted;
            drop(current);
            let _ = wake.send(());
        }
    }

    fn story_repos(&self) -> Vec<String> {
        let Some(board) = &self.board else {
            return Vec::new();
        };
        board
            .tasks
            .iter()
            .flat_map(|t| t.prs.iter())
            .filter_map(|p| url_parts(&p.url).map(|(repo, _)| repo))
            .collect()
    }

    /// Every manual run on the main screen; only runs in the open story's repos on a board.
    pub fn visible_runs(&self) -> Vec<&Run> {
        match self.screen {
            Screen::List => self.runs.items.iter().collect(),
            Screen::Board => {
                let repos = self.story_repos();
                self.runs
                    .items
                    .iter()
                    .filter(|r| repos.iter().any(|x| x.eq_ignore_ascii_case(&r.repo)))
                    .collect()
            }
        }
    }

    pub fn run_move(&mut self, delta: i32) {
        let len = self.visible_runs().len() as i32;
        if len > 0 {
            self.run_sel = (self.run_sel as i32 + delta).clamp(0, len - 1) as usize;
        }
    }

    pub fn open_run(&self, index: usize) {
        let url = self.visible_runs().get(index).map(|r| r.url.clone());
        if let Some(url) = url {
            (self.opener)(&url);
        }
    }

    pub fn open_job(&self, run: usize, job: usize) {
        let url = self
            .visible_runs()
            .get(run)
            .and_then(|r| r.open_jobs().get(job).map(|j| j.url.clone()));
        if let Some(url) = url {
            (self.opener)(&url);
        }
    }

    pub fn open_selected_run(&self) {
        self.open_run(self.run_sel);
    }

    pub fn settings(&self) -> Settings {
        Settings {
            split_pct: self.split_pct,
            bottom_pct: self.bottom_pct,
            split_pinned: self.split_pinned,
            columns: self.col_weights.to_vec(),
        }
    }

    pub fn load_settings(&mut self) {
        let Some(path) = &self.settings_path else {
            return;
        };
        let saved = settings::load(path);
        self.split_pct = saved.split_pct;
        self.bottom_pct = saved.bottom_pct;
        self.split_pinned = saved.split_pinned;
        for (slot, weight) in self.col_weights.iter_mut().zip(saved.columns) {
            *slot = weight;
        }
    }

    /// Remember the sizes; failing to write them never gets in the way.
    fn save_settings(&self) {
        if let Some(path) = &self.settings_path {
            let _ = settings::save(path, &self.settings());
        }
    }

    pub fn resize_split(&mut self, delta: i16) {
        self.split_pct = (self.split_pct as i16 + delta).clamp(SPLIT_RANGE.0 as i16, SPLIT_RANGE.1 as i16) as u16;
        self.split_pinned = true;
        self.save_settings();
    }

    /// Grow (or shrink) the selected kanban column at the expense of its neighbour.
    pub fn resize_column(&mut self, delta: i16) {
        let count = self.columns().len();
        if self.screen != Screen::Board || count < 2 {
            return;
        }
        let i = self.col.min(count - 1);
        let j = if i + 1 < count { i + 1 } else { i - 1 };
        let min = MIN_COLUMN_WEIGHT as i16;
        let (wi, wj) = (self.col_weights[i] as i16, self.col_weights[j] as i16);
        let change = delta.clamp(min - wi, wj - min);
        self.col_weights[i] = (wi + change) as u16;
        self.col_weights[j] = (wj - change) as u16;
        self.save_settings();
    }

    pub fn resize_bottom(&mut self, delta: i16) {
        let base = self.bottom_pct.unwrap_or(match self.screen {
            Screen::List => 45,
            Screen::Board => 40,
        });
        let next = (base as i16 + delta).clamp(HEIGHT_RANGE.0 as i16, HEIGHT_RANGE.1 as i16);
        self.bottom_pct = Some(next as u16);
        self.save_settings();
    }

    pub fn reset_layout(&mut self) {
        let defaults = Settings::default();
        self.split_pct = defaults.split_pct;
        self.bottom_pct = defaults.bottom_pct;
        self.split_pinned = defaults.split_pinned;
        self.col_weights = [DEFAULT_COLUMN_WEIGHT; COLUMNS];
        self.save_settings();
    }

    pub fn on_drag(&mut self, x: u16, y: u16) {
        if self.card_drag.is_some() {
            let over = match self.hit(x, y) {
                Some(Target::Column(c) | Target::Card { col: c, .. }) => Some(c),
                _ => None,
            };
            if let Some(drag) = &mut self.card_drag {
                drag.over = over;
            }
            return;
        }
        let g = self.geometry.get();
        match self.drag {
            Some(Drag::Split) if g.bottom_w > 0 => {
                let pct = u32::from(x.saturating_sub(g.bottom_x)) * 100 / u32::from(g.bottom_w);
                self.split_pct = (pct as u16).clamp(SPLIT_RANGE.0, SPLIT_RANGE.1);
                self.split_pinned = true;
            }
            Some(Drag::Height) if g.body_h > 0 => {
                let bottom = (i32::from(g.body_y) + i32::from(g.body_h) - i32::from(y)).max(0) as u32;
                let pct = bottom * 100 / u32::from(g.body_h);
                self.bottom_pct = Some((pct as u16).clamp(HEIGHT_RANGE.0, HEIGHT_RANGE.1));
            }
            Some(Drag::Column { index, x0, x1 }) if x1 > x0 && index + 1 < COLUMNS => {
                let total = self.col_weights[index] + self.col_weights[index + 1];
                let inside = x.clamp(x0, x1) - x0;
                let left = (u32::from(total) * u32::from(inside) / u32::from(x1 - x0)) as u16;
                let left = left.clamp(MIN_COLUMN_WEIGHT, total - MIN_COLUMN_WEIGHT);
                self.col_weights[index] = left;
                self.col_weights[index + 1] = total - left;
            }
            _ => {}
        }
    }

    pub fn on_release(&mut self) {
        if let Some(drag) = self.card_drag.take() {
            match drag.over {
                Some(to) if to != drag.col => {
                    let cols = self.columns();
                    let id = cols.get(drag.col).and_then(|c| c.tasks.get(drag.row)).map(|t| t.id.clone());
                    let status = cols.get(to).map(|c| c.status);
                    if let (Some(id), Some(status)) = (id, status) {
                        self.start_work(&id, status);
                    }
                }
                _ if drag.open => self.detail = true,
                _ => {}
            }
            return;
        }
        if self.drag.take().is_some() {
            self.save_settings();
        }
    }

    fn close_overlays(&mut self) {
        self.detail = false;
        self.help = false;
        self.pr_sheet = false;
        self.run_sheet = false;
    }

    fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
        match focus {
            Focus::Prs => self.tab = BottomTab::Prs,
            Focus::Runs => self.tab = BottomTab::Runs,
            Focus::Main => {}
        }
    }

    pub fn pr_tag(&self, url: &str) -> Option<String> {
        self.stories.iter().find_map(|story| {
            story
                .prs
                .iter()
                .find(|(u, _)| same_url(u, url))
                .map(|(_, task)| format!("{} · {task}", story.key))
        })
    }

    /// The panel rows: every open PR on the main screen, only the story's own PRs on the board.
    pub fn pr_rows(&self) -> Vec<PrRow<'_>> {
        match self.screen {
            Screen::List => self
                .prs
                .items
                .iter()
                .map(|p| PrRow {
                    live: Some(p),
                    url: p.url.clone(),
                    repo: p.repo.clone(),
                    number: Some(p.number),
                    title: p.title.clone(),
                    tag: self.pr_tag(&p.url),
                    board_state: None,
                })
                .collect(),
            Screen::Board => {
                let Some(board) = &self.board else {
                    return Vec::new();
                };
                let mut tasks: Vec<&Task> = board.tasks.iter().collect();
                tasks.sort_by_key(|t| id_number(t));
                let mut rows = Vec::new();
                for task in tasks {
                    for pr in &task.prs {
                        let live = self.prs.items.iter().find(|p| same_url(&p.url, &pr.url));
                        let parts = url_parts(&pr.url);
                        rows.push(PrRow {
                            live,
                            url: live.map_or_else(|| pr.url.clone(), |p| p.url.clone()),
                            repo: live.map_or_else(
                                || parts.as_ref().map_or_else(|| pr.repo.clone(), |(r, _)| r.clone()),
                                |p| p.repo.clone(),
                            ),
                            number: live.map(|p| p.number).or(parts.map(|(_, n)| n)),
                            title: live.map_or_else(String::new, |p| p.title.clone()),
                            tag: Some(task.id.clone()),
                            board_state: Some(pr.state),
                        });
                    }
                }
                rows
            }
        }
    }

    pub fn pr_move(&mut self, delta: i32) {
        let len = self.pr_rows().len() as i32;
        if len > 0 {
            self.pr_sel = (self.pr_sel as i32 + delta).clamp(0, len - 1) as usize;
        }
    }

    pub fn open_pr(&self, index: usize) {
        let url = self.pr_rows().get(index).map(|r| r.url.clone());
        if let Some(url) = url {
            (self.opener)(&url);
        }
    }

    /// Open one CI check of a PR row; a check without a link opens the PR instead.
    pub fn open_check(&self, pr: usize, check: usize) {
        let rows = self.pr_rows();
        let Some(row) = rows.get(pr) else {
            return;
        };
        let url = row
            .live
            .and_then(|p| p.open_checks().get(check).and_then(|c| c.url.clone()))
            .unwrap_or_else(|| row.url.clone());
        (self.opener)(&url);
    }

    /// Open the k-th stage listed in the sheet that is open (PR checks or run stages).
    pub fn open_sheet_stage(&self, k: usize) {
        let url = if self.pr_sheet {
            let rows = self.pr_rows();
            rows.get(self.pr_sel)
                .and_then(|r| r.live)
                .and_then(|p| p.ordered_checks().get(k).and_then(|c| c.url.clone()))
        } else if self.run_sheet {
            self.visible_runs()
                .get(self.run_sel)
                .and_then(|r| r.ordered_jobs().get(k).map(|j| j.url.clone()))
        } else {
            None
        };
        if let Some(url) = url {
            (self.opener)(&url);
        }
    }

    pub fn current_notice(&self) -> Option<&str> {
        match &self.notice {
            Some((text, at)) if at.elapsed().as_secs() < NOTICE_SECONDS => Some(text),
            _ => None,
        }
    }

    fn set_notice(&mut self, text: String) {
        self.notice = Some((text, std::time::Instant::now()));
    }

    pub fn copy_pr(&mut self, index: usize) {
        let Some((url, id)) = self.pr_rows().get(index).map(|r| (r.url.clone(), pr_id(r))) else {
            return;
        };
        (self.copier)(&url);
        self.set_notice(format!("copied {id}"));
    }

    /// Ask before marking a live draft ready for review.
    pub fn ask_mark_ready(&mut self, index: usize) {
        let confirm = self.pr_rows().get(index).and_then(|r| {
            r.live.filter(|p| p.is_draft).map(|p| Confirm { url: r.url.clone(), id: pr_id(r), title: p.title.clone() })
        });
        if confirm.is_some() {
            self.confirm = confirm;
        }
    }

    pub fn confirm_ready(&mut self) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        self.set_notice(format!("marking {} ready…", confirm.id));
        let readier = self.readier.clone();
        let done = self.ready_done.0.clone();
        let wake = self.feed.as_ref().map(|(_, w)| w.clone());
        std::thread::spawn(move || {
            let result = readier(&confirm.url).map(|_| confirm.id.clone()).map_err(|e| format!("{}: {e}", confirm.id));
            let _ = done.send(result);
            if let Some(wake) = wake {
                let _ = wake.send(());
            }
        });
    }

    /// Start an agent in Herdr in the background and report how it went.
    pub fn run_launch(&mut self, launch: Launch) {
        let Some(herdr) = self.herdr.clone() else {
            self.set_notice("not running inside Herdr, so no agent can be started".into());
            return;
        };
        self.set_notice(format!("starting {}…", launch.agent));
        let done = self.launch_done.0.clone();
        std::thread::spawn(move || {
            let text = match herdr.launch(&launch) {
                Ok(Outcome::Started) => format!("started {} in Herdr workspace {}", launch.agent, launch.workspace),
                Ok(Outcome::Focused) => format!("{} is already running: brought to the front", launch.agent),
                Err(e) => format!("could not start {}: {e}", launch.agent),
            };
            let _ = done.send(text);
        });
    }

    /// Start the skill that moves a task into `to`: planning from todo, implementing from planned (or
    /// straight from todo, for spikes and manual testing).
    pub fn start_work(&mut self, task_id: &str, to: Status) {
        let Some(key) = self.keys.get(self.story).cloned() else {
            return;
        };
        let Some(board) = &self.board else {
            return;
        };
        let Some(task) = board.task(task_id) else {
            return;
        };
        let (skill, tab, phase) = match (task.status, to) {
            (Status::Todo, Status::Planning) => ("story-plan-task", "plan", "plan"),
            (Status::Planned | Status::Todo, Status::Implementing) => {
                if !rules::is_ready(board, task) {
                    let waiting = task
                        .depends_on
                        .iter()
                        .filter(|d| board.task(d).map_or(true, |t| t.status != Status::Done))
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.set_notice(format!("{task_id} waits for {waiting} to be done"));
                    return;
                }
                ("story-implement-task", "implement", "impl")
            }
            (from, to) if from == to => return,
            _ => {
                self.set_notice("drop a TODO card on PLANNING to plan it, or a TODO or PLANNED card on IMPLEMENTING".into());
                return;
            }
        };
        let launch = Launch {
            workspace: key.clone(),
            tab: format!("{task_id} {tab}"),
            agent: herdr::agent_name(&[&key, task_id, phase]),
            prompt: format!("/{skill} {key} {task_id}"),
        };
        self.run_launch(launch);
    }

    pub fn start_selected(&mut self, to: Status) {
        if let Some(id) = self.selected_task().map(|t| t.id.clone()) {
            self.start_work(&id, to);
        }
    }

    pub fn open_new_story(&mut self) {
        self.new_story = Some(String::new());
    }

    pub fn on_paste(&mut self, text: &str) {
        if let Some(buffer) = &mut self.new_story {
            buffer.push_str(&text.replace(['\r', '\n'], " "));
        }
    }

    pub fn submit_new_story(&mut self) {
        let Some(text) = self.new_story.clone() else {
            return;
        };
        let Some(key) = herdr::find_key(&text) else {
            self.set_notice("no Jira key found in that text".into());
            return;
        };
        self.new_story = None;
        if self.keys.contains(&key) {
            self.set_notice(format!("{key} is already on the board"));
            return;
        }
        self.run_launch(Launch {
            workspace: key.clone(),
            tab: "break down".into(),
            agent: herdr::agent_name(&[&key, "breakdown"]),
            prompt: format!("/story-break-down {key}"),
        });
    }

    /// The column a pressed card is currently held over, for highlighting the drop target.
    pub fn drop_column(&self) -> Option<usize> {
        self.card_drag.as_ref().and_then(|d| d.over).filter(|c| Some(*c) != self.card_drag.as_ref().map(|d| d.col))
    }

    pub fn poll_ready(&mut self) {
        let mut texts = Vec::new();
        while let Ok(text) = self.launch_done.1.try_recv() {
            texts.push(text);
        }
        if let Some(last) = texts.pop() {
            self.set_notice(last);
        }
        while let Ok(result) = self.ready_done.1.try_recv() {
            match result {
                Ok(id) => self.set_notice(format!("{id} is ready for review")),
                Err(e) => self.set_notice(format!("could not mark ready: {e}")),
            }
        }
    }

    pub fn open_selected_pr(&self) {
        let url = self.pr_rows().get(self.pr_sel).map(|r| r.url.clone());
        if let Some(url) = url {
            (self.opener)(&url);
        }
    }

    fn reset_pr_view(&mut self) {
        self.focus = Focus::Main;
        self.pr_sel = 0;
        self.pr_sheet = false;
        self.run_sel = 0;
        self.run_sheet = false;
    }

    pub fn columns(&self) -> Vec<Column<'_>> {
        let Some(board) = &self.board else {
            return Vec::new();
        };
        let mut statuses = MAIN_COLUMNS.to_vec();
        if board.tasks.iter().any(|t| t.status == Status::Dropped) {
            statuses.push(Status::Dropped);
        }
        statuses
            .into_iter()
            .map(|status| {
                let mut tasks: Vec<&Task> =
                    board.tasks.iter().filter(|t| t.status == status).collect();
                tasks.sort_by_key(|t| id_number(t));
                Column { status, tasks }
            })
            .collect()
    }

    pub fn selected_task(&self) -> Option<&Task> {
        let cols = self.columns();
        let col = cols.get(self.col)?;
        col.tasks.get(self.row[self.col]).copied()
    }

    pub fn move_col(&mut self, delta: i32) {
        let n = self.columns().len() as i32;
        if n > 0 {
            self.col = (self.col as i32 + delta).clamp(0, n - 1) as usize;
        }
    }

    pub fn move_row(&mut self, delta: i32) {
        let len = self.columns().get(self.col).map_or(0, |c| c.tasks.len()) as i32;
        if len > 0 {
            self.row[self.col] = (self.row[self.col] as i32 + delta).clamp(0, len - 1) as usize;
        }
    }

    /// Move to the next or previous visible story while staying on the board.
    pub fn switch_story(&mut self, delta: i32) {
        let visible: Vec<String> = self.visible_stories().iter().map(|s| s.key.clone()).collect();
        let n = visible.len() as i32;
        if n <= 1 {
            return;
        }
        let current = self.keys.get(self.story);
        let next = match current.and_then(|k| visible.iter().position(|v| v == k)) {
            Some(p) => (p as i32 + delta).rem_euclid(n) as usize,
            None if delta > 0 => 0,
            None => (n - 1) as usize,
        };
        if let Some(i) = self.keys.iter().position(|k| *k == visible[next]) {
            self.story = i;
            self.load_current();
            self.focus_first_column();
            self.reset_pr_view();
        }
    }

    /// Returns true when the app should quit.
    pub fn on_key(&mut self, code: KeyCode, ctrl: bool) -> bool {
        if ctrl {
            return code == KeyCode::Char('c');
        }
        if self.new_story.is_some() {
            match code {
                KeyCode::Enter => self.submit_new_story(),
                KeyCode::Esc => self.new_story = None,
                KeyCode::Backspace => {
                    if let Some(b) = &mut self.new_story {
                        b.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(b) = &mut self.new_story {
                        b.push(c);
                    }
                }
                _ => {}
            }
            return false;
        }
        if self.confirm.is_some() {
            match code {
                KeyCode::Char('y') | KeyCode::Enter => self.confirm_ready(),
                KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => self.confirm = None,
                _ => {}
            }
            return false;
        }
        match code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('?') => {
                self.help = !self.help;
                return false;
            }
            KeyCode::Esc if self.detail || self.help || self.pr_sheet || self.run_sheet => {
                self.close_overlays();
                return false;
            }
            KeyCode::Char('>') => {
                self.resize_split(5);
                return false;
            }
            KeyCode::Char('<') => {
                self.resize_split(-5);
                return false;
            }
            KeyCode::Char('+') => {
                self.resize_bottom(5);
                return false;
            }
            KeyCode::Char('-') => {
                self.resize_bottom(-5);
                return false;
            }
            KeyCode::Char('=') => {
                self.reset_layout();
                return false;
            }
            KeyCode::Char('.') => {
                self.resize_column(20);
                return false;
            }
            KeyCode::Char(',') => {
                self.resize_column(-20);
                return false;
            }
            KeyCode::Tab => {
                let next = match self.focus {
                    Focus::Main => Focus::Prs,
                    Focus::Prs => Focus::Runs,
                    Focus::Runs => Focus::Main,
                };
                self.set_focus(next);
                return false;
            }
            KeyCode::BackTab => {
                let next = match self.focus {
                    Focus::Main => Focus::Runs,
                    Focus::Runs => Focus::Prs,
                    Focus::Prs => Focus::Main,
                };
                self.set_focus(next);
                return false;
            }
            KeyCode::Char('o') if self.focus == Focus::Prs => {
                self.open_selected_pr();
                return false;
            }
            KeyCode::Char('c') if self.focus == Focus::Prs => {
                self.copy_pr(self.pr_sel);
                return false;
            }
            KeyCode::Char('m') if self.focus == Focus::Prs => {
                self.ask_mark_ready(self.pr_sel);
                return false;
            }
            KeyCode::Char('o') if self.focus == Focus::Runs => {
                self.open_selected_run();
                return false;
            }
            _ => {}
        }
        if self.focus == Focus::Runs {
            match code {
                KeyCode::Up | KeyCode::Char('k') => self.run_move(-1),
                KeyCode::Down | KeyCode::Char('j') => self.run_move(1),
                KeyCode::Enter | KeyCode::Char(' ') => self.run_sheet = !self.run_sheet,
                KeyCode::Esc => self.focus = Focus::Main,
                KeyCode::Char('r') => self.refresh_prs(),
                _ => {}
            }
            return false;
        }
        if self.focus == Focus::Prs {
            match code {
                KeyCode::Up | KeyCode::Char('k') => self.pr_move(-1),
                KeyCode::Down | KeyCode::Char('j') => self.pr_move(1),
                KeyCode::Enter | KeyCode::Char(' ') => self.pr_sheet = !self.pr_sheet,
                KeyCode::Esc => self.focus = Focus::Main,
                KeyCode::Char('r') => self.refresh_prs(),
                _ => {}
            }
            return false;
        }
        match self.screen {
            Screen::List => self.on_key_list(code),
            Screen::Board => {
                self.on_key_board(code);
                false
            }
        }
    }

    fn on_key_list(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Esc => return true,
            KeyCode::Up | KeyCode::Char('k') => self.list_move(-1),
            KeyCode::Down | KeyCode::Char('j') => self.list_move(1),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => {
                self.open_selected()
            }
            KeyCode::Char('d') => self.toggle_hide_done(),
            KeyCode::Char('n') => self.open_new_story(),
            KeyCode::Char('r') => self.reload_now(),
            _ => {}
        }
        false
    }

    fn on_key_board(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('b') => self.back(),
            KeyCode::Left | KeyCode::Char('h') => self.move_col(-1),
            KeyCode::Right | KeyCode::Char('l') => self.move_col(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_row(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_row(1),
            KeyCode::Char('[') => self.switch_story(-1),
            KeyCode::Char(']') => self.switch_story(1),
            KeyCode::Enter | KeyCode::Char(' ') => self.detail = !self.detail,
            KeyCode::Char('p') => self.start_selected(Status::Planning),
            KeyCode::Char('i') => self.start_selected(Status::Implementing),
            KeyCode::Char('r') => self.reload_now(),
            _ => {}
        }
    }

    fn reload_now(&mut self) {
        self.reload();
        self.refresh_prs();
    }

    fn hit(&self, x: u16, y: u16) -> Option<Target> {
        let hits = self.hits.borrow();
        hits.iter().rev().find(|(rect, _)| contains(*rect, x, y)).map(|(_, t)| *t)
    }

    pub fn on_click(&mut self, x: u16, y: u16) {
        let target = self.hit(x, y);
        if self.new_story.is_some() {
            match target {
                Some(Target::NewStoryStart) => self.submit_new_story(),
                Some(Target::NewStoryCancel) => self.new_story = None,
                Some(Target::Sheet) => {}
                _ => self.new_story = None,
            }
            return;
        }
        if self.confirm.is_some() {
            match target {
                Some(Target::ConfirmYes) => self.confirm_ready(),
                Some(Target::Sheet) => {}
                _ => self.confirm = None,
            }
            return;
        }
        if self.detail || self.help || self.pr_sheet || self.run_sheet {
            match target {
                Some(Target::Sheet) => {}
                Some(Target::OpenPr) => self.open_selected_pr(),
                Some(Target::OpenSelectedRun) => self.open_selected_run(),
                Some(Target::OpenStage(k)) => self.open_sheet_stage(k),
                _ => self.close_overlays(),
            }
            return;
        }
        match target {
            Some(Target::Refresh) => {
                self.refresh_prs();
                return;
            }
            Some(Target::SplitHandle) => {
                self.drag = Some(Drag::Split);
                return;
            }
            Some(Target::HeightHandle) => {
                self.drag = Some(Drag::Height);
                return;
            }
            Some(Target::ColumnHandle(index, x0, x1)) => {
                self.drag = Some(Drag::Column { index, x0, x1 });
                return;
            }
            Some(Target::Pr(i)) => {
                self.set_focus(Focus::Prs);
                self.pr_sel = i;
                self.open_pr(i);
                return;
            }
            Some(Target::PrCopy(i)) => {
                self.set_focus(Focus::Prs);
                self.pr_sel = i;
                self.copy_pr(i);
                return;
            }
            Some(Target::PrBadge(i)) => {
                self.set_focus(Focus::Prs);
                self.pr_sel = i;
                self.ask_mark_ready(i);
                return;
            }
            Some(Target::PrDetails(i)) => {
                self.set_focus(Focus::Prs);
                self.pr_sel = i;
                self.pr_sheet = true;
                return;
            }
            Some(Target::OpenCheck(i, j)) => {
                self.set_focus(Focus::Prs);
                self.pr_sel = i;
                self.open_check(i, j);
                return;
            }
            Some(Target::Run(i)) => {
                self.set_focus(Focus::Runs);
                self.run_sel = i;
                self.open_run(i);
                return;
            }
            Some(Target::RunDetails(i)) => {
                self.set_focus(Focus::Runs);
                self.run_sel = i;
                self.run_sheet = true;
                return;
            }
            Some(Target::OpenJob(i, j)) => {
                self.set_focus(Focus::Runs);
                self.run_sel = i;
                self.open_job(i, j);
                return;
            }
            Some(Target::Tab(tab)) => {
                self.set_focus(if tab == BottomTab::Prs { Focus::Prs } else { Focus::Runs });
                return;
            }
            Some(Target::PrPanel) => {
                self.set_focus(Focus::Prs);
                return;
            }
            Some(Target::RunPanel) => {
                self.set_focus(Focus::Runs);
                return;
            }
            _ => {}
        }
        if target.is_some() {
            self.focus = Focus::Main;
        }
        match target {
            Some(Target::Story(i)) => {
                self.list_sel = i;
                self.open_selected();
            }
            Some(Target::ToggleDone) => self.toggle_hide_done(),
            Some(Target::Back) => self.back(),
            Some(Target::Column(c)) => self.col = c,
            Some(Target::Card { col, row }) => {
                let open = self.col == col && self.row[col] == row;
                self.col = col;
                self.row[col] = row;
                self.card_drag = Some(CardDrag { col, row, open, over: None });
            }
            Some(Target::NewStory) => self.open_new_story(),
            Some(Target::PrevColumn) => self.move_col(-1),
            Some(Target::NextColumn) => self.move_col(1),
            _ => {}
        }
    }

    pub fn on_scroll(&mut self, x: u16, y: u16, delta: i32) {
        if self.detail || self.help || self.pr_sheet || self.run_sheet {
            return;
        }
        let target = self.hit(x, y);
        if matches!(target, Some(Target::Pr(_) | Target::PrDetails(_) | Target::PrPanel | Target::OpenCheck(..))) {
            self.set_focus(Focus::Prs);
            self.pr_move(delta);
            return;
        }
        if matches!(target, Some(Target::Run(_) | Target::RunPanel | Target::RunDetails(_) | Target::OpenJob(..))) {
            self.set_focus(Focus::Runs);
            self.run_move(delta);
            return;
        }
        match self.screen {
            Screen::List => self.list_move(delta),
            Screen::Board => {
                if let Some(Target::Column(c) | Target::Card { col: c, .. }) = target {
                    self.col = c;
                }
                self.move_row(delta);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyCode;
    use crate::prs::{Check, CheckState, PullRequest, Review};
    use crate::runs::{Batch, Job, Run, RunState};
    use capcom::model::{PrState, StoryStatus};
    use capcom::model::TaskType::{self, Pr, Spike};
    use capcom::{ops, rules, store};
    use tempfile::TempDir;

    fn story(root: &TempDir, key: &str, tasks: &[(&str, TaskType, &[&str])]) {
        ops::init_story(root.path(), key, &format!("Story {key}"), None).unwrap();
        store::update(root.path(), key, |b| {
            let dir = root.path().join(key);
            for (title, kind, deps) in tasks {
                let deps = deps.iter().map(|d| d.to_string()).collect();
                ops::add_task(&dir, b, title, *kind, deps, vec![])?;
            }
            Ok(())
        })
        .unwrap();
    }

    fn set_status(root: &TempDir, key: &str, id: &str, to: Status) {
        store::update(root.path(), key, |b| rules::transition(b, id, to)).unwrap();
    }

    fn ids(app: &App, status: Status) -> Vec<String> {
        let cols = app.columns();
        let col = cols.iter().find(|c| c.status == status).unwrap();
        col.tasks.iter().map(|t| t.id.clone()).collect()
    }

    fn new(root: &TempDir) -> App {
        App::new(root.path().to_path_buf(), Some("PROJ-1".into()))
    }

    fn finish_story(root: &TempDir, key: &str) {
        store::update(root.path(), key, |b| {
            let dir = root.path().join(key);
            ops::add_task(&dir, b, "Done thing", Spike, vec![], vec![])?;
            let id = b.tasks.last().unwrap().id.clone();
            for s in [Status::Planning, Status::Planned, Status::Implementing, Status::Done] {
                rules::transition(b, &id, s)?;
            }
            rules::story_transition(b, StoryStatus::InReview)?;
            rules::story_transition(b, StoryStatus::Done)
        })
        .unwrap();
    }

    fn live(repo: &str, number: u64, title: &str) -> PullRequest {
        PullRequest {
            repo: repo.into(),
            number,
            title: title.into(),
            url: format!("https://github.com/{repo}/pull/{number}"),
            is_draft: false,
            labels: vec![],
            review: Review::None,
            comments: 0,
            updated_at: "2026-10-08T01:00:00Z".into(),
            checks: vec![Check {
                name: "build".into(),
                workflow: Some("CI".into()),
                state: CheckState::Running,
                started_at: None,
                completed_at: None,
                url: None,
            }],
        }
    }

    fn record_pr(root: &TempDir, key: &str, task: &str, repo: &str, url: &str, state: PrState) {
        store::update(root.path(), key, |b| ops::add_pr(b, task, repo, url, state)).unwrap();
    }

    const URL12: &str = "https://github.com/acme/widgets/pull/12";
    const URL5: &str = "https://github.com/acme/widgets/pull/5";

    fn run(repo: &str, id: u64, state: RunState) -> Run {
        Run {
            repo: repo.into(),
            id,
            name: "Deploy nonprod".into(),
            title: "Deploy".into(),
            branch: "main".into(),
            url: format!("https://github.com/{repo}/actions/runs/{id}"),
            state,
            created_at: "2026-10-08T03:00:00Z".into(),
            started_at: None,
            updated_at: "2026-10-08T03:01:00Z".into(),
            jobs: vec![Job {
                name: "deploy".into(),
                state: RunState::Running,
                url: format!("https://github.com/{repo}/actions/runs/{id}/job/1"),
                started_at: None,
                completed_at: None,
            }],
        }
    }

    fn batch(runs: Vec<Run>) -> Result<Batch, String> {
        Ok(Batch { runs, warnings: vec![] })
    }

    fn recorder(app: &mut App) -> std::rc::Rc<std::cell::RefCell<Vec<String>>> {
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let sink = log.clone();
        app.opener = Box::new(move |url| sink.borrow_mut().push(url.to_string()));
        log
    }

    fn open_key(app: &App) -> String {
        app.board.as_ref().unwrap().story.key.clone()
    }

    #[test]
    fn columns_group_tasks_by_status_and_sort_ids_numerically() {
        let root = TempDir::new().unwrap();
        let titles: Vec<String> = (1..=10).map(|n| format!("Task {n}")).collect();
        let tasks: Vec<(&str, TaskType, &[&str])> =
            titles.iter().map(|t| (t.as_str(), Pr, &[][..])).collect();
        story(&root, "PROJ-1", &tasks);
        set_status(&root, "PROJ-1", "T3", Status::Planning);
        let app = new(&root);
        assert_eq!(app.columns().len(), 5);
        let todo: Vec<String> = [1, 2, 4, 5, 6, 7, 8, 9, 10].iter().map(|n| format!("T{n}")).collect();
        assert_eq!(ids(&app, Status::Todo), todo);
        assert_eq!(ids(&app, Status::Planning), vec!["T3"]);
        assert!(ids(&app, Status::Done).is_empty());
    }

    #[test]
    fn a_dropped_column_appears_only_when_a_task_is_dropped() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[]), ("Two", Pr, &[])]);
        assert_eq!(new(&root).columns().len(), 5);
        set_status(&root, "PROJ-1", "T2", Status::Dropped);
        let app = new(&root);
        let cols = app.columns();
        assert_eq!(cols.len(), 6);
        assert_eq!(cols[5].status, Status::Dropped);
        assert_eq!(ids(&app, Status::Dropped), vec!["T2"]);
    }

    #[test]
    fn selection_moves_within_bounds_and_remembers_the_row_per_column() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[]), ("Two", Pr, &[]), ("Three", Pr, &[])]);
        set_status(&root, "PROJ-1", "T3", Status::Planning);
        let mut app = new(&root);
        assert_eq!(app.selected_task().unwrap().id, "T1");
        app.move_row(1);
        assert_eq!(app.selected_task().unwrap().id, "T2");
        app.move_row(5);
        assert_eq!(app.selected_task().unwrap().id, "T2");
        app.move_col(1);
        assert_eq!(app.selected_task().unwrap().id, "T3");
        app.move_col(-1);
        assert_eq!(app.selected_task().unwrap().id, "T2");
        app.move_row(-9);
        assert_eq!(app.selected_task().unwrap().id, "T1");
        app.move_col(-5);
        assert_eq!(app.col, 0);
        app.move_col(9);
        assert_eq!(app.col, 4);
        assert!(app.selected_task().is_none());
    }

    #[test]
    fn it_starts_on_the_first_column_that_has_tasks() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[])]);
        set_status(&root, "PROJ-1", "T1", Status::Planning);
        assert_eq!(new(&root).col, 1);
    }

    #[test]
    fn reload_picks_up_changes_and_keeps_the_selected_task() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[]), ("Two", Pr, &[])]);
        let mut app = new(&root);
        app.move_row(1);
        assert!(!app.needs_reload());
        set_status(&root, "PROJ-1", "T2", Status::Planning);
        assert!(app.needs_reload());
        app.reload();
        assert!(!app.needs_reload());
        assert_eq!(app.selected_task().unwrap().id, "T2");
        assert_eq!(app.col, 1);
        assert_eq!(ids(&app, Status::Todo), vec!["T1"]);
    }

    #[test]
    fn reload_notices_a_new_story_folder() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[])]);
        let mut app = new(&root);
        story(&root, "PROJ-2", &[("Two", Pr, &[])]);
        assert!(app.needs_reload());
        app.reload();
        assert_eq!(app.keys, vec!["PROJ-1", "PROJ-2"]);
        assert_eq!(app.board.as_ref().unwrap().story.key, "PROJ-1");
    }

    #[test]
    fn a_broken_board_keeps_the_last_good_view_and_reports_the_error() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[])]);
        let path = root.path().join("PROJ-1/board.json");
        let good = std::fs::read_to_string(&path).unwrap();
        let mut app = new(&root);
        std::fs::write(&path, "{ not json").unwrap();
        assert!(app.needs_reload());
        app.reload();
        assert_eq!(app.board.as_ref().unwrap().tasks.len(), 1);
        assert!(app.error.as_deref().unwrap().contains("board.json"));
        std::fs::write(&path, good).unwrap();
        app.reload();
        assert!(app.error.is_none());
    }

    #[test]
    fn switching_story_cycles_and_a_requested_key_wins() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[])]);
        story(&root, "PROJ-2", &[("Two", Spike, &[])]);
        let mut app = new(&root);
        assert_eq!(app.board.as_ref().unwrap().story.key, "PROJ-1");
        app.switch_story(1);
        assert_eq!(app.board.as_ref().unwrap().story.key, "PROJ-2");
        app.switch_story(1);
        assert_eq!(app.board.as_ref().unwrap().story.key, "PROJ-1");
        app.switch_story(-1);
        assert_eq!(app.board.as_ref().unwrap().story.key, "PROJ-2");

        let wanted = App::new(root.path().to_path_buf(), Some("PROJ-2".into()));
        assert_eq!(wanted.board.as_ref().unwrap().story.key, "PROJ-2");
        let unknown = App::new(root.path().to_path_buf(), Some("PROJ-9".into()));
        assert_eq!(unknown.board.as_ref().unwrap().story.key, "PROJ-1");
    }

    #[test]
    fn an_empty_root_has_no_board_and_no_columns() {
        let root = TempDir::new().unwrap();
        let mut app = new(&root);
        assert!(app.board.is_none());
        assert!(app.columns().is_empty());
        assert!(app.selected_task().is_none());
        app.move_col(1);
        app.move_row(1);
        app.switch_story(1);
        app.reload();
    }

    #[test]
    fn keys_navigate_toggle_overlays_and_quit() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[]), ("Two", Pr, &[])]);
        let mut app = new(&root);
        assert!(!app.on_key(KeyCode::Char('j'), false));
        assert_eq!(app.selected_task().unwrap().id, "T2");
        assert!(!app.on_key(KeyCode::Up, false));
        assert_eq!(app.selected_task().unwrap().id, "T1");
        assert!(!app.on_key(KeyCode::Char('l'), false));
        assert_eq!(app.col, 1);
        assert!(!app.on_key(KeyCode::Left, false));
        assert!(!app.on_key(KeyCode::Enter, false));
        assert!(app.detail);
        assert!(!app.on_key(KeyCode::Esc, false));
        assert!(!app.detail);
        assert!(!app.on_key(KeyCode::Char('?'), false));
        assert!(app.help);
        assert!(!app.on_key(KeyCode::Esc, false));
        assert!(!app.help);
        assert_eq!(app.screen, Screen::Board);
        assert!(!app.on_key(KeyCode::Esc, false));
        assert_eq!(app.screen, Screen::List);
        assert!(app.on_key(KeyCode::Esc, false));
        assert!(app.on_key(KeyCode::Char('q'), false));
        assert!(app.on_key(KeyCode::Char('c'), true));
    }
    #[test]
    fn the_list_summarises_every_story() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[]), ("Two", Pr, &[])]);
        set_status(&root, "PROJ-1", "T2", Status::Planning);
        story(&root, "PROJ-2", &[("Three", Pr, &[])]);
        let app = App::new(root.path().to_path_buf(), None);
        assert_eq!(app.screen, Screen::List);
        assert_eq!(app.stories.len(), 2);
        let s = &app.stories[0];
        assert_eq!((s.key.as_str(), s.title.as_str()), ("PROJ-1", "Story PROJ-1"));
        assert_eq!((s.total, s.done), (2, 0));
        assert_eq!((s.counts[0], s.counts[1]), (1, 1));
        assert_eq!(s.status, StoryStatus::InProgress);
        assert!(s.error.is_none());
    }

    #[test]
    fn completed_stories_are_hidden_by_default_and_can_be_shown() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[]);
        finish_story(&root, "PROJ-1");
        story(&root, "PROJ-2", &[("Open", Pr, &[])]);
        let mut app = App::new(root.path().to_path_buf(), None);
        assert!(app.hide_done);
        let keys = |app: &App| app.visible_stories().iter().map(|s| s.key.clone()).collect::<Vec<_>>();
        assert_eq!(keys(&app), vec!["PROJ-2"]);
        assert_eq!(app.hidden_done_count(), 1);
        app.toggle_hide_done();
        assert_eq!(keys(&app), vec!["PROJ-1", "PROJ-2"]);
        assert_eq!(app.hidden_done_count(), 0);
        app.toggle_hide_done();
        assert_eq!(keys(&app), vec!["PROJ-2"]);
    }

    #[test]
    fn a_story_that_fails_to_load_is_always_listed_with_its_error() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[])]);
        std::fs::write(root.path().join("PROJ-1/board.json"), "{ nope").unwrap();
        let app = App::new(root.path().to_path_buf(), None);
        let visible = app.visible_stories();
        assert_eq!(visible.len(), 1);
        assert!(visible[0].error.as_deref().unwrap().contains("board.json"));
    }

    #[test]
    fn the_list_selects_and_opens_stories_and_esc_goes_back() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[]);
        finish_story(&root, "PROJ-1");
        story(&root, "PROJ-2", &[("Two", Pr, &[])]);
        story(&root, "PROJ-3", &[("Three", Pr, &[])]);
        let mut app = App::new(root.path().to_path_buf(), None);
        assert_eq!(app.list_sel, 0);
        app.on_key(KeyCode::Char('j'), false);
        assert_eq!(app.list_sel, 1);
        app.on_key(KeyCode::Down, false);
        assert_eq!(app.list_sel, 1);
        app.on_key(KeyCode::Char('k'), false);
        app.on_key(KeyCode::Enter, false);
        assert_eq!(app.screen, Screen::Board);
        assert_eq!(open_key(&app), "PROJ-2");
        app.on_key(KeyCode::Esc, false);
        assert_eq!(app.screen, Screen::List);
        app.on_key(KeyCode::Char('j'), false);
        app.on_key(KeyCode::Char('l'), false);
        assert_eq!(open_key(&app), "PROJ-3");
        app.on_key(KeyCode::Backspace, false);
        assert_eq!(app.screen, Screen::List);
        app.on_key(KeyCode::Char('d'), false);
        assert!(!app.hide_done);
        assert_eq!(app.visible_stories().len(), 3);
    }

    #[test]
    fn starting_with_a_story_key_opens_that_board() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[])]);
        story(&root, "PROJ-2", &[("Two", Pr, &[])]);
        let app = App::new(root.path().to_path_buf(), Some("PROJ-2".into()));
        assert_eq!(app.screen, Screen::Board);
        assert_eq!(open_key(&app), "PROJ-2");
    }

    #[test]
    fn the_list_selection_survives_a_reload() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[])]);
        story(&root, "PROJ-2", &[("Two", Pr, &[])]);
        let mut app = App::new(root.path().to_path_buf(), None);
        app.on_key(KeyCode::Char('j'), false);
        set_status(&root, "PROJ-2", "T1", Status::Planning);
        app.reload();
        assert_eq!(app.visible_stories()[app.list_sel].key, "PROJ-2");
        assert_eq!(app.stories[1].counts[1], 1);
    }

    #[test]
    fn switching_story_on_the_board_skips_hidden_done_stories() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[]);
        finish_story(&root, "PROJ-1");
        story(&root, "PROJ-2", &[("Two", Pr, &[])]);
        story(&root, "PROJ-3", &[("Three", Pr, &[])]);
        let mut app = App::new(root.path().to_path_buf(), Some("PROJ-2".into()));
        assert_eq!(app.story_position(), Some((1, 2)));
        app.switch_story(1);
        assert_eq!(open_key(&app), "PROJ-3");
        app.switch_story(1);
        assert_eq!(open_key(&app), "PROJ-2");
        app.hide_done = false;
        app.switch_story(-1);
        assert_eq!(open_key(&app), "PROJ-1");
    }
    #[test]
    fn story_summaries_remember_which_task_owns_each_pr_url() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[])]);
        record_pr(&root, "PROJ-1", "T1", "widgets", URL12, PrState::Draft);
        let app = App::new(root.path().to_path_buf(), None);
        assert_eq!(app.pr_tag(URL12).as_deref(), Some("PROJ-1 · T1"));
        assert_eq!(app.pr_tag(&format!("{URL12}/")).as_deref(), Some("PROJ-1 · T1"));
        assert_eq!(app.pr_tag("https://github.com/acme/widgets/pull/99"), None);
    }

    #[test]
    fn the_main_panel_lists_every_open_pr_and_tags_the_ones_on_a_story() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[])]);
        record_pr(&root, "PROJ-1", "T1", "widgets", URL12, PrState::Draft);
        let mut app = App::new(root.path().to_path_buf(), None);
        app.apply_prs(Ok(vec![live("acme/widgets", 12, "Add notices"), live("acme/api", 99, "Unrelated")]));
        let rows = app.pr_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].tag.as_deref(), Some("PROJ-1 · T1"));
        assert!(rows[0].live.is_some());
        assert_eq!((rows[1].tag.clone(), rows[1].number), (None, Some(99)));
    }

    #[test]
    fn the_story_panel_lists_only_that_storys_prs_and_falls_back_to_board_data() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[]), ("Two", Pr, &[])]);
        story(&root, "PROJ-2", &[("Three", Pr, &[])]);
        record_pr(&root, "PROJ-1", "T1", "widgets", URL12, PrState::Draft);
        record_pr(&root, "PROJ-1", "T2", "widgets", URL5, PrState::Merged);
        record_pr(&root, "PROJ-2", "T1", "api", "https://github.com/acme/api/pull/99", PrState::Draft);
        let mut app = App::new(root.path().to_path_buf(), Some("PROJ-1".into()));
        app.apply_prs(Ok(vec![live("acme/widgets", 12, "Add notices"), live("acme/api", 99, "Other story")]));
        let rows = app.pr_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].tag.as_deref(), rows[0].live.is_some()), (Some("T1"), true));
        assert_eq!(rows[0].title, "Add notices");
        assert_eq!((rows[1].tag.as_deref(), rows[1].live.is_none()), (Some("T2"), true));
        assert_eq!((rows[1].repo.as_str(), rows[1].number), ("acme/widgets", Some(5)));
        assert_eq!(rows[1].board_state, Some(PrState::Merged));
    }

    #[test]
    fn tab_switches_focus_and_arrows_move_the_pr_selection() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[])]);
        story(&root, "PROJ-2", &[("Two", Pr, &[])]);
        let mut app = App::new(root.path().to_path_buf(), None);
        app.apply_prs(Ok(vec![live("a/b", 1, "x"), live("a/b", 2, "y"), live("a/b", 3, "z")]));
        app.on_key(KeyCode::Tab, false);
        assert_eq!(app.focus, Focus::Prs);
        app.on_key(KeyCode::Down, false);
        app.on_key(KeyCode::Char('j'), false);
        assert_eq!((app.pr_sel, app.list_sel), (2, 0));
        app.on_key(KeyCode::Down, false);
        assert_eq!(app.pr_sel, 2);
        app.on_key(KeyCode::Up, false);
        assert_eq!(app.pr_sel, 1);
        assert!(!app.on_key(KeyCode::Esc, false));
        assert_eq!(app.focus, Focus::Main);
        app.on_key(KeyCode::Down, false);
        assert_eq!((app.pr_sel, app.list_sel), (1, 1));
        app.on_key(KeyCode::Tab, false);
        app.on_key(KeyCode::Tab, false);
        app.on_key(KeyCode::Tab, false);
        assert_eq!(app.focus, Focus::Main, "main, PRs, runs, then back to main");
    }

    #[test]
    fn base64_matches_the_standard_encoding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
    }

    #[test]
    fn enter_opens_the_pr_sheet_and_o_opens_the_pr_in_the_browser() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[])]);
        let mut app = App::new(root.path().to_path_buf(), None);
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let sink = log.clone();
        app.opener = Box::new(move |url| sink.borrow_mut().push(url.to_string()));
        app.apply_prs(Ok(vec![live("acme/widgets", 12, "Add notices")]));
        app.on_key(KeyCode::Char('o'), false);
        assert!(log.borrow().is_empty(), "o only acts on the PR panel");
        app.on_key(KeyCode::Tab, false);
        app.on_key(KeyCode::Enter, false);
        assert!(app.pr_sheet);
        app.on_key(KeyCode::Char('o'), false);
        assert_eq!(*log.borrow(), vec![URL12.to_string()]);
        assert!(!app.on_key(KeyCode::Esc, false));
        assert!(!app.pr_sheet);
        assert_eq!(app.focus, Focus::Prs);
        assert!(!app.on_key(KeyCode::Esc, false));
        assert_eq!(app.focus, Focus::Main);
    }

    #[test]
    fn a_feed_error_keeps_the_last_prs_and_the_next_result_clears_it() {
        let root = TempDir::new().unwrap();
        let mut app = App::new(root.path().to_path_buf(), None);
        assert!(!app.prs.loaded);
        app.apply_started();
        assert!(app.prs.loading);
        app.apply_prs(Ok(vec![live("a/b", 1, "x"), live("a/b", 2, "y")]));
        assert!(app.prs.loaded && !app.prs.loading && app.prs.updated.is_some());
        app.apply_prs(Err("rate limited".into()));
        assert_eq!(app.prs.items.len(), 2);
        assert_eq!(app.prs.error.as_deref(), Some("rate limited"));
        app.apply_prs(Ok(vec![live("a/b", 1, "x")]));
        assert!(app.prs.error.is_none());
        assert_eq!(app.prs.items.len(), 1);
    }

    #[test]
    fn the_pr_selection_is_clamped_when_the_list_shrinks() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[])]);
        let mut app = App::new(root.path().to_path_buf(), None);
        app.apply_prs(Ok(vec![live("a/b", 1, "x"), live("a/b", 2, "y"), live("a/b", 3, "z")]));
        app.focus = Focus::Prs;
        app.pr_move(5);
        assert_eq!(app.pr_sel, 2);
        app.apply_prs(Ok(vec![live("a/b", 1, "x")]));
        assert_eq!(app.pr_sel, 0);
    }
    #[test]
    fn the_repos_to_check_combine_pr_repos_story_prs_and_the_extra_list() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[])]);
        record_pr(&root, "PROJ-1", "T1", "widgets", URL5, PrState::Merged);
        let mut app = App::new(root.path().to_path_buf(), None);
        app.apply_prs(Ok(vec![live("acme/api", 99, "x")]));
        app.extra_repos = vec!["acme/ops".into(), "bad repo".into(), "acme/api".into()];
        assert_eq!(app.deploy_repos(), vec!["acme/api", "acme/ops", "acme/widgets"]);
    }

    #[test]
    fn the_shared_repo_list_follows_the_app_and_wakes_the_poller() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[])]);
        record_pr(&root, "PROJ-1", "T1", "widgets", URL5, PrState::Merged);
        let mut app = App::new(root.path().to_path_buf(), None);
        let (_tx, rx) = std::sync::mpsc::channel();
        let (wake_tx, wake_rx) = std::sync::mpsc::channel();
        let shared = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        app.attach_runs(rx, wake_tx, shared.clone());
        assert_eq!(*shared.lock().unwrap(), vec!["acme/widgets"]);
        while wake_rx.try_recv().is_ok() {}
        app.apply_prs(Ok(vec![live("acme/new", 1, "x")]));
        assert_eq!(*shared.lock().unwrap(), vec!["acme/new", "acme/widgets"]);
        assert!(wake_rx.try_recv().is_ok(), "a changed repo list refreshes the runs at once");
        app.apply_prs(Ok(vec![live("acme/new", 1, "x")]));
        assert!(wake_rx.try_recv().is_err(), "no change, no wake");
    }

    #[test]
    fn run_data_tracks_loading_results_warnings_and_errors() {
        let root = TempDir::new().unwrap();
        let mut app = App::new(root.path().to_path_buf(), None);
        assert!(!app.runs.loaded);
        app.apply_run_started();
        assert!(app.runs.loading);
        let mut ok = Batch { runs: vec![run("acme/widgets", 1, RunState::Running)], warnings: vec!["acme/api: HTTP 403".into()] };
        app.apply_runs(Ok(ok.clone()));
        assert!(app.runs.loaded && !app.runs.loading && app.runs.updated.is_some());
        assert_eq!(app.runs.warnings, vec!["acme/api: HTTP 403"]);
        app.apply_runs(Err("rate limited".into()));
        assert_eq!(app.runs.items.len(), 1);
        assert_eq!(app.runs.error.as_deref(), Some("rate limited"));
        ok.warnings.clear();
        app.apply_runs(Ok(ok));
        assert!(app.runs.error.is_none() && app.runs.warnings.is_empty());
    }

    #[test]
    fn the_story_view_only_shows_runs_in_the_storys_repos() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[])]);
        record_pr(&root, "PROJ-1", "T1", "widgets", URL12, PrState::Draft);
        let mut app = App::new(root.path().to_path_buf(), None);
        app.apply_runs(batch(vec![run("acme/widgets", 1, RunState::Running), run("acme/api", 2, RunState::Success)]));
        assert_eq!(app.visible_runs().len(), 2, "the main screen shows every run");
        app.open_story("PROJ-1");
        let runs = app.visible_runs();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].repo, "acme/widgets");
    }

    #[test]
    fn tab_cycles_main_prs_runs_and_arrows_move_the_run_selection() {
        let root = TempDir::new().unwrap();
        let mut app = App::new(root.path().to_path_buf(), None);
        app.apply_runs(batch(vec![run("a/b", 1, RunState::Running), run("a/b", 2, RunState::Success), run("a/b", 3, RunState::Failed)]));
        app.on_key(KeyCode::Tab, false);
        assert_eq!((app.focus, app.tab), (Focus::Prs, BottomTab::Prs));
        app.on_key(KeyCode::Tab, false);
        assert_eq!((app.focus, app.tab), (Focus::Runs, BottomTab::Runs));
        app.on_key(KeyCode::Down, false);
        app.on_key(KeyCode::Char('j'), false);
        app.on_key(KeyCode::Down, false);
        assert_eq!(app.run_sel, 2);
        app.on_key(KeyCode::Up, false);
        assert_eq!(app.run_sel, 1);
        app.on_key(KeyCode::Tab, false);
        assert_eq!(app.focus, Focus::Main);
        assert_eq!(app.list_sel, 0);
    }

    #[test]
    fn enter_opens_the_run_sheet_and_o_opens_the_run_in_the_browser() {
        let root = TempDir::new().unwrap();
        let mut app = App::new(root.path().to_path_buf(), None);
        let log = recorder(&mut app);
        app.apply_runs(batch(vec![run("acme/widgets", 7, RunState::Running)]));
        app.focus = Focus::Runs;
        app.on_key(KeyCode::Enter, false);
        assert!(app.run_sheet);
        app.on_key(KeyCode::Char('o'), false);
        assert_eq!(*log.borrow(), vec!["https://github.com/acme/widgets/actions/runs/7".to_string()]);
        assert!(!app.on_key(KeyCode::Esc, false));
        assert!(!app.run_sheet);
        assert_eq!(app.focus, Focus::Runs);
        assert!(!app.on_key(KeyCode::Esc, false));
        assert_eq!(app.focus, Focus::Main);
    }

    #[test]
    fn opening_a_run_or_a_stage_uses_their_own_links_and_the_selection_is_clamped() {
        let root = TempDir::new().unwrap();
        let mut app = App::new(root.path().to_path_buf(), None);
        let log = recorder(&mut app);
        app.apply_runs(batch(vec![run("a/b", 1, RunState::Running), run("a/b", 2, RunState::Running)]));
        app.open_run(1);
        app.open_job(0, 0);
        assert_eq!(
            *log.borrow(),
            vec!["https://github.com/a/b/actions/runs/2".to_string(), "https://github.com/a/b/actions/runs/1/job/1".to_string()]
        );
        app.focus = Focus::Runs;
        app.run_move(5);
        assert_eq!(app.run_sel, 1);
        app.apply_runs(batch(vec![run("a/b", 1, RunState::Running)]));
        assert_eq!(app.run_sel, 0);
    }
    fn temp_settings() -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("tui.json");
        (dir, path)
    }

    #[test]
    fn layout_changes_are_saved_and_restored_in_the_next_session() {
        let root = TempDir::new().unwrap();
        let (_dir, path) = temp_settings();
        let mut app = App::new(root.path().to_path_buf(), None);
        app.settings_path = Some(path.clone());
        app.on_key(KeyCode::Char('>'), false);
        app.on_key(KeyCode::Char('+'), false);
        let mut next = App::new(root.path().to_path_buf(), None);
        next.settings_path = Some(path.clone());
        next.load_settings();
        assert_eq!((next.split_pct, next.bottom_pct, next.split_pinned), (63, Some(50), true));
        next.on_key(KeyCode::Char('='), false);
        let mut third = App::new(root.path().to_path_buf(), None);
        third.settings_path = Some(path);
        third.load_settings();
        assert_eq!((third.split_pct, third.bottom_pct, third.split_pinned), (58, None, false), "reset is saved too");
    }

    #[test]
    fn nothing_is_written_without_a_settings_path_and_a_drag_saves_on_release() {
        let root = TempDir::new().unwrap();
        let mut app = App::new(root.path().to_path_buf(), None);
        app.on_key(KeyCode::Char('>'), false);
        let (_dir, path) = temp_settings();
        app.settings_path = Some(path.clone());
        app.geometry.set(Geometry { body_y: 1, body_h: 40, bottom_x: 0, bottom_w: 200 });
        app.hits.borrow_mut().push((Rect::new(100, 20, 2, 5), Target::SplitHandle));
        app.on_click(100, 21);
        app.on_drag(140, 21);
        assert!(!path.exists(), "nothing is saved while dragging");
        app.on_release();
        assert!(path.exists(), "the release saves");
        assert_eq!(crate::settings::load(&path).split_pct, 70);
    }

    #[test]
    fn column_keys_resize_the_selected_column_at_its_neighbours_expense() {
        let root = TempDir::new().unwrap();
        story(&root, "PROJ-1", &[("One", Pr, &[])]);
        let mut app = new(&root);
        assert_eq!(app.col_weights[..2], [100, 100]);
        app.on_key(KeyCode::Char('.'), false);
        assert_eq!(app.col_weights[..2], [120, 80]);
        app.on_key(KeyCode::Char(','), false);
        app.on_key(KeyCode::Char(','), false);
        assert_eq!(app.col_weights[..2], [80, 120]);
        for _ in 0..20 {
            app.on_key(KeyCode::Char(','), false);
        }
        assert_eq!(app.col_weights[..2], [20, 180], "no column goes below the minimum");
        app.col = 4;
        app.on_key(KeyCode::Char('.'), false);
        assert_eq!(app.col_weights[3..5], [80, 120], "the last column takes from the one before it");
        app.on_key(KeyCode::Char('='), false);
        assert_eq!(app.col_weights, [100; 6]);
    }
}
