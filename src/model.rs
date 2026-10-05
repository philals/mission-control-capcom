use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Todo,
    Planning,
    Planned,
    Implementing,
    Done,
    Dropped,
}

impl Status {
    const ALL: [Status; 6] = [
        Status::Todo,
        Status::Planning,
        Status::Planned,
        Status::Implementing,
        Status::Done,
        Status::Dropped,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::Planning => "planning",
            Status::Planned => "planned",
            Status::Implementing => "implementing",
            Status::Done => "done",
            Status::Dropped => "dropped",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        Status::ALL.into_iter().find(|x| x.as_str() == s)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Status::Done | Status::Dropped)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryStatus {
    #[default]
    InProgress,
    InReview,
    Done,
}

impl StoryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            StoryStatus::InProgress => "in_progress",
            StoryStatus::InReview => "in_review",
            StoryStatus::Done => "done",
        }
    }

    pub fn parse(s: &str) -> Option<StoryStatus> {
        [StoryStatus::InProgress, StoryStatus::InReview, StoryStatus::Done]
            .into_iter()
            .find(|x| x.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskType {
    Pr,
    Spike,
    Decision,
}

impl TaskType {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskType::Pr => "pr",
            TaskType::Spike => "spike",
            TaskType::Decision => "decision",
        }
    }

    pub fn parse(s: &str) -> Option<TaskType> {
        [TaskType::Pr, TaskType::Spike, TaskType::Decision]
            .into_iter()
            .find(|x| x.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrState {
    Draft,
    Ready,
    Merged,
    Closed,
}

impl PrState {
    pub fn as_str(self) -> &'static str {
        match self {
            PrState::Draft => "draft",
            PrState::Ready => "ready",
            PrState::Merged => "merged",
            PrState::Closed => "closed",
        }
    }

    pub fn parse(s: &str) -> Option<PrState> {
        [PrState::Draft, PrState::Ready, PrState::Merged, PrState::Closed]
            .into_iter()
            .find(|x| x.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Board {
    pub schema_version: u32,
    pub story: Story,
    pub tasks: Vec<Task>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Story {
    pub key: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira_url: Option<String>,
    #[serde(default)]
    pub status: StoryStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub kind: TaskType,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<Blocked>,
    pub depends_on: Vec<String>,
    pub file: String,
    pub repos: Vec<String>,
    pub prs: Vec<Pr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira_subtask: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<Agent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blocked {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pr {
    pub repo: String,
    pub url: String,
    pub state: PrState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Agent {
    pub pane: String,
    pub skill: String,
    pub started_at: String,
}

impl Board {
    pub fn new(key: &str, title: &str, jira_url: Option<&str>) -> Board {
        Board {
            schema_version: SCHEMA_VERSION,
            story: Story {
                key: key.to_string(),
                title: title.to_string(),
                jira_url: jira_url.map(str::to_string),
                status: StoryStatus::InProgress,
            },
            tasks: Vec::new(),
        }
    }

    pub fn task(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }
}

impl Task {
    pub fn new(
        id: &str,
        title: &str,
        kind: TaskType,
        depends_on: Vec<String>,
        repos: Vec<String>,
        file: String,
    ) -> Task {
        Task {
            id: id.to_string(),
            title: title.to_string(),
            kind,
            status: Status::Todo,
            blocked: None,
            depends_on,
            file,
            repos,
            prs: Vec::new(),
            jira_subtask: None,
            agent: None,
        }
    }
}
