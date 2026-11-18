//! Anthropic Messages API provider: `POST {base}/messages` with SSE
//! `content_block_*` events. `tool_use` blocks map to our `ToolCallSpec`;
//! `thinking_delta` maps to `Reasoning`.
use std::{collections::BTreeMap, sync::Arc};

use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

use crate::domain::{AppResult, ChatMessage, ToolCallSpec, ToolSpec};

use super::{
    http_client, identity_headers, stream_common, wire_tools, ChatConfig, Frame, FrameParser,
    LlmProvider, StreamEvent, TurnOutcome,
};

pub struct AnthropicProvider;

/// Anthropic wire format: tool_use blocks inside assistant content arrays and
/// tool_result blocks folded into following user messages.
fn anthropic_messages(messages: &[ChatMessage]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for message in messages {
        match message.role.as_str() {
            "system" => {}
            "tool" => {
                // tool_result 必须带 is_error —— 否则失败结果与成功输出在
                // 模型眼中不可区分（Anthropic wire 契约）。
                let mut block = json!({
                    "type": "tool_result",
                    "tool_use_id": message.tool_call_id.clone().unwrap_or_default(),
                    "content": message.content,
                });
                if message.is_error {
                    block["is_error"] = true.into();
                }
                match out.last_mut() {
                    Some(last)
                        if last.get("role").and_then(|r| r.as_str()) == Some("user")
                            && last.get("content").is_some_and(|c| c.is_array()) =>
                    {
                        last["content"].as_array_mut().unwrap().push(block);
                    }
                    _ => out.push(json!({ "role": "user", "content": [block] })),
                }
            }
            "assistant" if message.tool_calls.is_some() => {
                let mut content = Vec::new();
                // thinking.enabled 时 Anthropic 要求带 tool_use 的 assistant
                // 轮次原样回传其 thinking/redacted_thinking 块（含 signature），
                // 否则 400 —— 回放时 thinking 块必须先于 text/tool_use。
                if let Some(blocks) = message.thinking.as_ref().and_then(|v| v.as_array()) {
                    content.extend(blocks.iter().cloned());
                }
                if !message.content.is_empty() {
                    content.push(json!({ "type": "text", "text": message.content }));
                }
                for call in message.tool_calls.iter().flatten() {
                    content.push(json!({
                        "type": "tool_use", "id": call.id, "name": call.name,
                        "input": serde_json::from_str::<Value>(&call.arguments)
                            .unwrap_or_else(|_| json!({})),
                    }));
                }
                out.push(json!({ "role": "assistant", "content": content }));
            }
            "user" if message.images.as_ref().is_some_and(|v| !v.is_empty()) => {
                let mut content = vec![json!({ "type": "text", "text": message.content })];
                for url in message.images.iter().flatten() {
                    if let Some(rest) = url.strip_prefix("data:") {
                        if let Some((media, data)) = rest.split_once(";base64,") {
                            content.push(json!({
                                "type": "image",
                                "source": { "type": "base64", "media_type": media, "data": data },
                            }));
                        }
                    }
                }
                out.push(json!({ "role": "user", "content": content }));
            }
            role => out.push(json!({ "role": role, "content": message.content })),
        }
    }
    out
}

