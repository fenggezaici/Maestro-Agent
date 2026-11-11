use serde::Serialize;

use crate::domain::{metrics::RunMetrics, plan::PlanCard};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Plan,
    Code,
    Confirm,
    Auto,
    // Historical records only; new UI selections use Code.
    Execute,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Code => "code",
            Self::Confirm => "confirm",
            Self::Auto => "auto",
            Self::Execute => "execute",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Completed,
    Cancelled,
    Failed,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "completed" => Self::Completed,
            "cancelled" => Self::Cancelled,
            "running" => Self::Running,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunView {
    pub id: i64,
    pub session_id: i64,
    pub prompt: String,
    pub mode: Mode,
    pub status: RunStatus,
    pub output: String,
    pub error: Option<String>,
    pub created_at: i64,
}

/// Unix seconds for a freshly created run (wall clock, UI relative time only).
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub ready: bool,
    pub mode: Mode,
    pub active_run: Option<RunView>,
    pub last_run: Option<RunView>,
    pub sequence: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub id: i64,
    pub title: String,
    pub updated_at: i64,
    pub run_count: i64,
    pub project_id: i64,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub id: i64,
    pub name: String,
    pub path: Option<String>,
    pub archived: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub session: SessionView,
    pub runs: Vec<RunView>,
    pub plans: Vec<PlanCard>,
    pub metrics: Vec<RunMetrics>,
    pub tools: Vec<ToolCallView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallView {
    pub run_id: i64,
    pub tool: String,
    pub detail: String,
    pub allowed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_mode_cannot_be_deserialized_as_execute() {
        assert_eq!(
            serde_json::from_str::<Mode>("\"execute\"").unwrap(),
            Mode::Execute
        );
        assert!(serde_json::from_str::<Mode>("\"unrestricted\"").is_err());
        assert!(serde_json::from_str::<Mode>("\"Execute\"").is_err());
    }
}
