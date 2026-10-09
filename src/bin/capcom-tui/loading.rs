//! What a panel shows while its first data is on the way: a 1960s space-programme scene. The pull
//! requests panel gets Mission Control's go/no-go poll (the Flight Director calls each console in
//! turn and each answers "GO"); the runs panel gets the launch pad's countdown and checklist, which
//! holds at T-minus 3 if GitHub is slow. Both share one look: an amber title, a subtitle, a
//! checklist that answers line by line, and a signal strip. It is decoration, not progress: the real
//! status ("↻ refreshing…") stays in the panel's title.
use crate::{panel, theme};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// One animation frame lasts this long.
pub const FRAME_MILLIS: u64 = 250;
const FRAMES_PER_SECOND: u64 = 1000 / FRAME_MILLIS;
/// The countdown runs from T-10 and holds at T-3 after this many seconds.
const HOLD_AFTER_SECONDS: u64 = 7;

const NAME_WIDTH: usize = 9;
const BLOCK_WIDTH: usize = 34;
/// The widest line of either scene (the countdown) plus a margin: narrower panels get one line.
const FULL_WIDTH: usize = 40;
/// The longest reply ("STANDING BY") and the space it is given.
const REPLY_WIDTH: usize = 11;
const WAVE: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scene {
    /// Mission Control polls its consoles for go/no-go.
    Pulls,
    /// The launch pad counts down through its checklist.
    Runs,
}

struct Script {
    title: &'static str,
    /// Console or checklist item, and what it replies once checked.
    items: &'static [(&'static str, &'static str)],
    /// What the one being checked shows (it blinks).
    pending: &'static str,
    frames_per_item: u64,
}

impl Scene {
    fn script(self) -> Script {
        match self {
            Scene::Pulls => Script {
                title: "MISSION CONTROL · HOUSTON",
                items: &[("RETRO", "GO"), ("FIDO", "GO"), ("GUIDO", "GO"), ("CONTROL", "GO"), ("TELMU", "GO"), ("SURGEON", "GO"), ("CAPCOM", "GO")],
                pending: "STANDING BY",
                frames_per_item: 3,
            },
            Scene::Runs => Script {
                title: "LAUNCH CONTROL · CAPE CANAVERAL",
                items: &[("PAD", "CLEAR"), ("RANGE", "GREEN"), ("GANTRY", "RETRACTED"), ("FUEL", "LOADED"), ("COMMS", "LOCKED")],
                pending: "CHECKING",
                frames_per_item: 3,
            },
        }
    }

    /// The label and text of the subtitle line.
    fn subtitle(self, frame: u64, all_done: bool) -> (&'static str, String) {
        match self {
            Scene::Pulls => ("FLIGHT ▸ ", if all_done { "ALL STATIONS GO" } else { "GO / NO-GO FOR PR UPLINK" }.to_string()),
            Scene::Runs => {
                // T-10 and counting, one second a step, until the hold at T-3
                let left = (10 - HOLD_AFTER_SECONDS.min(frame / FRAMES_PER_SECOND)).max(3);
                if left == 3 {
                    ("COUNTDOWN ▸ ", "HOLD AT T-MINUS 00:03".to_string())
                } else {
                    ("COUNTDOWN ▸ ", format!("T-MINUS 00:{left:02} AND COUNTING"))
                }
            }
        }
    }

    /// The words beside the signal strip. The pad only says it is holding once the clock does.
    fn waiting(self, frame: u64, all_done: bool) -> &'static str {
        match self {
            Scene::Pulls if all_done => "AWAITING TELEMETRY",
            Scene::Runs if frame / FRAMES_PER_SECOND >= HOLD_AFTER_SECONDS => "HOLDING FOR TELEMETRY",
            _ => "ACQUIRING SIGNAL",
        }
    }
}

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

