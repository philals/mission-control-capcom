//! One fixed dark palette in 1960s space-programme colours (console charcoal, cream paper, NASA
//! blue, amber lamps, phosphor green), so the TUI looks the same whatever the terminal's own background or
//! colour scheme is (for example a background that changes per folder). Every text colour has
//! at least a 4.5:1 contrast ratio against `BG`, and against `SELECT` or `HEADER` where text sits on them.
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Clear};
use ratatui::Frame;

pub const BG: Color = Color::Rgb(0x14, 0x18, 0x1a);
/// The title and key strips, like the blue of a Mercury-era console.
pub const HEADER: Color = Color::Rgb(0x0d, 0x2c, 0x5c);
pub const SELECT: Color = Color::Rgb(0x2b, 0x3b, 0x34);
pub const FG: Color = Color::Rgb(0xf3, 0xeb, 0xd6);
pub const MUTED: Color = Color::Rgb(0xd9, 0xd0, 0xb8);
pub const DIM: Color = Color::Rgb(0xb8, 0xae, 0x94);
pub const BORDER: Color = Color::Rgb(0x80, 0x89, 0x7a);
pub const CYAN: Color = Color::Rgb(0x8c, 0xc2, 0xff);
pub const BLUE: Color = Color::Rgb(0x79, 0xae, 0xff);
pub const GREEN: Color = Color::Rgb(0x84, 0xf0, 0xa0);
pub const RED: Color = Color::Rgb(0xff, 0x80, 0x6c);
pub const YELLOW: Color = Color::Rgb(0xff, 0xd2, 0x4a);
pub const MAGENTA: Color = Color::Rgb(0xd4, 0xb8, 0xf4);
pub const ORANGE: Color = Color::Rgb(0xff, 0x9a, 0x3c);

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
