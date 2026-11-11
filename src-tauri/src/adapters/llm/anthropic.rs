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
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": message.tool_call_id.clone().unwrap_or_default(),
                    "content": message.content,
                });
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
                payload["tools"] = wire_tools(tools, true);
            }
            let system = messages
                .iter()
                .find(|message| message.role == "system")
                .map(|message| message.content.clone())
                .unwrap_or_default();
            payload["system"] = system.into();
            payload["messages"] = anthropic_messages(messages).into();
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
    saw_finish: bool,
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
            });
        }
        match value.get("type").and_then(|value| value.as_str()) {
            Some("content_block_start") => {
                if value.pointer("/content_block/type").and_then(|t| t.as_str()) == Some("tool_use")
                {
                    let index = value.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                    let acc = self.tool_calls.entry(index).or_default();
                    if let Some(id) = value.pointer("/content_block/id").and_then(|v| v.as_str()) {
                        acc.id = id.into();
                    }
                    if let Some(name) = value.pointer("/content_block/name").and_then(|v| v.as_str()) {
                        acc.name = name.into();
                    }
                }
            }
            Some("content_block_delta") => {
                if let Some(partial) = value
                    .pointer("/delta/partial_json")
                    .and_then(|text| text.as_str())
                {
                    let index = value.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                    self.tool_calls.entry(index).or_default().arguments.push_str(partial);
                }
                if let Some(text) = value.pointer("/delta/text").and_then(|text| text.as_str()) {
                    events.push(StreamEvent::Token(text.into()));
                }
                if let Some(thinking) = value
                    .pointer("/delta/thinking")
                    .and_then(|text| text.as_str())
                    .filter(|text| !text.is_empty())
                {
                    events.push(StreamEvent::Reasoning(thinking.into()));
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
}