impl LlmProvider for AnthropicProvider {
    fn stream<'a>(
        self: Arc<Self>,
        config: &'a ChatConfig,
        messages: &'a [ChatMessage],
        tools: &'a [ToolSpec],
        cancel: &'a mut watch::Receiver<bool>,
        events: mpsc::UnboundedSender<StreamEvent>,
    ) -> BoxFuture<'a, AppResult<TurnOutcome>> {
        Box::pin(async move {
            let url = config.validate()?;
            let client = http_client()?;
            let mut payload = json!({ "model": config.model.trim(), "stream": true });
            if !tools.is_empty() {
                let mut wire = wire_tools(tools, true);
                if let Some(last) = wire.as_array_mut().and_then(|list| list.last_mut()) {
                    last["cache_control"] = json!({ "type": "ephemeral" });
                }
                payload["tools"] = wire;
            }
            // 多条 system 消息（系统策略 + 压缩摘要 + 子智能体报告）全部进入
            // `system` 字段——只取第一条会静默丢弃后续上下文。Anthropic 接受
            // 字符串或 text-block 数组；多条时用数组保留各自边界。
            let system_parts: Vec<&str> = messages
                .iter()
                .filter(|message| message.role == "system")
                .map(|message| message.content.as_str())
                .collect();
            // Prompt caching: the stable system prefix, the tool catalogue,
            // and the latest conversation tail each carry one ephemeral
            // breakpoint (Anthropic allows four). System+tools change rarely,
            // so they read from cache across turns; the tail breakpoint lets
            // each new turn reuse the accumulated prefix.
            payload["system"] = match system_parts.as_slice() {
                [] => json!(""),
                parts => {
                    let mut blocks: Vec<Value> = parts
                        .iter()
                        .map(|text| json!({ "type": "text", "text": text }))
                        .collect();
                    if let Some(last) = blocks.last_mut() {
                        last["cache_control"] = json!({ "type": "ephemeral" });
                    }
                    blocks.into()
                }
            };
            let mut wire_messages = anthropic_messages(messages);
            if let Some(last) = wire_messages.last_mut() {
                let content = &mut last["content"];
                if content.is_string() {
                    *content = json!([{ "type": "text", "text": content.take() }]);
                }
                if let Some(blocks) = content.as_array_mut() {
                    if let Some(block) = blocks.last_mut() {
                        block["cache_control"] = json!({ "type": "ephemeral" });
                    }
                }
            }
            payload["messages"] = wire_messages.into();
            payload["max_tokens"] = config.max_output.unwrap_or(4096).into();
            if let Some(level) = &config.reasoning {
                if !config.model.contains("3-7") && !config.model.contains("3.7") {
                    return Err("This Anthropic model has no configured thinking budget".into());
                }
                let budget = match level.as_str() {
                    "low" => 2048,
                    "medium" => 4096,
                    "high" => 8192,
                    _ => return Err("Unsupported Anthropic thinking level".into()),
                };
                payload["thinking"] = json!({ "type": "enabled", "budget_tokens": budget });
                payload["max_tokens"] = (budget + config.max_output.unwrap_or(4096)).into();
            }
            let mut key = reqwest::header::HeaderValue::from_str(config.api_key.trim())
                .map_err(|_| "Invalid Anthropic API key header")?;
            key.set_sensitive(true);
            let request = identity_headers(
                client
                    .post(url)
                    .header("Accept", "text/event-stream")
                    .header("x-api-key", key)
                    .header("anthropic-version", "2023-06-01"),
                config,
            )
            .json(&payload);
            let mut parser = AnthropicParser::default();
            stream_common(config, request, &mut parser, cancel, events).await
        })
    }
}

/// Anthropic `content_block_*`/`message_*` events → normalized events.
#[derive(Default)]
pub(crate) struct AnthropicParser {
    tool_calls: BTreeMap<u64, ToolCallSpec>,
    /// Thinking/redacted_thinking blocks keyed by content-block index. Replayed
    /// verbatim on the next request — Anthropic requires them (with signature)
    /// on assistant turns that precede tool_use.
    thinking_blocks: BTreeMap<u64, Value>,
    /// Visible正文累计 —— 回放给下一轮 `messages`。
    content: String,
    saw_finish: bool,
    finish_reason: Option<String>,
}

