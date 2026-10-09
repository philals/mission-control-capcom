//! The wait for the first list of pull requests, as a 1960s Mission Control go/no-go poll: the
//! Flight Director calls each console in turn and each answers "GO". It is decoration, not
//! progress: the real status is in the panel's title.
use crate::{panel, theme};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// One animation frame lasts this long.
pub const FRAME_MILLIS: u64 = 250;

/// The consoles, in the order the Flight Director polled them.
const STATIONS: [&str; 7] = ["RETRO", "FIDO", "GUIDO", "CONTROL", "TELMU", "SURGEON", "CAPCOM"];
const FRAMES_PER_STATION: u64 = 3;
const NAME_WIDTH: usize = 9;
const BLOCK_WIDTH: usize = 34;
const WAVE: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

fn dim() -> Style {
    Style::new().fg(theme::DIM)
}

fn amber() -> Style {
    Style::new().fg(panel::ORANGE).add_modifier(Modifier::BOLD)
}

/// A sweep of bars that travels along the strip, like a signal being acquired.
fn signal(frame: u64, width: usize) -> String {
    (0..width).map(|i| WAVE[((i as u64 + frame) % 8) as usize]).collect()
}

fn centered(spans: Vec<Span<'static>>, content_width: usize, width: u16) -> Line<'static> {
    let pad = (width as usize).saturating_sub(content_width) / 2;
    let mut all = vec![Span::raw(" ".repeat(pad))];
    all.extend(spans);
    Line::from(all)
}

/// The consoles that have answered by this frame, and whether all of them have.
fn answered(frame: u64) -> (usize, bool) {
    let done = (frame / FRAMES_PER_STATION) as usize;
    (done.min(STATIONS.len()), done >= STATIONS.len())
}

fn station_line(index: usize, frame: u64, width: u16) -> Line<'static> {
    let (done, _) = answered(frame);
    let name = format!("{:<NAME_WIDTH$}", STATIONS[index]);
    let dots = "·".repeat(BLOCK_WIDTH - NAME_WIDTH - 12);
    let reply = if index < done {
        Span::styled("GO", Style::new().fg(theme::GREEN).add_modifier(Modifier::BOLD))
    } else if index == done {
        // blinks, so a stalled poll still looks alive
        Span::styled(if frame % 2 == 0 { "STANDING BY" } else { "           " }, dim())
    } else {
        Span::raw("")
    };
    centered(vec![Span::styled(name, Style::new().fg(theme::FG)), Span::styled(dots, dim()), Span::raw(" "), reply], BLOCK_WIDTH, width)
}

/// The lines to show in a space of this size. Small spaces get a shorter version, never a cut-off one.
pub fn lines(frame: u64, width: u16, height: u16) -> Vec<Line<'static>> {
    let (done, all_go) = answered(frame);
    let flight = if all_go { "ALL STATIONS GO" } else { "GO / NO-GO FOR PR UPLINK" };
    let waiting = if all_go { "AWAITING TELEMETRY" } else { "ACQUIRING SIGNAL" };
    let strip = |n: usize| Span::styled(signal(frame, n), Style::new().fg(theme::CYAN));
    if (width as usize) < BLOCK_WIDTH + 2 || height < 5 {
        let text = "STAND BY FOR TELEMETRY";
        return vec![Line::from(vec![Span::styled(text, amber()), Span::raw(" "), strip(6)])];
    }
    let mut out = vec![
        centered(vec![Span::styled("MISSION CONTROL · HOUSTON", amber())], 25, width),
        centered(vec![Span::styled("FLIGHT ▸ ", dim()), Span::styled(flight, Style::new().fg(theme::FG).add_modifier(Modifier::BOLD))], 9 + flight.chars().count(), width),
        Line::raw(""),
    ];
    if height >= 14 {
        out.extend((0..STATIONS.len()).map(|i| station_line(i, frame, width)));
    } else {
        // not enough rows for every console: just the one being polled
        out.push(station_line(done.min(STATIONS.len() - 1), frame, width));
    }
    out.push(Line::raw(""));
    out.push(centered(vec![strip(10), Span::raw("  "), Span::styled(waiting, dim())], 12 + waiting.chars().count(), width));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line<'_>]) -> String {
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn the_poll_starts_with_nobody_answered_and_ends_with_every_console_go() {
        let start = text(&lines(0, 60, 20));
        assert!(start.contains("MISSION CONTROL") && start.contains("GO / NO-GO FOR PR UPLINK"), "{start}");
        assert!(start.contains("RETRO") && start.contains("STANDING BY"), "{start}");
        let answers = |frame| text(&lines(frame, 60, 20)).lines().filter(|l| l.contains("····") && l.trim_end().ends_with(" GO")).count();
        assert_eq!((answers(0), answers(3), answers(7), answers(40)), (0, 1, 2, 7), "one console answers every three frames");
        let end = text(&lines(40, 60, 20));
        assert!(end.contains("ALL STATIONS GO") && end.contains("AWAITING TELEMETRY") && !end.contains("STANDING BY"), "{end}");
    }

    #[test]
    fn the_standing_by_reply_blinks_and_the_signal_moves() {
        assert_ne!(text(&lines(0, 60, 20)), text(&lines(1, 60, 20)));
        assert_ne!(signal(0, 10), signal(1, 10));
        assert_eq!(signal(0, 10).chars().count(), 10);
    }

    #[test]
    fn small_spaces_get_a_shorter_poll_and_tiny_ones_a_single_line() {
        let medium = lines(0, 60, 9);
        assert!(medium.len() <= 9, "{} lines for 9 rows", medium.len());
        let medium = text(&medium);
        assert!(medium.contains("RETRO") && !medium.contains("SURGEON"), "only the console being polled:\n{medium}");
        assert!(text(&lines(4, 60, 9)).contains("FIDO"), "the poll moves on");
        for (w, h) in [(20, 20), (60, 3), (10, 1)] {
            let one = lines(2, w, h);
            assert_eq!(one.len(), 1, "{w}x{h}");
            assert!(text(&one).contains("STAND BY FOR TELEMETRY"));
        }
        assert!(lines(0, 60, 14).len() <= 14 && lines(0, 80, 40).len() <= 14);
    }

    #[test]
    fn every_line_fits_the_width_it_was_given() {
        for width in [36u16, 50, 120] {
            for line in lines(5, width, 30) {
                assert!(line.width() <= width as usize, "{} wide in {width}", line.width());
            }
        }
    }
}
