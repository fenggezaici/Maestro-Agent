//! Conversation and tool-call structures shared by the LLM adapters and the
//! agent service. Pure data: no transport, no persistence.
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    /// Rust-internal tool plumbing; frontend never sends these fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallSpec>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Data-URL images attached to a user message (frontend sends `data:*;base64,*`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ToolCallSpec {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

impl ChatMessage {
    /// Plain text message with no tool plumbing.
    pub fn text(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            images: None,
        }
    }
}

/// One tool offered to the model: built-in, skill loader, or MCP-backed.
/// `parameters` is always a standard JSON Schema object; each provider
/// adapter translates it into its own wire shape (OpenAI `function.parameters`,
/// Anthropic `input_schema`).
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

use crate::domain::error::AppResult;

/// Boundary validation for messages arriving over IPC.
pub fn validate_messages(messages: &[ChatMessage]) -> AppResult<String> {
    if messages.is_empty()
        || messages.len() > 129
        || messages.iter().any(|message| {
            !matches!(message.role.as_str(), "user" | "assistant") || message.content.len() > 65536
        })
        || messages
            .iter()
            .map(|message| message.content.len())
            .sum::<usize>()
            > 262144
    {
        return Err("聊天消息为空、过大，或包含不支持的 role（仅 user/assistant）".into());
    }
    let last = messages.last().ok_or("缺少用户消息")?;
    if last.role != "user" || last.content.trim().is_empty() || last.content.trim().len() > 8192 {
        return Err("最后一条必须是 1–8192 UTF-8 bytes 的用户消息".into());
    }
    Ok(last.content.trim().to_string())
}
