//! One fixed dark palette, so the TUI looks the same whatever the terminal's own background or
//! colour scheme is (for example a background that changes per folder). Every text colour has
//! at least a 4.5:1 contrast ratio against `BG`, and against `SELECT` where text sits on it.
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Clear};
use ratatui::Frame;

pub const BG: Color = Color::Rgb(0x10, 0x15, 0x1c);
pub const SELECT: Color = Color::Rgb(0x25, 0x32, 0x4a);
pub const FG: Color = Color::Rgb(0xe8, 0xeb, 0xf1);
pub const MUTED: Color = Color::Rgb(0xc3, 0xca, 0xd8);
pub const DIM: Color = Color::Rgb(0xa3, 0xaf, 0xc2);
pub const BORDER: Color = Color::Rgb(0x66, 0x73, 0x88);
pub const CYAN: Color = Color::Rgb(0x62, 0xd6, 0xff);
pub const BLUE: Color = Color::Rgb(0x7f, 0xa6, 0xff);
pub const GREEN: Color = Color::Rgb(0x6e, 0xe7, 0x87);
pub const RED: Color = Color::Rgb(0xff, 0x7b, 0x72);
pub const YELLOW: Color = Color::Rgb(0xf2, 0xcc, 0x60);
pub const MAGENTA: Color = Color::Rgb(0xd9, 0xb0, 0xff);
pub const ORANGE: Color = Color::Rgb(0xff, 0xa2, 0x4d);

pub fn base() -> Style {
    Style::new().fg(FG).bg(BG)
}

/// Paint the whole frame, so no cell shows the terminal's own background.
pub fn paint(f: &mut Frame) {
    f.render_widget(Block::new().style(base()), f.area());
}

/// Blank a rectangle for a sheet or dialog, in the theme's colours rather than the terminal's.
pub fn clear(f: &mut Frame, area: Rect) {
    f.render_widget(Clear, area);
    f.render_widget(Block::new().style(base()), area);
}
