//! LLM adapter layer: one `LlmProvider` trait, per-vendor wire formats, and a
//! shared streaming transport. No SQLite, no Kernel — events flow out through
//! a channel so callers decide what to do with them.
mod anthropic;
mod catalog;
mod openai;
mod sse;

use std::{sync::Arc, time::Duration};

use futures_util::{future::BoxFuture, StreamExt};
use reqwest::Url;
use serde::Deserialize;
use tokio::sync::{mpsc, watch};

use crate::domain::{AppResult, ChatMessage, ToolCallSpec, ToolSpec};

pub use anthropic::AnthropicProvider;
pub use catalog::{fetch_models, model_catalog, CatalogModel, FetchedModel};
pub use openai::OpenAiProvider;
pub(crate) use sse::{Frame, SseDecoder};

// Deliberately no Debug/Serialize: credentials must not enter logs/events/database records.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Openai,
    Deepseek,
    Anthropic,
    Ollama,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Deepseek => "deepseek",
            Self::Anthropic => "anthropic",
            Self::Ollama => "ollama",
        }
    }
}

pub struct ChatConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub provider: Provider,
    pub reasoning: Option<String>,
    pub max_output: Option<u64>,
}

impl ChatConfig {
    pub fn validate(&self) -> AppResult<Url> {
        if (self.api_key.trim().is_empty() && !matches!(self.provider, Provider::Ollama))
            || self.api_key.len() > 8192
            || self.api_key.chars().any(char::is_control)
        {
            return Err("API Key 不能为空或包含控制字符".into());
        }
        if self.model.trim().is_empty() || self.model.len() > 256 {
            return Err("Model 不能为空或过长".into());
        }
        let mut url =
            Url::parse(self.base_url.trim()).map_err(|_| "Base URL 格式无效".to_string())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("Base URL 必须为 HTTP(S)，且不能含账号、查询参数或片段".into());
        }
        let path = url.path().trim_end_matches('/').to_string();
        let suffix = if matches!(self.provider, Provider::Anthropic) {
            "/messages"
        } else {
            "/chat/completions"
        };
        let endpoint = if path.ends_with(suffix) {
            path
        } else {
            format!("{path}{suffix}")
        };
        url.set_path(&endpoint);
        Ok(url)
    }

    pub fn redact(&self, message: &str) -> String {
        message
            .replace(
                if self.api_key.trim().is_empty() {
                    "\u{0}"
                } else {
                    self.api_key.trim()
                },
                "[redacted]",
            )
            .chars()
            .take(700)
            .filter(|character| !character.is_control() || *character == '\n')
            .collect()
    }
}

/// Normalized streaming events every provider emits.
#[derive(Debug, PartialEq)]
pub enum StreamEvent {
    Token(String),
    /// Model reasoning/thinking channel (DeepSeek `reasoning_content`,
    /// Anthropic `thinking_delta`, inline `<think>` blocks).
    Reasoning(String),
    Usage {
        input: Option<u64>,
        output: Option<u64>,
        cache_hit: Option<u64>,
    },
    Done,
}

/// One model turn: either the stream completed (`done`, possibly with
/// `tool_calls` to execute) or it was cancelled.
pub struct TurnOutcome {
    pub done: bool,
    pub tool_calls: Vec<ToolCallSpec>,
}

/// A model provider: builds its wire payload and interprets its own frames.
/// Implementations stay Kernel-free — streaming events are pushed through
/// `events` for the agent loop to relay.
pub trait LlmProvider: Send + Sync {
    fn stream<'a>(
        self: Arc<Self>,
        config: &'a ChatConfig,
        messages: &'a [ChatMessage],
        tools: &'a [ToolSpec],
        cancel: &'a mut watch::Receiver<bool>,
        events: mpsc::UnboundedSender<StreamEvent>,
    ) -> BoxFuture<'a, AppResult<TurnOutcome>>;
}

/// Resolve the concrete provider implementation for a `Provider` variant.
/// The resolver seam is how tests substitute a stub without touching HTTP.
pub type ProviderResolver = Arc<dyn Fn(Provider) -> Arc<dyn LlmProvider> + Send + Sync>;

pub fn provider_for(provider: Provider) -> Arc<dyn LlmProvider> {
    match provider {
        Provider::Anthropic => Arc::new(AnthropicProvider),
        _ => Arc::new(OpenAiProvider),
    }
}

/// Per-provider frame interpretation. Implemented by each vendor module; the
/// shared transport feeds `Frame`s and drains accumulated tool calls at Done.
pub(crate) trait FrameParser: Send {
    fn handle(&mut self, frame: Frame) -> AppResult<Vec<StreamEvent>>;
    /// End of the HTTP body; error when the provider's own finish marker was
    /// never seen (truncated stream).
    fn finish(&mut self) -> AppResult<Vec<StreamEvent>>;
    fn drain_tool_calls(&mut self) -> Vec<ToolCallSpec>;
}

