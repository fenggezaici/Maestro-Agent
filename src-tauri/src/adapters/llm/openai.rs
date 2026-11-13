//! OpenAI-compatible provider: `POST {base}/chat/completions` with
//! `stream: true`. Shared by OpenAI, DeepSeek and Ollama — same wire format,
//! plus the `reasoning_content`/`reasoning` delta channels and inline
//! `<think>…</think>` blocks some models emit inside `content`.
use std::{collections::BTreeMap, sync::Arc};

use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

use crate::domain::{AppResult, ChatMessage, ToolCallSpec, ToolSpec};

use super::{
    http_client, identity_headers, stream_common, wire_tools, ChatConfig, Frame, FrameParser,
    LlmProvider, StreamEvent, TurnOutcome,
};

pub struct OpenAiProvider;

/// OpenAI wire format: assistant tool_calls become function objects; tool
/// results travel as `role: "tool"` messages with `tool_call_id`.
fn openai_messages(messages: &[ChatMessage]) -> Vec<Value> {
    messages
        .iter()
        .map(|message| {
            if let Some(calls) = &message.tool_calls {
                json!({
                    "role": "assistant",
                    "content": if message.content.is_empty() { Value::Null } else { message.content.clone().into() },
                    "tool_calls": calls.iter().map(|call| json!({
                        "id": call.id, "type": "function",
                        "function": { "name": call.name, "arguments": call.arguments },
                    })).collect::<Vec<_>>(),
                })
            } else if message.role == "tool" {
                json!({
                    "role": "tool",
                    "tool_call_id": message.tool_call_id.clone().unwrap_or_default(),
                    "content": message.content,
                })
            } else if message.role == "user" && message.images.as_ref().is_some_and(|v| !v.is_empty()) {
                let mut content = vec![json!({ "type": "text", "text": message.content })];
                for url in message.images.iter().flatten() {
                    if url.starts_with("data:") && url.len() < 3_500_000 {
                        content.push(json!({ "type": "image_url", "image_url": { "url": url } }));
                    }
                }
                json!({ "role": "user", "content": content })
            } else {
                json!({ "role": message.role, "content": message.content })
            }
        })
        .collect()
}

impl LlmProvider for OpenAiProvider {
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
                payload["tools"] = wire_tools(tools, false);
            }
            payload["messages"] = openai_messages(messages).into();
            payload["stream_options"] = json!({ "include_usage": true });
            if let Some(level) = &config.reasoning {
                if !matches!(config.provider, super::Provider::Openai)
                    || !matches!(level.as_str(), "minimal" | "low" | "medium" | "high" | "xhigh" | "max")
                {
                    return Err("Unsupported reasoning strength for this provider/model".into());
                }
                payload["reasoning_effort"] = level.clone().into();
            }
            let mut request = identity_headers(
                client.post(url).header("Accept", "text/event-stream"),
                config,
            );
            if !config.api_key.is_empty() {
                request = request.bearer_auth(config.api_key.trim());
            }
            let mut parser = OpenAiParser::default();
            stream_common(config, request.json(&payload), &mut parser, cancel, events).await
        })
    }
}

/// OpenAI `choices[0].delta` → normalized events. Tool call fragments
/// accumulate by their stream `index` until the turn completes.
#[derive(Default)]
pub(crate) struct OpenAiParser {
    tool_calls: BTreeMap<u64, ToolCallSpec>,
    saw_finish: bool,
    inside_think: bool,
    think_buf: String,
}

impl OpenAiParser {
    /// Route streamed text through the `<think>` tag splitter: inside-tag text
    /// becomes `Reasoning`, outside becomes `Token`. Trailing partial tags are
    /// held until the next delta resolves them.
    fn emit_text(&mut self, text: &str, events: &mut Vec<StreamEvent>) {
        self.think_buf.push_str(text);
        loop {
            let tag = if self.inside_think { "</think>" } else { "<think>" };
            match self.think_buf.find(tag) {
                Some(pos) => {
                    let (head, rest) = self.think_buf.split_at(pos);
                    push_piece(head, self.inside_think, events);
                    self.think_buf = rest[tag.len()..].to_string();
                    self.inside_think = !self.inside_think;
                }
                None => {
                    // Hold back the longest suffix that could still grow into the tag.
                    let keep = (1..tag.len())
                        .rev()
                        .find(|&n| self.think_buf.ends_with(&tag[..n]))
                        .unwrap_or(0);
                    let cut = self.think_buf.len() - keep;
                    let head: String = self.think_buf.drain(..cut).collect();
                    push_piece(&head, self.inside_think, events);
                    break;
                }
            }
        }
    }
}

fn push_piece(piece: &str, inside: bool, events: &mut Vec<StreamEvent>) {
    if piece.is_empty() {
        return;
    }
    events.push(if inside {
        StreamEvent::Reasoning(piece.into())
    } else {
        StreamEvent::Token(piece.into())
    });
}

