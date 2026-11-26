use serde::{Deserialize, Serialize};

use crate::domain::{compaction::ContextCompaction, metrics::RunMetrics, plan::PlanCard};

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

/// One ordered piece of a run's timeline, mirroring ZCode's message parts:
/// replaying them in `seq` order reproduces the live interleaving exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPart {
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    #[serde(default)]
    pub started_at: i64,
    #[serde(default)]
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningSegment {
    pub text: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunMessage {
    pub run_id: i64,
    pub session_id: i64,
    pub seq: i64,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub tool_calls: Option<serde_json::Value>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// Provider thinking blocks replayed verbatim (Anthropic); NULL on disk
    /// for OpenAI providers and for rows written before v16.
    #[serde(default)]
    pub thinking: Option<serde_json::Value>,
    /// Tool message carried a failure result (Anthropic `is_error`).
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub session: SessionView,
    pub runs: Vec<RunView>,
    pub plans: Vec<PlanCard>,
    pub metrics: Vec<RunMetrics>,
    pub tools: Vec<ToolCallView>,
    #[serde(default)]
    pub compactions: Vec<ContextCompaction>,
    #[serde(default)]
    pub run_messages: Vec<RunMessage>,
    /// Persisted reasoning bursts per run id; the live timeline is memory-only.
    #[serde(default)]
    pub reasoning: std::collections::HashMap<i64, Vec<ReasoningSegment>>,
    /// Ordered timeline parts per run (reasoning / tool / content), persisted so
    /// replay keeps the exact interleaving instead of a heuristic ordering.
    #[serde(default)]
    pub parts: std::collections::HashMap<i64, Vec<RunPart>>,
    /// Data-URL images attached to a run's user turn, keyed by run id. Lives
    /// outside `RunView` so streaming `Status` events never carry megabytes of
    /// base64 — the map is hydrated once with the session detail.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub run_images: std::collections::HashMap<i64, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallView {
    pub run_id: i64,
    pub tool: String,
    pub detail: String,
    pub allowed: bool,
    /// Set for `spawn_subagent` rows so the UI can open the matching trace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_id: Option<String>,
    /// Live child-trace rows carry the lifecycle flags the UI renders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
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