/// Translate our neutral `ToolSpec` list into the provider's wire shape.
pub(crate) fn wire_tools(tools: &[ToolSpec], anthropic: bool) -> serde_json::Value {
    if anthropic {
        tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.parameters,
                })
            })
            .collect()
    } else {
        tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    },
                })
            })
            .collect()
    }
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
const CHUNK_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_OUTPUT_BYTES: usize = 1048576;

/// Shared HTTP/SSE transport: send the request, feed frames through the
/// provider's parser, forward normalized events, honor cancellation and the
/// output size cap.
pub(crate) async fn stream_common(
    config: &ChatConfig,
    request: reqwest::RequestBuilder,
    parser: &mut dyn FrameParser,
    cancel: &mut watch::Receiver<bool>,
    events: mpsc::UnboundedSender<StreamEvent>,
) -> AppResult<TurnOutcome> {
    if *cancel.borrow() {
        return Ok(TurnOutcome { done: false, tool_calls: Vec::new() });
    }
    let response = tokio::select! {
        biased;
        _ = cancel.changed() => return Ok(TurnOutcome { done: false, tool_calls: Vec::new() }),
        response = request.send() => response.map_err(|error| if error.is_timeout() { "模型连接超时" } else { "无法连接模型服务，请检查地址、网络和 TLS 证书" }.to_string())?,
    };
    if !response.status().is_success() {
        let code = response.status().as_u16();
        // Surface the provider's own error body (e.g. "Model Not Exist") so the
        // user sees the real reason instead of a generic guess — still redacted.
        let body = response.text().await.unwrap_or_default();
        let detail = config
            .redact(&body)
            .chars()
            .take(300)
            .collect::<String>();
        let suffix = if detail.trim().is_empty() { String::new() } else { format!("：{}", detail.trim()) };
        return Err(match code {
            401 | 403 => format!("模型服务 HTTP {code}：请检查 API Key 与访问权限{suffix}"),
            429 => format!("模型服务 HTTP 429：请求过多或配额不足{suffix}"),
            _ => format!("模型服务返回 HTTP {code}，请检查 Base URL 与 Model{suffix}"),
        });
    }
    if !response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/event-stream"))
    {
        return Err("模型服务未返回 text/event-stream，请确认接口支持 stream: true".into());
    }
    let mut body = response.bytes_stream();
    let mut decoder = SseDecoder::new();
    let mut output_bytes = 0usize;
    let mut process = |frames: Vec<Frame>, parser: &mut dyn FrameParser, events: &mpsc::UnboundedSender<StreamEvent>| -> AppResult<bool> {
        for frame in frames {
            for event in parser.handle(frame)? {
                if let StreamEvent::Token(ref token) = event {
                    output_bytes += token.len();
                    if output_bytes > MAX_OUTPUT_BYTES {
                        return Err("模型回复超过 1 MiB 限制".into());
                    }
                }
                let done = matches!(event, StreamEvent::Done);
                let _ = events.send(event);
                if done {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    };
    loop {
        if *cancel.borrow() {
            return Ok(TurnOutcome { done: false, tool_calls: Vec::new() });
        }
        let chunk = tokio::select! {
            biased;
            _ = cancel.changed() => return Ok(TurnOutcome { done: false, tool_calls: Vec::new() }),
            chunk = tokio::time::timeout(CHUNK_TIMEOUT, body.next()) => chunk.map_err(|_| "等待模型 token 超时".to_string())?,
        };
        match chunk {
            Some(Ok(bytes)) => {
                if process(decoder.push(&bytes)?, parser, &events)? {
                    return Ok(TurnOutcome { done: true, tool_calls: parser.drain_tool_calls() });
                }
            }
            Some(Err(_)) => return Err("读取模型流失败，网络连接已中断".into()),
            None => {
                if process(decoder.finish()?, parser, &events)? {
                    return Ok(TurnOutcome { done: true, tool_calls: parser.drain_tool_calls() });
                }
                for event in parser.finish()? {
                    let done = matches!(event, StreamEvent::Done);
                    let _ = events.send(event);
                    if done {
                        return Ok(TurnOutcome { done: true, tool_calls: parser.drain_tool_calls() });
                    }
                }
                return Ok(TurnOutcome { done: true, tool_calls: parser.drain_tool_calls() });
            }
        }
    }
}

pub(crate) fn http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|_| "无法初始化模型 HTTP 客户端".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_endpoint_rejects_unsafe_credentials_and_redacts_key() {
        let mut config = ChatConfig {
            api_key: "test-secret".into(),
            base_url: "https://example.com/v1/".into(),
            model: "test-model".into(),
            provider: Provider::Openai,
            reasoning: None,
            max_output: None,
        };
        assert_eq!(
            config.validate().unwrap().as_str(),
            "https://example.com/v1/chat/completions"
        );
        assert!(!config.redact("error test-secret").contains("test-secret"));
        for url in [
            "file:///tmp/model",
            "https://user:pass@example.com/v1",
            "https://example.com/v1?key=test-secret",
        ] {
            config.base_url = url.into();
            assert!(config.validate().is_err());
        }
        assert!(crate::domain::validate_messages(&[ChatMessage::text("system", "bypass")]).is_err());
    }
}
