//! What a task's markdown file says, for the details sheet: what is still to be planned, or the plan.
use crate::model::Status;

/// The text under a `## name` heading, up to the next `## ` heading (`###` headings stay inside it).
pub fn section(doc: &str, name: &str) -> Option<String> {
    let mut inside = false;
    let mut found = false;
    let mut body: Vec<&str> = Vec::new();
    for line in doc.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            if inside {
                break;
            }
            inside = title.trim().eq_ignore_ascii_case(name);
            found |= inside;
        } else if inside {
            body.push(line);
        }
    }
    found.then(|| body.join("\n").trim().to_string())
}

pub struct Summary {
    pub label: &'static str,
    pub body: String,
}

/// A task still to be planned shows its description (what is going to be planned); a planned, started or
/// finished one shows its plan, or its description when no plan was written.
pub fn summary(doc: &str, status: Status) -> Summary {
    let description = section(doc, "Description").filter(|t| !t.is_empty());
    let plan = section(doc, "Plan").filter(|t| !t.is_empty());
    match (matches!(status, Status::Todo | Status::Planning), plan, description) {
        (false, Some(plan), _) => Summary { label: "Plan", body: plan },
        (false, None, Some(description)) => Summary { label: "Description (no plan written yet)", body: description },
        (true, _, Some(description)) => Summary { label: "To plan", body: description },
        (true, Some(plan), None) => Summary { label: "Plan", body: plan },
        (_, None, None) => Summary { label: "Description", body: "Nothing written yet.".to_string() },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Heading,
    Text,
    Blank,
}

/// The body wrapped to `width`, one screen line each: bullets keep a hanging indent, `#` headings are
/// marked, runs of blank lines collapse to one.
pub fn layout(body: &str, width: usize) -> Vec<(Kind, String)> {
    let width = width.max(8);
    let mut out: Vec<(Kind, String)> = Vec::new();
    for raw in body.lines() {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            if out.last().is_some_and(|(k, _)| *k != Kind::Blank) {
                out.push((Kind::Blank, String::new()));
            }
            continue;
        }
        if line.trim_start().starts_with('#') {
            let title = line.trim_start().trim_start_matches('#').trim();
            for wrapped in wrap(title, 0, 0, width) {
                out.push((Kind::Heading, wrapped));
            }
            continue;
        }
        let indent = line.chars().take_while(|c| *c == ' ').count();
        let rest = line.trim_start();
        let hang = if rest.starts_with("- ") || rest.starts_with("* ") {
            2
        } else {
            let digits = rest.chars().take_while(char::is_ascii_digit).count();
            if digits > 0 && rest[digits..].starts_with(". ") { digits + 2 } else { 0 }
        };
        for wrapped in wrap(rest, indent, hang, width) {
            out.push((Kind::Text, wrapped));
        }
    }
    while out.last().is_some_and(|(k, _)| *k == Kind::Blank) {
        out.pop();
    }
    out
}

fn wrap(text: &str, indent: usize, hang: usize, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = " ".repeat(indent);
    let mut fresh = true;
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        let room = |line: &str| width.saturating_sub(line.chars().count());
        if !fresh && room(&current) < word.chars().count() + 1 {
            lines.push(std::mem::replace(&mut current, " ".repeat(indent + hang)));
            fresh = true;
        }
        while word.chars().count() > room(&current).max(1) && fresh {
            let take = room(&current).max(1);
            current.extend(word.chars().take(take));
            lines.push(std::mem::replace(&mut current, " ".repeat(indent + hang)));
            word = word.chars().skip(take).collect();
        }
        if word.is_empty() {
            continue;
        }
        if !fresh {
            current.push(' ');
        }
        current.push_str(&word);
        fresh = false;
    }
    if !fresh || lines.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "# T2: Helper\n\nType: pr\n\n## Description\n\nBuild the helper.\n\n## Plan\n\n### Approach\n\nAdd `isFlagEnabled`.\n\n- one\n\n## Decisions\n\nnone\n\n## Progress\n";

    #[test]
    fn a_section_runs_to_the_next_level_two_heading_and_keeps_its_subheadings() {
        assert_eq!(section(DOC, "Description").as_deref(), Some("Build the helper."));
        assert_eq!(section(DOC, "Plan").as_deref(), Some("### Approach\n\nAdd `isFlagEnabled`.\n\n- one"));
        assert_eq!(section(DOC, "Progress").as_deref(), Some(""));
        assert_eq!(section(DOC, "Nope"), None);
    }

    #[test]
    fn a_task_to_plan_shows_what_will_be_planned_and_a_planned_one_shows_its_plan() {
        for status in [Status::Todo, Status::Planning] {
            let s = summary(DOC, status);
            assert_eq!((s.label, s.body.as_str()), ("To plan", "Build the helper."));
        }
        for status in [Status::Planned, Status::Implementing, Status::Done] {
            let s = summary(DOC, status);
            assert_eq!(s.label, "Plan");
            assert!(s.body.contains("isFlagEnabled"));
        }
    }

    #[test]
    fn without_a_plan_or_a_description_the_sheet_says_what_is_missing() {
        let bare = "# T1\n\n## Description\n\n## Plan\n\n## Decisions\n";
        assert_eq!(summary(bare, Status::Todo).body, "Nothing written yet.");
        assert_eq!(summary(bare, Status::Planned).body, "Nothing written yet.");
        let only_description = "## Description\n\nWhat to do.\n\n## Plan\n";
        let s = summary(only_description, Status::Planned);
        assert!(s.label.contains("no plan written") && s.body == "What to do.");
        assert_eq!(summary("no headings at all", Status::Todo).body, "Nothing written yet.");
    }

    #[test]
    fn layout_wraps_to_the_width_keeps_a_hanging_indent_and_collapses_blank_lines() {
        let lines = layout("### Approach\n\n\n\n- a long bullet that has to wrap onto another line\n  - nested\n1. numbered item that wraps around too", 20);
        let text: Vec<&str> = lines.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(lines[0], (Kind::Heading, "Approach".to_string()));
        assert_eq!(lines[1].0, Kind::Blank);
        assert_eq!(lines[2].0, Kind::Text);
        assert_eq!(lines.iter().filter(|(k, _)| *k == Kind::Blank).count(), 1);
        assert!(text.iter().all(|l| l.chars().count() <= 20), "{text:?}");
        assert!(text.contains(&"- a long bullet that"), "{text:?}");
        assert!(text.iter().any(|l| l.starts_with("  has to wrap")), "continuation lines hang under the text: {text:?}");
        assert!(text.contains(&"  - nested"));
    }

    #[test]
    fn a_word_longer_than_the_line_is_broken_rather_than_lost() {
        let lines = layout(&"x".repeat(30), 10);
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|(_, l)| l.chars().count() <= 10));
        assert_eq!(lines.iter().map(|(_, l)| l.as_str()).collect::<String>(), "x".repeat(30));
    }
}
