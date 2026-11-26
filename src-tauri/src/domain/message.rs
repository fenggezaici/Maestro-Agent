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
    /// Provider reasoning/thinking blocks to replay verbatim (Anthropic
    /// `thinking`/`redacted_thinking` content blocks). Opaque JSON array —
    /// OpenAI wire format ignores it entirely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<serde_json::Value>,
    /// True when a tool message carries a failure result (Anthropic `is_error`).
    #[serde(default)]
    pub is_error: bool,
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
            thinking: None,
            is_error: false,
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

/// Boundary validation for messages arriving over IPC. Returns the trailing
/// user turn as `(prompt, images)` — history is rebuilt backend-side from the
/// database, so only the last message's content crosses this boundary.
pub fn validate_messages(messages: &[ChatMessage]) -> AppResult<(String, Vec<String>)> {
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
        // images 此前完全不校验：单条 ≤8 张、总量 ≤24MB（provider 侧另有
        // 单张 3.5MB 上限；这里先挡住明显失控的 payload 才进网络层）。
        || messages.iter().any(|message| {
            message.images.as_ref().is_some_and(|images| images.len() > 8)
        })
        || messages
            .iter()
            .flat_map(|message| message.images.iter().flatten())
            .map(|url| url.len())
            .sum::<usize>()
            > 25_165_824
    {
        return Err("聊天消息为空、过大、包含不支持的 role（仅 user/assistant），或图片超限".into());
    }
    let last = messages.last().ok_or("缺少用户消息")?;
    if last.role != "user" || last.content.trim().is_empty() || last.content.trim().len() > 8192 {
        return Err("最后一条必须是 1–8192 UTF-8 bytes 的用户消息".into());
    }
    // 图片随用户轮次上行：此前校验后只返回文本，附件在交接处被静默丢弃——
    // 视觉门控放行的 dataURL 必须真正抵达 wire 层才有意义。
    Ok((last.content.trim().to_string(), last.images.clone().unwrap_or_default()))
}
