//! Turns a drawn terminal screen into an SVG picture (used for the README screenshots).
//! Box lines, blocks and bars are drawn as shapes, so the picture looks the same in every
//! viewer whatever fonts it has; text is placed word by word on the character grid.
use crate::theme;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

const CELL_W: f64 = 8.4;
const CELL_H: f64 = 18.0;
const PAD: f64 = 8.0;
const FONT: &str = "'DejaVu Sans Mono', 'SFMono-Regular', Menlo, Consolas, 'Liberation Mono', monospace";

fn hex(color: Color, fallback: Color) -> String {
    match if color == Color::Reset { fallback } else { color } {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => "#ffffff".to_string(),
    }
}

/// `amount` of the foreground over the background, as a solid colour (so no viewer needs opacity).
fn blend(fg: Color, bg: Color, amount: f64) -> String {
    let rgb = |c: Color, fallback: Color| match if c == Color::Reset { fallback } else { c } {
        Color::Rgb(r, g, b) => (f64::from(r), f64::from(g), f64::from(b)),
        _ => (255.0, 255.0, 255.0),
    };
    let (f, b) = (rgb(fg, theme::FG), rgb(bg, theme::BG));
    let mix = |f: f64, b: f64| (b + (f - b) * amount).round() as u8;
    format!("#{:02x}{:02x}{:02x}", mix(f.0, b.0), mix(f.1, b.1), mix(f.2, b.2))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn is_wide(symbol: &str) -> bool {
    symbol.chars().next().is_some_and(|c| c >= '\u{1F000}')
}

/// Arm weights (up, down, left, right): 0 none, 1 light, 2 heavy, 3 double; and whether the corner is rounded.
fn arms(c: char) -> Option<([u8; 4], bool)> {
    let a = |u, d, l, r| Some(([u, d, l, r], false));
    match c {
        '─' => a(0, 0, 1, 1),
        '│' => a(1, 1, 0, 0),
        '┌' => a(0, 1, 0, 1),
        '┐' => a(0, 1, 1, 0),
        '└' => a(1, 0, 0, 1),
        '┘' => a(1, 0, 1, 0),
        '├' => a(1, 1, 0, 1),
        '┤' => a(1, 1, 1, 0),
        '┬' => a(0, 1, 1, 1),
        '┴' => a(1, 0, 1, 1),
        '┼' => a(1, 1, 1, 1),
        '━' => a(0, 0, 2, 2),
        '┃' => a(2, 2, 0, 0),
        '┏' => a(0, 2, 0, 2),
        '┓' => a(0, 2, 2, 0),
        '┗' => a(2, 0, 0, 2),
        '┛' => a(2, 0, 2, 0),
        '═' => a(0, 0, 3, 3),
        '║' => a(3, 3, 0, 0),
        '╔' => a(0, 3, 0, 3),
        '╗' => a(0, 3, 3, 0),
        '╚' => a(3, 0, 0, 3),
        '╝' => a(3, 0, 3, 0),
        '╭' => Some(([0, 1, 0, 1], true)),
        '╮' => Some(([0, 1, 1, 0], true)),
        '╰' => Some(([1, 0, 0, 1], true)),
        '╯' => Some(([1, 0, 1, 0], true)),
        _ => None,
    }
}

fn line(x1: f64, y1: f64, x2: f64, y2: f64) -> String {
    format!("M{x1:.1} {y1:.1}L{x2:.1} {y2:.1}")
}

/// SVG path data for one box-drawing glyph in the cell whose top-left corner is (x, y).
fn box_path(c: char, x: f64, y: f64) -> Option<(String, f64)> {
    let ([up, down, left, right], rounded) = arms(c)?;
    let (cx, cy) = (x + CELL_W / 2.0, y + CELL_H / 2.0);
    let (x1, y1) = (x + CELL_W, y + CELL_H);
    let heavy = [up, down, left, right].contains(&2);
    let double = [up, down, left, right].contains(&3);
    let width = if heavy { 2.4 } else { 1.2 };
    let d = 2.2;
    let corner = (up > 0) as u8 + (down > 0) as u8 + (left > 0) as u8 + (right > 0) as u8 == 2 && (up > 0 || down > 0) && (left > 0 || right > 0);
    let mut path = String::new();
    if rounded {
        let r = CELL_W / 2.0;
        let dx = if right > 0 { 1.0 } else { -1.0 };
        let dy = if down > 0 { 1.0 } else { -1.0 };
        let edge_x = if right > 0 { x1 } else { x };
        let edge_y = if down > 0 { y1 } else { y };
        path.push_str(&format!(
            "M{edge_x:.1} {cy:.1}L{:.1} {cy:.1}Q{cx:.1} {cy:.1} {cx:.1} {:.1}L{cx:.1} {edge_y:.1}",
            cx + dx * r,
            cy + dy * r
        ));
    } else if double && corner {
        let dx = if right > 0 { 1.0 } else { -1.0 };
        let dy = if down > 0 { 1.0 } else { -1.0 };
        let edge_x = if right > 0 { x1 } else { x };
        let edge_y = if down > 0 { y1 } else { y };
        for sign in [-1.0, 1.0] {
            let (px, py) = (cx + sign * dx * d, cy + sign * dy * d);
            path.push_str(&format!("M{edge_x:.1} {py:.1}L{px:.1} {py:.1}L{px:.1} {edge_y:.1}"));
        }
    } else if double {
        if left > 0 && right > 0 {
            path.push_str(&line(x, cy - d, x1, cy - d));
            path.push_str(&line(x, cy + d, x1, cy + d));
        } else {
            path.push_str(&line(cx - d, y, cx - d, y1));
            path.push_str(&line(cx + d, y, cx + d, y1));
        }
    } else {
        if up > 0 {
            path.push_str(&line(cx, y, cx, cy));
        }
        if down > 0 {
            path.push_str(&line(cx, cy, cx, y1));
        }
        if left > 0 {
            path.push_str(&line(x, cy, cx, cy));
        }
        if right > 0 {
            path.push_str(&line(cx, cy, x1, cy));
        }
    }
    Some((path, if double { 1.1 } else { width }))
}

/// A small drawn rocket for the header, so the picture does not depend on an emoji font.
fn rocket(x: f64, y: f64) -> String {
    let cx = x + CELL_W;
    let (white, blue, red, orange) = (hex(theme::FG, theme::FG), hex(theme::CYAN, theme::FG), hex(theme::RED, theme::FG), hex(theme::ORANGE, theme::FG));
    format!(
        "<path d=\"M{cx:.1} {:.1}C{:.1} {:.1} {:.1} {:.1} {:.1} {:.1}L{:.1} {:.1}C{:.1} {:.1} {:.1} {:.1} {cx:.1} {:.1}Z\" fill=\"{white}\"/>\n<path d=\"M{:.1} {:.1}L{:.1} {:.1}L{:.1} {:.1}ZM{:.1} {:.1}L{:.1} {:.1}L{:.1} {:.1}Z\" fill=\"{red}\"/>\n<circle cx=\"{cx:.1}\" cy=\"{:.1}\" r=\"1.9\" fill=\"{blue}\"/>\n<path d=\"M{:.1} {:.1}L{cx:.1} {:.1}L{:.1} {:.1}Z\" fill=\"{orange}\"/>\n",
        y + 2.0,
        cx + 5.5, y + 6.0, cx + 5.0, y + 10.0, cx + 3.6, y + 13.0,
        cx - 3.6, y + 13.0,
        cx - 5.0, y + 10.0, cx - 5.5, y + 6.0, y + 2.0,
        cx - 3.6, y + 10.0, cx - 7.2, y + 15.0, cx - 3.6, y + 13.0,
        cx + 3.6, y + 10.0, cx + 7.2, y + 15.0, cx + 3.6, y + 13.0,
        y + 8.0,
        cx - 2.2, y + 13.5, y + 17.6, cx + 2.2, y + 13.5,
    )
}

#[derive(Default)]
struct Segment {
    start: u16,
    cells: u16,
    fill: String,
    bold: bool,
    text: String,
}

pub fn render(buf: &Buffer, title: &str, description: &str) -> String {
    let (w, h) = (buf.area.width, buf.area.height);
    let (px_w, px_h) = (f64::from(w) * CELL_W + 2.0 * PAD, f64::from(h) * CELL_H + 2.0 * PAD);
    let base_bg = hex(theme::BG, theme::BG);
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{px_w:.0}\" height=\"{px_h:.0}\" viewBox=\"0 0 {px_w:.0} {px_h:.0}\" role=\"img\" aria-labelledby=\"t d\">\n<title id=\"t\">{}</title>\n<desc id=\"d\">{}</desc>\n<rect width=\"100%\" height=\"100%\" rx=\"10\" fill=\"{base_bg}\"/>\n",
        escape(title),
        escape(description)
    );
    let mut rects = String::new();
    let mut shapes = String::new();
    let mut texts = String::new();
    for y in 0..h {
        let top = PAD + f64::from(y) * CELL_H;
        let mut seg = Segment::default();
        let mut rect: Option<(u16, String)> = None;
        let flush = |seg: &mut Segment, texts: &mut String| {
            if !seg.text.is_empty() {
                texts.push_str(&format!(
                    "<text x=\"{:.1}\" y=\"{:.1}\" fill=\"{}\"{} textLength=\"{:.1}\" lengthAdjust=\"spacing\">{}</text>\n",
                    PAD + f64::from(seg.start) * CELL_W,
                    top + 13.5,
                    seg.fill,
                    if seg.bold { " font-weight=\"700\"" } else { "" },
                    f64::from(seg.cells) * CELL_W,
                    escape(&seg.text)
                ));
            }
            *seg = Segment::default();
        };
        let close_rect = |rect: &mut Option<(u16, String)>, end: u16, rects: &mut String| {
            if let Some((start, color)) = rect.take() {
                if color != base_bg {
                    rects.push_str(&format!(
                        "<rect x=\"{:.1}\" y=\"{top:.1}\" width=\"{:.1}\" height=\"{CELL_H}\" fill=\"{color}\"/>\n",
                        PAD + f64::from(start) * CELL_W,
                        f64::from(end - start) * CELL_W
                    ));
                }
            }
        };
        let mut x = 0;
        while x < w {
            let cell = &buf[(x, y)];
            let bg = hex(cell.bg, theme::BG);
            if rect.as_ref().map(|(_, c)| c) != Some(&bg) {
                close_rect(&mut rect, x, &mut rects);
                rect = Some((x, bg));
            }
            let fill = hex(cell.fg, theme::FG);
            let bold = cell.modifier.contains(Modifier::BOLD);
            let symbol = cell.symbol();
            let width = if is_wide(symbol) { 2 } else { 1 };
            let left = PAD + f64::from(x) * CELL_W;
            let first = symbol.chars().next().unwrap_or(' ');
            if let Some((path, stroke)) = box_path(first, left, top) {
                flush(&mut seg, &mut texts);
                shapes.push_str(&format!(
                    "<path d=\"{path}\" stroke=\"{fill}\" stroke-width=\"{stroke}\" fill=\"none\" stroke-linecap=\"square\"/>\n"
                ));
            } else if first == '🚀' {
                flush(&mut seg, &mut texts);
                shapes.push_str(&rocket(left, top));
            } else if matches!(first, '█' | '░' | '▌') {
                flush(&mut seg, &mut texts);
                let (rw, color) = match first {
                    '█' => (CELL_W, fill),
                    '░' => (CELL_W, blend(cell.fg, cell.bg, 0.22)),
                    _ => (CELL_W / 2.0, fill),
                };
                shapes.push_str(&format!(
                    "<rect x=\"{left:.1}\" y=\"{top:.1}\" width=\"{rw:.1}\" height=\"{CELL_H}\" fill=\"{color}\"/>\n"
                ));
            } else if first == ' ' || symbol.is_empty() {
                flush(&mut seg, &mut texts);
            } else {
                if seg.text.is_empty() {
                    seg = Segment { start: x, cells: 0, fill: fill.clone(), bold, text: String::new() };
                } else if seg.fill != fill || seg.bold != bold || width == 2 {
                    flush(&mut seg, &mut texts);
                    seg = Segment { start: x, cells: 0, fill: fill.clone(), bold, text: String::new() };
                }
                seg.text.push_str(symbol);
                seg.cells += width;
                if width == 2 {
                    flush(&mut seg, &mut texts);
                }
            }
            x += width;
        }
        flush(&mut seg, &mut texts);
        close_rect(&mut rect, w, &mut rects);
    }
    out.push_str(&rects);
    out.push_str(&shapes);
    out.push_str(&format!("<g font-family=\"{FONT}\" font-size=\"14\">\n{texts}</g>\n</svg>\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    fn screen(width: u16) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, 3));
        buf.set_style(buf.area, Style::new().fg(theme::FG).bg(theme::BG));
        buf
    }

    #[test]
    fn text_colour_background_and_bold_come_through_and_markup_is_escaped() {
        let mut buf = screen(12);
        buf.set_string(0, 0, "a<b&c", Style::new().fg(theme::ORANGE).bg(theme::HEADER).add_modifier(Modifier::BOLD));
        buf.set_string(0, 1, "plain", Style::new().fg(theme::GREEN).bg(theme::BG));
        let svg = render(&buf, "Title", "Description");
        assert!(svg.starts_with("<svg") && svg.trim_end().ends_with("</svg>"));
        assert!(svg.contains("a&lt;b&amp;c"), "{svg}");
        assert!(svg.contains("font-weight=\"700\""));
        assert!(svg.contains(&hex(theme::ORANGE, theme::FG)) && svg.contains(&hex(theme::HEADER, theme::BG)));
        assert!(svg.contains(&hex(theme::GREEN, theme::FG)));
        assert!(svg.contains("<title id=\"t\">Title</title>") && svg.contains("Description"));
    }

    #[test]
    fn words_are_placed_on_their_own_columns_so_spaces_never_matter() {
        let mut buf = screen(20);
        buf.set_string(2, 0, "ab   cd", Style::new().fg(theme::FG).bg(theme::BG));
        let svg = render(&buf, "t", "d");
        assert_eq!(svg.matches("<text ").count(), 2, "{svg}");
        assert!(svg.contains(&format!("x=\"{:.1}\"", PAD + 2.0 * CELL_W)));
        assert!(svg.contains(&format!("x=\"{:.1}\"", PAD + 7.0 * CELL_W)), "the second word starts five columns later");
    }

    #[test]
    fn a_wide_character_takes_two_cells_and_blank_rows_make_no_text() {
        let mut buf = screen(6);
        buf.set_string(0, 0, "🚀x", Style::new().fg(theme::FG).bg(theme::BG));
        let svg = render(&buf, "t", "d");
        assert_eq!(svg.matches("<text ").count(), 1, "the rocket is drawn, only the x is text:\n{svg}");
        assert!(svg.contains("<circle"), "the rocket window");
        assert!(svg.contains(&format!("x=\"{:.1}\"", PAD + 2.0 * CELL_W)), "x sits after the two-cell rocket");
    }

    #[test]
    fn box_lines_blocks_and_bars_are_shapes_not_text() {
        let mut buf = screen(10);
        buf.set_string(0, 0, "┌─┐╔═╗╭╮", Style::new().fg(theme::BLUE).bg(theme::BG));
        buf.set_string(0, 1, "│ │║ ║", Style::new().fg(theme::BLUE).bg(theme::BG));
        buf.set_string(0, 2, "██░░▌", Style::new().fg(theme::YELLOW).bg(theme::BG));
        let svg = render(&buf, "t", "d");
        assert_eq!(svg.matches("<text ").count(), 0, "{svg}");
        assert!(svg.matches("<path ").count() >= 10);
        assert!(!svg.contains("opacity"), "shades are solid blended colours");
        assert_ne!(blend(theme::YELLOW, theme::BG, 0.22), hex(theme::YELLOW, theme::FG));
    }

    #[test]
    fn every_glyph_the_ui_draws_has_a_shape() {
        for c in "─│┌┐└┘├┤┬┴┼━┃┏┓┗┛═║╔╗╚╝╭╮╰╯".chars() {
            assert!(box_path(c, 0.0, 0.0).is_some(), "{c}");
        }
        assert!(box_path('x', 0.0, 0.0).is_none());
    }
}
