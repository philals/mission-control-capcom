//! One fixed dark palette in 1960s space-programme colours (console charcoal, cream paper, NASA
//! blue, amber lamps, phosphor green), so the TUI looks the same whatever the terminal's own background or
//! colour scheme is (for example a background that changes per folder). Every text colour has
//! at least a 4.5:1 contrast ratio against `BG`, and against `SELECT` or `HEADER` where text sits on them.
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::style::Modifier;
use ratatui::widgets::{Block, BorderType, Clear};
use ratatui::Frame;

pub const BG: Color = Color::Rgb(0x14, 0x18, 0x1a);
/// The title and key strips, like the blue of a Mercury-era console.
pub const HEADER: Color = Color::Rgb(0x0d, 0x2c, 0x5c);
/// The selected row: a slate blue, so green on a row means one thing only (a PR that is green and waiting for a reviewer).
pub const SELECT: Color = Color::Rgb(0x26, 0x37, 0x4f);
/// The row under the pointer: lighter than the background, quieter than a selection.
pub const HOVER: Color = Color::Rgb(0x1f, 0x29, 0x2d);
/// Row tint for a PR that is green and waiting for a reviewer: a dark console green.
pub const GO_ROW: Color = Color::Rgb(0x10, 0x3a, 0x22);
/// The kanban board's floor: a shade lighter than the console `BG` around it, so the work area reads as its own surface.
pub const BOARD_BG: Color = Color::Rgb(0x1f, 0x2a, 0x31);
pub const FG: Color = Color::Rgb(0xf3, 0xeb, 0xd6);
pub const MUTED: Color = Color::Rgb(0xd9, 0xd0, 0xb8);
pub const DIM: Color = Color::Rgb(0xb8, 0xae, 0x94);
/// The steel of a console panel's frame.
pub const FRAME: Color = Color::Rgb(0xa9, 0xb6, 0xad);
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

/// How a panel's frame looks: at rest, with the focus or selection, or as the drop target of a drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Idle,
    Focus,
    Drop,
}

/// A heavy steel frame at rest, a double blue one with the focus, a double orange one as a drop target.
/// The line style differs as well as the colour, so focus is never shown by colour alone.
pub fn frame(edge: Edge) -> (BorderType, Style) {
    match edge {
        Edge::Idle => (BorderType::Thick, Style::new().fg(FRAME)),
        Edge::Focus => (BorderType::Double, Style::new().fg(BLUE)),
        Edge::Drop => (BorderType::Double, Style::new().fg(ORANGE)),
    }
}

/// The name plate in a panel's top edge: cream on console blue at rest, dark on a lit colour otherwise.
pub fn plate(edge: Edge) -> Style {
    let (fg, bg) = match edge {
        Edge::Idle => (FG, HEADER),
        Edge::Focus => (BG, BLUE),
        Edge::Drop => (BG, ORANGE),
    };
    Style::new().fg(fg).bg(bg).add_modifier(Modifier::BOLD)
}

/// A panel's block with its frame and name plate.
pub fn panel_block<'a>(edge: Edge, title: impl Into<ratatui::text::Line<'a>>) -> Block<'a> {
    let (kind, style) = frame(edge);
    Block::bordered().border_type(kind).border_style(style).title(title.into())
}

/// Lay the board's floor over a rectangle.
pub fn board_floor(f: &mut Frame, area: Rect) {
    f.render_widget(Block::new().style(Style::new().bg(BOARD_BG)), area);
}
