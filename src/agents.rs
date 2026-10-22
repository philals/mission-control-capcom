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
    /// The Claude Code session running in it, which is how a parent finds its children.
    pub session: Option<String>,
}

const SHORT_NAME_WORDS: usize = 3;
const SHORT_NAME_CHARS: usize = 28;
const TRAILING_FILLER: [&str; 9] = ["a", "an", "the", "to", "of", "for", "and", "or", "in"];

/// A very short name for a story: its first few words, without trailing filler words.
pub fn short_name(title: &str) -> String {
    let mut words: Vec<&str> = Vec::new();
    let mut length = 0;
    for word in title.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric())).filter(|w| !w.is_empty()) {
        let added = length + word.chars().count() + usize::from(!words.is_empty());
        if words.len() == SHORT_NAME_WORDS || added > SHORT_NAME_CHARS {
            break;
        }
        words.push(word);
        length = added;
    }
    while words.len() > 1 && TRAILING_FILLER.contains(&words[words.len() - 1].to_lowercase().as_str()) {
        words.pop();
    }
    words.join(" ")
}

/// Whether an agent is waiting on you: Herdr says it is blocked, or its note says so while it is not
/// at work (a note left from before it carried on does not count).
pub fn needs_you(status: Status, note: Option<&crate::notes::Note>) -> bool {
    status == Status::Blocked || (status != Status::Working && note.is_some_and(|n| n.needs_you))
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
                        session: a["agent_session"]["value"].as_str().map(str::to_string),
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
    /// 0 for an agent on its own, 1 for one it started, and so on.
    pub depth: usize,
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

/// For each agent, the live agent that started it: the one whose session made the pane this one runs in.
pub fn parents(agents: &[AgentInfo], spawns: &[(String, String)]) -> Vec<Option<usize>> {
    agents
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let session = &spawns.iter().find(|(pane, _)| *pane == a.pane)?.1;
            agents.iter().position(|p| p.session.as_ref() == Some(session)).filter(|&p| p != i)
        })
        .collect()
}

/// The panel's groups: a story's agents under the story, the rest under their Herdr workspace.
/// Stories come first, in board order; inside a story the tasks run in order. An agent started by
/// another follows it, one level in.
pub fn groups(agents: &[AgentInfo], stories: &[(String, String)], parents: &[Option<usize>]) -> Vec<Group> {
    let label_of = |agent: &AgentInfo| match agent.name.as_deref().and_then(|n| named_for(n, stories)) {
        Some((_, label)) => label,
        None if agent.topic.is_empty() => agent.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| "agent".to_string()),
        None => agent.topic.clone(),
    };
    let top = |i: usize| parents.get(i).copied().flatten().is_none();
    let mut out: Vec<Group> = Vec::new();
    for (i, agent) in agents.iter().enumerate().filter(|(i, _)| top(*i)) {
        let named = agent.name.as_deref().and_then(|n| named_for(n, stories));
        let (label, story) = match &named {
            Some((s, _)) => (format!("{} {}", s.0, s.1), true),
            None => (agent.workspace.clone(), false),
        };
        let entry = Entry { index: i, label: label_of(agent), depth: 0 };
        match out.iter_mut().find(|g| g.label == label && g.story == story) {
            Some(g) => g.entries.push(entry),
            None => out.push(Group { label, story, entries: vec![entry] }),
        }
    }
    for g in &mut out {
        g.entries.sort_by(|a, b| task_number(&a.label).cmp(&task_number(&b.label)).then_with(|| a.label.cmp(&b.label)));
        let mut tree = Vec::with_capacity(g.entries.len());
        for entry in std::mem::take(&mut g.entries) {
            let at = entry.index;
            tree.push(entry);
            descendants(agents, parents, &label_of, at, 1, &mut tree);
        }
        g.entries = tree;
    }
    let rank = |g: &Group| match stories.iter().position(|s| g.label.split(' ').next() == Some(s.0.as_str())) {
        Some(p) if g.story => (0, p),
        _ => (1, 0),
    };
    out.sort_by_key(rank);
    out
}

fn descendants(agents: &[AgentInfo], parents: &[Option<usize>], label_of: &dyn Fn(&AgentInfo) -> String, of: usize, depth: usize, out: &mut Vec<Entry>) {
    let mut kids: Vec<usize> = (0..agents.len()).filter(|&i| parents.get(i).copied().flatten() == Some(of)).collect();
    kids.sort_by_key(|&i| (task_number(&label_of(&agents[i])), label_of(&agents[i])));
    for kid in kids {
        if depth > 8 || out.iter().any(|e| e.index == kid) {
            continue;
        }
        out.push(Entry { index: kid, label: label_of(&agents[kid]), depth });
        descendants(agents, parents, label_of, kid, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn agent(pane: &str, name: Option<&str>, status: Status, topic: &str, workspace: &str) -> AgentInfo {
        AgentInfo { pane: pane.into(), name: name.map(str::to_string), status, topic: topic.into(), workspace: workspace.into(), cwd: String::new(), session: None }
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
        let g = groups(&list, &stories, &vec![None; list.len()]);
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

    #[test]
    fn an_agent_follows_the_one_that_started_it_one_level_in() {
        let stories: Vec<(String, String)> = vec![];
        let mut list = vec![
            agent("w3E:p1", None, Status::Idle, "Edit UX feedback", "Edit UX"),
            agent("w3N:p1", None, Status::Working, "hold icon", "hold-icon"),
            agent("w4:p1", None, Status::Idle, "grandchild", "g"),
            agent("w9:p1", None, Status::Idle, "unrelated", "u"),
        ];
        list[0].session = Some("parent".into());
        list[1].session = Some("child".into());
        let spawns = vec![("w3N:p1".to_string(), "parent".to_string()), ("w4:p1".to_string(), "child".to_string())];
        let p = parents(&list, &spawns);
        assert_eq!(p, vec![None, Some(0), Some(1), None]);
        let g = groups(&list, &stories, &p);
        let shape: Vec<(String, Vec<(String, usize)>)> = g.iter().map(|g| (g.label.clone(), g.entries.iter().map(|e| (e.label.clone(), e.depth)).collect())).collect();
        assert_eq!(
            shape,
            vec![
                ("Edit UX".to_string(), vec![("Edit UX feedback".to_string(), 0), ("hold icon".to_string(), 1), ("grandchild".to_string(), 2)]),
                ("u".to_string(), vec![("unrelated".to_string(), 0)]),
            ],
            "the child is no group of its own"
        );
        let mut loop_ = vec![agent("a:p1", None, Status::Idle, "a", "a"), agent("b:p1", None, Status::Idle, "b", "b")];
        loop_[0].session = Some("sa".into());
        loop_[1].session = Some("sb".into());
        let both = parents(&loop_, &[("a:p1".into(), "sb".into()), ("b:p1".into(), "sa".into())]);
        assert_eq!(groups(&loop_, &stories, &both).len(), 0, "a cycle has no top, and nothing hangs");
    }
}
