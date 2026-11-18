//! Agents talk to each other by Pokemon name. The Herdr agents panel keeps each live agent's Pokemon
//! in `capcom.persona` of the app's `shared.json` (terminal address to name, colour and portrait);
//! this reads it, so `capcom tell krabby "..."` can find the terminal and hand the text to `hc send`.
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

pub struct Agent {
    pub terminal: String,
    pub name: String,
}

pub fn shared_path() -> Option<PathBuf> {
    let home = match std::env::var_os("HERDR_APP_HOME") {
        Some(h) => PathBuf::from(h),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".config/herdr-app"),
    };
    Some(home.join("shared.json"))
}

fn key(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Every agent with a Pokemon name in the shared state's `capcom.persona`.
pub fn agents(shared: &Value) -> Vec<Agent> {
    let Some(persona) = shared.get("capcom.persona").and_then(Value::as_object) else { return Vec::new() };
    let mut found: Vec<Agent> = persona
        .iter()
        .filter_map(|(terminal, p)| {
            let name = p.get("label")?.as_str()?.trim();
            (!name.is_empty()).then(|| Agent { terminal: terminal.clone(), name: name.to_string() })
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// Who a command line names: a Pokemon (`krabby`), a terminal (`t5h`), or both (`Krabby (t5h)`, what the
/// panel's name button copies). A Pokemon is released when its agent ends and can be given to another agent,
/// so with both the terminal wins and the name only has to still match it.
#[derive(Debug, PartialEq)]
pub enum Spec {
    Name(String),
    Terminal(String),
    Both { name: String, terminal: String },
}

fn is_terminal(s: &str) -> bool {
    let id = s.rsplit('/').next().unwrap_or(s);
    id.len() > 1 && id.starts_with('t') && id[1..].starts_with(|c: char| c.is_ascii_digit()) && id.chars().all(|c| c.is_ascii_alphanumeric())
}

pub fn parse(spec: &str) -> Spec {
    let spec = spec.trim();
    if let Some(open) = spec.rfind('(') {
        let inner = spec[open + 1..].trim_end_matches(')').trim();
        if spec.ends_with(')') && is_terminal(inner) && !spec[..open].trim().is_empty() {
            return Spec::Both { name: spec[..open].trim().to_string(), terminal: inner.to_string() };
        }
    }
    if is_terminal(spec) { Spec::Terminal(spec.to_string()) } else { Spec::Name(spec.to_string()) }
}

pub struct Target {
    pub terminal: String,
    /// Its Pokemon, when the agents panel knows it.
    pub name: Option<String>,
}

pub fn resolve(agents: &[Agent], spec: &str) -> Result<Target> {
    let by_terminal = |t: &str| agents.iter().find(|a| a.terminal == t);
    match parse(spec) {
        Spec::Terminal(t) => Ok(Target { name: by_terminal(&t).map(|a| a.name.clone()), terminal: t }),
        Spec::Both { name, terminal } => match by_terminal(&terminal) {
            Some(a) if key(&a.name) != key(&name) => bail!("{terminal} is now {}, not {name}: that Pokemon went to another agent", a.name),
            found => Ok(Target { name: found.map(|a| a.name.clone()), terminal }),
        },
        Spec::Name(name) => {
            let want = key(&name);
            if want.is_empty() {
                bail!("give a Pokemon name or a terminal like t5h");
            }
            agents.iter().find(|a| key(&a.name) == want).map(|a| Target { terminal: a.terminal.clone(), name: Some(a.name.clone()) }).ok_or_else(|| {
                let known: Vec<String> = agents.iter().map(|a| format!("{} ({})", a.name, a.terminal)).collect();
                if known.is_empty() {
                    anyhow!("no agent is called {name}: the Agents panel is not running, so no Pokemon are assigned; use a terminal like t5h")
                } else {
                    anyhow!("no agent is called {name}; live agents: {}", known.join(", "))
                }
            })
        }
    }
}

pub fn load() -> Result<Vec<Agent>> {
    let path = shared_path().ok_or_else(|| anyhow!("no home folder: set HERDR_APP_HOME"))?;
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(agents(&serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?))
}

/// The text as the receiving agent sees it: who it is from and how to answer (by terminal, which outlives a name).
pub fn message(from_name: Option<&str>, from_terminal: Option<&str>, text: &str) -> String {
    match (from_name, from_terminal) {
        (Some(n), Some(t)) => format!("Message from {n} ({t}) (answer with: capcom tell {t} \"<text>\"): {text}"),
        (None, Some(t)) => format!("Message from {t} (answer with: capcom tell {t} \"<text>\"): {text}"),
        _ => format!("Message from the user: {text}"),
    }
}

/// Sends `text` to the agent `to` names, from the agent in terminal `from_terminal` (if any). Returns who it
/// went to (name, terminal) and who it was from.
pub fn tell(to: &str, text: &str, from_terminal: Option<&str>) -> Result<(Option<String>, String, String)> {
    let all = load().unwrap_or_default();
    let target = resolve(&all, to)?;
    let from_name = from_terminal.and_then(|t| all.iter().find(|a| a.terminal == t)).map(|a| a.name.as_str());
    let out = Command::new("hc")
        .args(["send", &target.terminal, &message(from_name, from_terminal, text)])
        .output()
        .context("running `hc send` (is hc on PATH?)")?;
    if !out.status.success() {
        bail!("hc send to {} failed: {}", target.terminal, String::from_utf8_lossy(&out.stderr).trim());
    }
    let from = from_name.or(from_terminal).unwrap_or("the user").to_string();
    Ok((target.name, target.terminal, from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Vec<Agent> {
        agents(&json!({ "capcom.persona": {
            "t5h": { "label": "Krabby", "color": "#4fcf8e" },
            "t5m": { "label": "Mr. Mime" },
            "t5r": { "label": "" },
        }}))
    }

    #[test]
    fn finds_an_agent_by_name_ignoring_case_and_punctuation() {
        let all = sample();
        assert_eq!(resolve(&all, "krabby").unwrap().terminal, "t5h");
        assert_eq!(resolve(&all, "KRABBY").unwrap().terminal, "t5h");
        assert_eq!(resolve(&all, "mr mime").unwrap().terminal, "t5m");
    }

    #[test]
    fn skips_agents_without_a_name() {
        assert_eq!(sample().len(), 2);
    }

    #[test]
    fn parses_a_name_a_terminal_or_both() {
        assert_eq!(parse("krabby"), Spec::Name("krabby".into()));
        assert_eq!(parse("t5h"), Spec::Terminal("t5h".into()));
        assert_eq!(parse("vps/t7"), Spec::Terminal("vps/t7".into()));
        assert_eq!(parse("Krabby (t5h)"), Spec::Both { name: "Krabby".into(), terminal: "t5h".into() });
        // Names that start with t are still names.
        assert_eq!(parse("Tauros"), Spec::Name("Tauros".into()));
        assert_eq!(parse("Mr. Mime (draft)"), Spec::Name("Mr. Mime (draft)".into()));
    }

    #[test]
    fn a_terminal_resolves_without_the_panel() {
        let t = resolve(&[], "t9z").unwrap();
        assert_eq!((t.terminal.as_str(), t.name), ("t9z", None));
    }

    #[test]
    fn a_terminal_carries_its_name_when_known() {
        assert_eq!(resolve(&sample(), "t5h").unwrap().name.as_deref(), Some("Krabby"));
    }

    #[test]
    fn name_and_terminal_must_still_agree() {
        assert_eq!(resolve(&sample(), "Krabby (t5h)").unwrap().terminal, "t5h");
        let err = resolve(&sample(), "Koffing (t5h)").err().unwrap().to_string();
        assert!(err.contains("t5h is now Krabby"), "{err}");
    }

    #[test]
    fn an_unknown_name_lists_who_is_live() {
        let err = resolve(&sample(), "Pikachu").err().unwrap().to_string();
        assert!(err.contains("Krabby (t5h)") && err.contains("Mr. Mime (t5m)"), "{err}");
    }

    #[test]
    fn no_agents_says_the_panel_may_not_be_running() {
        let err = resolve(&[], "Krabby").err().unwrap().to_string();
        assert!(err.contains("Agents panel"), "{err}");
    }

    #[test]
    fn the_message_says_who_it_is_from_and_how_to_answer_by_terminal() {
        assert_eq!(
            message(Some("Mr. Mime"), Some("t5m"), "hi"),
            "Message from Mr. Mime (t5m) (answer with: capcom tell t5m \"<text>\"): hi"
        );
        assert_eq!(message(None, Some("t5m"), "hi"), "Message from t5m (answer with: capcom tell t5m \"<text>\"): hi");
        assert_eq!(message(None, None, "hi"), "Message from the user: hi");
    }
}
