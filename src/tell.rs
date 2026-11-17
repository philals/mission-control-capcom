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

/// The agent called `name`, ignoring case, spaces and punctuation ("mr mime" finds "Mr. Mime").
pub fn resolve<'a>(agents: &'a [Agent], name: &str) -> Result<&'a Agent> {
    let want = key(name);
    if want.is_empty() {
        bail!("give a Pokemon name");
    }
    agents.iter().find(|a| key(&a.name) == want).ok_or_else(|| {
        let known: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
        if known.is_empty() {
            anyhow!("no agent is called {name}: the Agents panel is not running, so no Pokemon are assigned")
        } else {
            anyhow!("no agent is called {name}; live agents: {}", known.join(", "))
        }
    })
}

pub fn load() -> Result<Vec<Agent>> {
    let path = shared_path().ok_or_else(|| anyhow!("no home folder: set HERDR_APP_HOME"))?;
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(agents(&serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?))
}

/// The text as the receiving agent sees it: who it is from and how to answer.
pub fn message(from: &str, text: &str) -> String {
    format!("Message from {from} (answer with: capcom tell {} \"<text>\"): {text}", key(from))
}

/// Sends `text` to the agent called `to`, from the agent in terminal `from_terminal` (if any).
pub fn tell(to: &str, text: &str, from_terminal: Option<&str>) -> Result<(String, String, String)> {
    let all = load()?;
    let target = resolve(&all, to)?;
    let from = from_terminal
        .and_then(|t| all.iter().find(|a| a.terminal == t))
        .map(|a| a.name.clone())
        .unwrap_or_else(|| from_terminal.unwrap_or("the user").to_string());
    let out = Command::new("hc")
        .args(["send", &target.terminal, &message(&from, text)])
        .output()
        .context("running `hc send` (is hc on PATH?)")?;
    if !out.status.success() {
        bail!("hc send to {} ({}) failed: {}", target.name, target.terminal, String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok((target.name.clone(), target.terminal.clone(), from))
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
    fn an_unknown_name_lists_who_is_live() {
        let err = resolve(&sample(), "Pikachu").err().unwrap().to_string();
        assert!(err.contains("Krabby") && err.contains("Mr. Mime"), "{err}");
    }

    #[test]
    fn no_agents_says_the_panel_may_not_be_running() {
        let err = resolve(&[], "Krabby").err().unwrap().to_string();
        assert!(err.contains("Agents panel"), "{err}");
    }

    #[test]
    fn the_message_says_who_it_is_from_and_how_to_answer() {
        assert_eq!(
            message("Mr. Mime", "hi"),
            "Message from Mr. Mime (answer with: capcom tell mrmime \"<text>\"): hi"
        );
    }
}