impl FrameParser for AnthropicParser {
    fn handle(&mut self, frame: Frame) -> AppResult<Vec<StreamEvent>> {
        let mut events = Vec::new();
        let value = match frame {
            Frame::Done => {
                events.push(StreamEvent::Done);
                return Ok(events);
            }
            Frame::Data(value) => value,
        };
        if let Some(usage) = value.get("usage").or_else(|| value.pointer("/message/usage")) {
            events.push(StreamEvent::Usage {
                input: usage.get("input_tokens").and_then(|v| v.as_u64()),
                output: usage.get("output_tokens").and_then(|v| v.as_u64()),
                cache_hit: usage
                    .get("cache_read_input_tokens")
                    .and_then(|v| v.as_u64()),
                cache_write: usage
                    .get("cache_creation_input_tokens")
                    .and_then(|v| v.as_u64()),
                reasoning: None,
            });
        }
        match value.get("type").and_then(|value| value.as_str()) {
            Some("content_block_start") => {
                let index = value.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                match value.pointer("/content_block/type").and_then(|t| t.as_str()) {
                    Some("tool_use") => {
                        let acc = self.tool_calls.entry(index).or_default();
                        if let Some(id) = value.pointer("/content_block/id").and_then(|v| v.as_str()) {
                            acc.id = id.into();
                        }
                        if let Some(name) = value.pointer("/content_block/name").and_then(|v| v.as_str()) {
                            acc.name = name.into();
                            // tool_use 块出现即一段可见生成输出（DSH isTokenDelta）。
                            events.push(StreamEvent::ToolCallDelta(name.to_string()));
                        }
                    }
                    Some("thinking") => {
                        self.thinking_blocks.insert(
                            index,
                            json!({ "type": "thinking", "thinking": "", "signature": "" }),
                        );
                    }
                    Some("redacted_thinking") => {
                        // 整块原样保留（`data` 是不透明载荷），回放时逐字回传。
                        if let Some(block) = value.get("content_block") {
                            self.thinking_blocks.insert(index, block.clone());
                        }
                    }
                    _ => {}
                }
            }
            Some("content_block_delta") => {
                let index = value.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                if let Some(partial) = value
                    .pointer("/delta/partial_json")
                    .and_then(|text| text.as_str())
                {
                    self.tool_calls.entry(index).or_default().arguments.push_str(partial);
                    if !partial.is_empty() {
                        events.push(StreamEvent::ToolCallDelta(partial.to_string()));
                    }
                }
                if let Some(text) = value.pointer("/delta/text").and_then(|text| text.as_str()) {
                    self.content.push_str(text);
                    events.push(StreamEvent::Token(text.into()));
                }
                if let Some(thinking) = value
                    .pointer("/delta/thinking")
                    .and_then(|text| text.as_str())
                    .filter(|text| !text.is_empty())
                {
                    if let Some(block) = self.thinking_blocks.get_mut(&index) {
                        if let Some(current) = block.pointer_mut("/thinking").and_then(|t| t.as_str()) {
                            let mut merged = current.to_string();
                            merged.push_str(thinking);
                            block["thinking"] = merged.into();
                        }
                    }
                    events.push(StreamEvent::Reasoning(thinking.into()));
                }
                if let Some(signature) = value
                    .pointer("/delta/signature")
                    .and_then(|text| text.as_str())
                {
                    if let Some(block) = self.thinking_blocks.get_mut(&index) {
                        block["signature"] = signature.into();
                    }
                }
            }
            Some("message_stop") => {
                events.push(StreamEvent::Done);
            }
            Some("message_delta")
                if value
                    .pointer("/delta/stop_reason")
                    .is_some_and(|value| !value.is_null()) =>
            {
                self.saw_finish = true;
                if let Some(reason) = value.pointer("/delta/stop_reason").and_then(|v| v.as_str()) {
                    self.finish_reason = Some(reason.to_string());
                }
            }
            _ => {}
        }
        Ok(events)
    }

    fn finish(&mut self) -> AppResult<Vec<StreamEvent>> {
        if !self.saw_finish {
            return Err("模型流提前结束，未收到结束标记".into());
        }
        Ok(vec![StreamEvent::Done])
    }

    fn drain_tool_calls(&mut self) -> Vec<ToolCallSpec> {
        std::mem::take(&mut self.tool_calls)
            .into_values()
            .filter(|call| !call.name.is_empty())
            .collect()
    }

    fn take_content(&mut self) -> String {
        std::mem::take(&mut self.content)
    }

    fn take_thinking(&mut self) -> Option<serde_json::Value> {
        let blocks = std::mem::take(&mut self.thinking_blocks);
        (!blocks.is_empty()).then(|| Value::Array(blocks.into_values().collect()))
    }

    fn finish_reason(&self) -> Option<String> {
        self.finish_reason.clone()
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}