impl FrameParser for OpenAiParser {
    fn handle(&mut self, frame: Frame) -> AppResult<Vec<StreamEvent>> {
        let mut events = Vec::new();
        let value = match frame {
            Frame::Done => {
                events.push(StreamEvent::Done);
                return Ok(events);
            }
            Frame::Data(value) => value,
        };
        if let Some(usage) = value.get("usage") {
            events.push(StreamEvent::Usage {
                input: usage.get("prompt_tokens").and_then(|v| v.as_u64()),
                output: usage.get("completion_tokens").and_then(|v| v.as_u64()),
                // DeepSeek: prompt_cache_hit_tokens / OpenAI: cached_tokens
                cache_hit: usage
                    .get("prompt_cache_hit_tokens")
                    .or_else(|| usage.pointer("/prompt_tokens_details/cached_tokens"))
                    .and_then(|v| v.as_u64()),
            });
        }
        if let Some(choice) = value.get("choices").and_then(|choices| choices.get(0)) {
            if choice
                .get("finish_reason")
                .is_some_and(|reason| !reason.is_null())
            {
                self.saw_finish = true;
            }
            if let Some(content) = choice
                .pointer("/delta/content")
                .and_then(|content| content.as_str())
                .filter(|content| !content.is_empty())
            {
                self.emit_text(content, &mut events);
            }
            for key in ["reasoning_content", "reasoning"] {
                if let Some(text) = choice
                    .pointer(&format!("/delta/{key}"))
                    .and_then(|text| text.as_str())
                    .filter(|text| !text.is_empty())
                {
                    events.push(StreamEvent::Reasoning(text.into()));
                }
            }
            if let Some(calls) = choice
                .pointer("/delta/tool_calls")
                .and_then(|calls| calls.as_array())
            {
                for call in calls {
                    let index = call.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                    let acc = self.tool_calls.entry(index).or_default();
                    if let Some(id) = call.get("id").and_then(|i| i.as_str()) {
                        acc.id = id.into();
                    }
                    if let Some(name) = call.pointer("/function/name").and_then(|n| n.as_str()) {
                        acc.name = name.into();
                    }
                    if let Some(args) = call.pointer("/function/arguments").and_then(|a| a.as_str()) {
                        acc.arguments.push_str(args);
                    }
                }
            }
        }
        Ok(events)
    }

    fn finish(&mut self) -> AppResult<Vec<StreamEvent>> {
        // Flush any held-back partial tag; an unclosed <think> block counts as
        // reasoning (some local models never emit the closing tag).
        let mut events = Vec::new();
        let tail = std::mem::take(&mut self.think_buf);
        push_piece(&tail, self.inside_think, &mut events);
        if !self.saw_finish {
            return Err("模型流提前结束，未收到结束标记".into());
        }
        events.push(StreamEvent::Done);
        Ok(events)
    }

    fn drain_tool_calls(&mut self) -> Vec<ToolCallSpec> {
        std::mem::take(&mut self.tool_calls)
            .into_values()
            .filter(|call| !call.name.is_empty())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(parser: &mut OpenAiParser, wire: &[u8]) -> Vec<StreamEvent> {
        let mut decoder = crate::adapters::llm::SseDecoder::new();
        let mut events = Vec::new();
        for byte in wire {
            for frame in decoder.push(&[*byte]).unwrap() {
                events.extend(parser.handle(frame).unwrap());
            }
        }
        events
    }

    #[test]
    fn truncated_streams_are_rejected() {
        let mut parser = OpenAiParser::default();
        feed(&mut parser, b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n");
        assert!(parser.finish().is_err());
        let mut parser = OpenAiParser::default();
        feed(&mut parser, b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n");
        assert_eq!(parser.finish().unwrap().last(), Some(&StreamEvent::Done));
    }

    #[test]
    fn think_blocks_split_across_chunk_boundaries_become_reasoning() {
        let frames = [
            r#"data: {"choices":[{"delta":{"content":"答<thi"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"nk>推理一"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"段</think>正文"}}]}"#,
            r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        ];
        let wire = frames.join("\n\n") + "\n\ndata: [DONE]\n\n";
        let mut parser = OpenAiParser::default();
        let events = feed(&mut parser, wire.as_bytes());
        assert!(events.contains(&StreamEvent::Token("答".into())));
        assert!(events.contains(&StreamEvent::Reasoning("推理一".into())));
        assert!(events.contains(&StreamEvent::Reasoning("段".into())));
        assert!(events.contains(&StreamEvent::Token("正文".into())));
    }

    #[test]
    fn tool_call_deltas_accumulate_and_emit_at_done() {
        let wire = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"pa\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"th\\\":\\\"a\\\"}\"}}]}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut parser = OpenAiParser::default();
        let events = feed(&mut parser, wire.as_bytes());
        assert_eq!(events, vec![StreamEvent::Done]);
        let calls = parser.drain_tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments, "{\"path\":\"a\"}");
    }
}
