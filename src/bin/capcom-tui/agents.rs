//! The agents Herdr is running, for the panel left of the story board: what Herdr reports live
//! (name, state, topic) set against the stories capcom keeps, so each agent sits under its story.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Working,
    Idle,
    Blocked,
    Done,
    Unknown,
}

impl Status {
    fn parse(text: &str) -> Status {
        match text {
            "working" => Status::Working,
            "idle" => Status::Idle,
            "blocked" => Status::Blocked,
            "done" => Status::Done,
            _ => Status::Unknown,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Status::Working => "working",
            Status::Idle => "idle",
            Status::Blocked => "needs you",
            Status::Done => "done",
            Status::Unknown => "unknown",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Status::Working => "◐",
            Status::Idle => "○",
            Status::Blocked => "!",
            Status::Done => "✓",
            Status::Unknown => "·",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInfo {
    pub pane: String,
    pub name: Option<String>,
    pub status: Status,
    /// What the agent says it is doing (its terminal title, without the spinner).
    pub topic: String,
    pub workspace: String,
    pub cwd: String,
}

/// The agents in `herdr agent list`, each with its workspace's label from `herdr workspace list`.
pub fn parse(agents: &Value, workspaces: &Value) -> Vec<AgentInfo> {
    let label_of = |id: &str| {
        workspaces["result"]["workspaces"]
            .as_array()
            .and_then(|all| all.iter().find(|w| w["workspace_id"] == id))
            .and_then(|w| w["label"].as_str())
            .unwrap_or(id)
            .to_string()
    };
    let text = |a: &Value, key: &str| a[key].as_str().unwrap_or_default().to_string();
    agents["result"]["agents"]
        .as_array()
        .map(|all| {
            all.iter()
                .filter_map(|a| {
                    Some(AgentInfo {
                        pane: a["pane_id"].as_str()?.to_string(),
                        name: a["name"].as_str().map(str::to_string),
                        status: Status::parse(a["agent_status"].as_str().unwrap_or_default()),
                        topic: text(a, "terminal_title_stripped"),
                        workspace: label_of(a["workspace_id"].as_str().unwrap_or_default()),
                        cwd: text(a, "cwd"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One agent in the panel, with the words to show for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub index: usize,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub label: String,
    /// A story of the board, not just a Herdr workspace.
    pub story: bool,
    pub entries: Vec<Entry>,
}

/// The story an agent was named for (`proj-1-t2-impl` belongs to `PROJ-1`), the task and its role.
fn named_for<'a>(name: &str, stories: &'a [(String, String)]) -> Option<(&'a (String, String), String)> {
    stories.iter().find_map(|s| {
        let rest = name.strip_prefix(&format!("{}-", s.0.to_lowercase()))?;
        let mut parts = rest.splitn(2, '-');
        let first = parts.next()?;
        let is_task = first.strip_prefix('t').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        let label = match (is_task, parts.next()) {
            (true, Some(role)) => format!("{} {}", first.to_uppercase(), role.replace('-', " ")),
            (true, None) => first.to_uppercase(),
            (false, _) => rest.replace('-', " "),
        };
        Some((s, label))
    })
}

fn task_number(label: &str) -> u32 {
    label.strip_prefix('T').and_then(|r| r.split(' ').next()).and_then(|n| n.parse().ok()).unwrap_or(u32::MAX)
}

/// The panel's groups: a story's agents under the story, the rest under their Herdr workspace.
/// Stories come first, in board order; inside a story the tasks run in order.
pub fn groups(agents: &[AgentInfo], stories: &[(String, String)]) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for (i, agent) in agents.iter().enumerate() {
        let named = agent.name.as_deref().and_then(|n| named_for(n, stories));
        let (label, story, entry_label) = match &named {
            Some((s, label)) => (format!("{} {}", s.0, s.1), true, label.clone()),
            None => (agent.workspace.clone(), false, if agent.topic.is_empty() { "agent".to_string() } else { agent.topic.clone() }),
        };
        let entry = Entry { index: i, label: entry_label };
        match out.iter_mut().find(|g| g.label == label && g.story == story) {
            Some(g) => g.entries.push(entry),
            None => out.push(Group { label, story, entries: vec![entry] }),
        }
    }
    for g in &mut out {
        g.entries.sort_by(|a, b| task_number(&a.label).cmp(&task_number(&b.label)).then_with(|| a.label.cmp(&b.label)));
    }
    let rank = |g: &Group| match stories.iter().position(|s| g.label.split(' ').next() == Some(s.0.as_str())) {
        Some(p) if g.story => (0, p),
        _ => (1, 0),
    };
    out.sort_by_key(rank);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn agent(pane: &str, name: Option<&str>, status: Status, topic: &str, workspace: &str) -> AgentInfo {
        AgentInfo { pane: pane.into(), name: name.map(str::to_string), status, topic: topic.into(), workspace: workspace.into(), cwd: String::new() }
    }

    #[test]
    fn herdrs_agent_list_is_read_with_each_workspaces_label() {
        let agents = json!({"result": {"agents": [
            {"pane_id": "w1:p1", "agent_status": "working", "workspace_id": "w1", "terminal_title_stripped": "Fix the thing", "cwd": "/w"},
            {"pane_id": "w2:p3", "name": "proj-1-t2-impl", "agent_status": "blocked", "workspace_id": "w2"},
            {"agent_status": "idle"},
        ]}});
        let workspaces = json!({"result": {"workspaces": [{"workspace_id": "w2", "label": "PROJ-1 - Notices"}]}});
        let got = parse(&agents, &workspaces);
        assert_eq!(got.len(), 2, "an entry with no pane is skipped");
        assert_eq!((got[0].status, got[0].workspace.as_str(), got[0].topic.as_str()), (Status::Working, "w1", "Fix the thing"));
        assert_eq!((got[1].status, got[1].workspace.as_str(), got[1].name.as_deref()), (Status::Blocked, "PROJ-1 - Notices", Some("proj-1-t2-impl")));
        assert!(parse(&json!({}), &json!({})).is_empty());
    }

    #[test]
    fn agents_sit_under_their_story_by_task_and_the_rest_under_their_workspace() {
        let stories = vec![("PROJ-1".to_string(), "Notices".to_string()), ("PROJ-2".to_string(), "Other".to_string())];
        let list = vec![
            agent("a", None, Status::Idle, "AWS alert", "scratch"),
            agent("b", Some("proj-2-t1-plan"), Status::Idle, "", "x"),
            agent("c", Some("proj-1-t10-impl"), Status::Working, "", "x"),
            agent("d", Some("proj-1-t2-impl"), Status::Done, "", "x"),
            agent("e", Some("proj-1-review"), Status::Idle, "", "x"),
        ];
        let g = groups(&list, &stories);
        let shape: Vec<(String, Vec<String>)> = g.iter().map(|g| (g.label.clone(), g.entries.iter().map(|e| e.label.clone()).collect())).collect();
        assert_eq!(
            shape,
            vec![
                ("PROJ-1 Notices".to_string(), vec!["T2 impl".to_string(), "T10 impl".to_string(), "review".to_string()]),
                ("PROJ-2 Other".to_string(), vec!["T1 plan".to_string()]),
                ("scratch".to_string(), vec!["AWS alert".to_string()]),
            ]
        );
        assert!(g[0].story && !g[2].story);
    }
}