/// How many items have answered by this frame, and whether every one has.
fn answered(script: &Script, frame: u64) -> (usize, bool) {
    let done = (frame / script.frames_per_item) as usize;
    (done.min(script.items.len()), done >= script.items.len())
}

fn item_line(script: &Script, index: usize, done: usize, frame: u64, width: u16) -> Line<'static> {
    let (name, reply) = script.items[index];
    let dots = "·".repeat(BLOCK_WIDTH - NAME_WIDTH - REPLY_WIDTH - 1);
    let reply = if index < done {
        Span::styled(reply, Style::new().fg(theme::GREEN).add_modifier(Modifier::BOLD))
    } else if index == done {
        // blinks, so a stalled wait still looks alive
        Span::styled(if frame % 2 == 0 { script.pending } else { "" }, dim())
    } else {
        Span::raw("")
    };
    centered(
        vec![Span::styled(format!("{name:<NAME_WIDTH$}"), Style::new().fg(theme::FG)), Span::styled(dots, dim()), Span::raw(" "), reply],
        BLOCK_WIDTH,
        width,
    )
}

/// The lines to show in a space of this size. Small spaces get a shorter version, never a cut-off one.
pub fn lines(scene: Scene, frame: u64, width: u16, height: u16) -> Vec<Line<'static>> {
    let script = scene.script();
    let (done, all_done) = answered(&script, frame);
    let strip = |n: usize| Span::styled(signal(frame, n), Style::new().fg(theme::CYAN));
    if (width as usize) < FULL_WIDTH || height < 5 {
        return vec![Line::from(vec![Span::styled("STAND BY FOR TELEMETRY", amber()), Span::raw(" "), strip(6)])];
    }
    let (label, text) = scene.subtitle(frame, all_done);
    let title_width = script.title.chars().count();
    let waiting = scene.waiting(frame, all_done);
    let mut out = vec![
        centered(vec![Span::styled(script.title, amber())], title_width, width),
        centered(
            vec![Span::styled(label, dim()), Span::styled(text.clone(), Style::new().fg(theme::FG).add_modifier(Modifier::BOLD))],
            label.chars().count() + text.chars().count(),
            width,
        ),
        Line::raw(""),
    ];
    // title, subtitle, blank, the items, blank and the strip
    if height as usize >= script.items.len() + 5 {
        out.extend((0..script.items.len()).map(|i| item_line(&script, i, done, frame, width)));
    } else {
        // not enough rows for every item: just the one being checked
        out.push(item_line(&script, done.min(script.items.len() - 1), done, frame, width));
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

    /// How many checklist lines have answered by this frame.
    fn answers(scene: Scene, frame: u64) -> usize {
        let script = scene.script();
        let replies: Vec<&str> = script.items.iter().map(|(_, reply)| *reply).collect();
        text(&lines(scene, frame, 60, 20))
            .lines()
            .filter(|l| l.contains("····") && replies.iter().any(|r| l.trim_end().ends_with(&format!(" {r}"))))
            .count()
    }

    #[test]
    fn mission_control_polls_each_console_in_turn_until_all_stations_are_go() {
        let start = text(&lines(Scene::Pulls, 0, 60, 20));
        assert!(start.contains("MISSION CONTROL · HOUSTON") && start.contains("GO / NO-GO FOR PR UPLINK"), "{start}");
        assert!(start.contains("RETRO") && start.contains("STANDING BY"), "{start}");
        let counts: Vec<usize> = [0, 3, 7, 40].iter().map(|f| answers(Scene::Pulls, *f)).collect();
        assert_eq!(counts, vec![0, 1, 2, 7], "one console answers every three frames");
        let end = text(&lines(Scene::Pulls, 40, 60, 20));
        assert!(end.contains("ALL STATIONS GO") && end.contains("AWAITING TELEMETRY") && !end.contains("STANDING BY"), "{end}");
    }

    #[test]
    fn the_launch_pad_counts_down_checks_its_list_and_holds_at_t_minus_three() {
        let start = text(&lines(Scene::Runs, 0, 60, 20));
        assert!(start.contains("LAUNCH CONTROL · CAPE CANAVERAL") && start.contains("T-MINUS 00:10 AND COUNTING"), "{start}");
        assert!(start.contains("PAD") && start.contains("CHECKING"), "{start}");
        let counting = text(&lines(Scene::Runs, 8, 60, 20));
        assert!(counting.contains("T-MINUS 00:08 AND COUNTING") && counting.contains("ACQUIRING SIGNAL"), "one second is four frames:\n{counting}");
        let almost = text(&lines(Scene::Runs, 27, 60, 20));
        assert!(almost.contains("T-MINUS 00:04 AND COUNTING") && !almost.contains("HOLDING"), "not holding until the clock does:\n{almost}");
        let counts: Vec<usize> = [0, 3, 40].iter().map(|f| answers(Scene::Runs, *f)).collect();
        assert_eq!(counts, vec![0, 1, 5]);
        let hold = text(&lines(Scene::Runs, 200, 60, 20));
        assert!(hold.contains("HOLD AT T-MINUS 00:03") && !hold.contains("AND COUNTING"), "the clock never runs out:\n{hold}");
        assert!(hold.contains("CLEAR") && hold.contains("RETRACTED") && hold.contains("LOCKED") && hold.contains("HOLDING FOR TELEMETRY"), "{hold}");
    }

    #[test]
    fn the_two_scenes_share_one_look_and_say_different_things() {
        for frame in [0, 9, 50] {
            let (pulls, runs) = (lines(Scene::Pulls, frame, 60, 20), lines(Scene::Runs, frame, 60, 20));
            assert_eq!(pulls[0].spans[1].style, runs[0].spans[1].style, "the same amber title style");
            assert_eq!(pulls.last().unwrap().spans[1].style, runs.last().unwrap().spans[1].style, "the same signal strip");
            assert_ne!(text(&pulls), text(&runs));
        }
    }

    #[test]
    fn the_waiting_reply_blinks_and_the_signal_moves() {
        for scene in [Scene::Pulls, Scene::Runs] {
            assert_ne!(text(&lines(scene, 0, 60, 20)), text(&lines(scene, 1, 60, 20)));
        }
        assert_ne!(signal(0, 10), signal(1, 10));
        assert_eq!(signal(0, 10).chars().count(), 10);
    }

    #[test]
    fn small_spaces_get_a_shorter_scene_and_tiny_ones_a_single_line() {
        for scene in [Scene::Pulls, Scene::Runs] {
            let first = scene.script().items[0].0;
            let last = scene.script().items.last().unwrap().0;
            let medium = lines(scene, 0, 60, 6);
            assert!(medium.len() <= 6, "{} lines for 6 rows", medium.len());
            let medium = text(&medium);
            assert!(medium.contains(first) && !medium.contains(last), "only the item being checked:\n{medium}");
            for (w, h) in [(20, 20), (60, 3), (10, 1)] {
                let one = lines(scene, 2, w, h);
                assert_eq!(one.len(), 1, "{scene:?} {w}x{h}");
                assert!(text(&one).contains("STAND BY FOR TELEMETRY"));
            }
            let full = (scene.script().items.len() + 5) as u16;
            assert!(lines(scene, 0, 60, full).len() <= full as usize, "the full scene fits exactly its own height");
            assert!(text(&lines(scene, 0, 60, full)).contains(last), "and shows every item there");
        }
    }

    #[test]
    fn every_line_fits_the_width_it_was_given() {
        for scene in [Scene::Pulls, Scene::Runs] {
            for width in [40u16, 50, 120] {
                for frame in [0, 5, 100] {
                    for line in lines(scene, frame, width, 30) {
                        assert!(line.width() <= width as usize, "{scene:?} {} wide in {width}", line.width());
                    }
                }
            }
        }
    }
}
