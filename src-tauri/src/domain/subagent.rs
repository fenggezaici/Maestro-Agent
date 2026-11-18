use serde::Serialize;

use crate::domain::ToolCallView;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl SubagentStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Self {
        match value {
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Running,
        }
    }

    /// Short Chinese label used in the parent tool result header.
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "执行中",
            Self::Completed => "已完成",
            Self::Failed => "失败",
            Self::Cancelled => "已取消",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentTokenKind {
    Content,
    Reasoning,
}

/// One child research run. Live events update it in memory; start/finish are
/// also persisted so the drill-down survives restarts and session switches.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentView {
    pub id: String,
    pub run_id: i64,
    pub session_id: i64,
    pub agent: String,
    pub description: String,
    pub prompt: String,
    pub model: Option<String>,
    pub status: SubagentStatus,
    pub summary: String,
    pub error: Option<String>,
    pub content: String,
    pub reasoning: String,
    pub tool_count: usize,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub first_token_ms: Option<u64>,
    pub elapsed_ms: u64,
    /// Σ 每次 LLM 调用的 decode 窗口（首个任意 delta → 末次 delta），仅计入
    /// 上报了 output usage 的调用；与 DSH decode 口径一致。
    #[serde(default)]
    pub decode_ms: u64,
    /// 与 decode_ms 成对的 output tokens —— 只有贡献了 decode 窗口的调用
    /// 的 output usage 才计入；tok/s = decode_tokens / decode_ms。
    #[serde(default)]
    pub decode_tokens: u64,
    pub last_input_tokens: Option<u64>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub tools: Vec<ToolCallView>,
}
