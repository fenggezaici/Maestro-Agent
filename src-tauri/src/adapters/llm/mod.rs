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
    /// Stable per-conversation id. OpenCode gateways require it as
    /// `x-opencode-session` so requests can be routed/cached together.
    pub session: Option<String>,
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

/// OpenCode Go/Zen gateways: their docs require a session header there, while
/// other providers must not receive it.
pub(crate) fn is_opencode_host(base_url: &str) -> bool {
    Url::parse(base_url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| host == "opencode.ai" || host.ends_with(".opencode.ai"))
}

/// Attach `x-opencode-session` when talking to an OpenCode gateway; every other
/// provider gets the request untouched. A fixed fallback covers sessionless
/// utility calls (model listing).
pub(crate) fn identity_headers(
    request: reqwest::RequestBuilder,
    config: &ChatConfig,
) -> reqwest::RequestBuilder {
    if is_opencode_host(&config.base_url) {
        request.header(
            "x-opencode-session",
            config.session.as_deref().unwrap_or("maestro-utility"),
        )
    } else {
        request
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
const RETRY_MAX_RETRIES: usize = 5;
const RETRY_INITIAL_DELAY_MS: u64 = 2_000;
const RETRY_MAX_DELAY_MS: u64 = 30_000;

fn retryable_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504 | 524)
}

fn retryable_error(error: &reqwest::Error) -> bool {
    error.is_connect() || error.is_timeout() || error.is_request()
}

fn retry_after_ms(response: &reqwest::Response) -> Option<u64> {
    let headers = response.headers();
    if let Some(value) = headers.get("retry-after-ms").and_then(|value| value.to_str().ok()) {
        if let Ok(ms) = value.parse::<f64>() {
            return Some(ms.max(0.0) as u64);
        }
    }
    headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<f64>().ok())
        .map(|seconds| (seconds.max(0.0) * 1000.0) as u64)
}

fn retry_delay(attempt: usize, retry_after: Option<u64>) -> Duration {
    if let Some(ms) = retry_after {
        return Duration::from_millis(ms.min(RETRY_MAX_DELAY_MS));
    }
    let base = RETRY_INITIAL_DELAY_MS.saturating_mul(1u64 << attempt.min(4));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.subsec_nanos())
        .unwrap_or(0);
    let jitter = (base as f64 * 0.25 * (nanos as f64 / 1_000_000_000.0)) as u64;
    Duration::from_millis((base + jitter).min(RETRY_MAX_DELAY_MS))
}

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
    let host = request
        .try_clone()
        .and_then(|builder| builder.build().ok())
        .and_then(|request| request.url().host_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".into());
    let mut attempt = 0usize;
    let response = loop {
        let Some(builder) = request.try_clone() else {
            return Err("模型请求无法重试".into());
        };
        let result = tokio::select! {
            biased;
            _ = cancel.changed() => return Ok(TurnOutcome { done: false, tool_calls: Vec::new() }),
            response = builder.send() => response,
        };
        match result {
            Ok(response) => {
                let status = response.status().as_u16();
                if response.status().is_success() || !retryable_status(status) || attempt >= RETRY_MAX_RETRIES {
                    break response;
                }
                let delay = retry_delay(attempt, retry_after_ms(&response));
                eprintln!("LLM retry {}/{} host={host} status={status} delay={}ms", attempt + 1, RETRY_MAX_RETRIES, delay.as_millis());
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = cancel.changed() => return Ok(TurnOutcome { done: false, tool_calls: Vec::new() }),
                }
                attempt += 1;
            }
            Err(error) => {
                if attempt < RETRY_MAX_RETRIES && retryable_error(&error) {
                    let delay = retry_delay(attempt, None);
                    eprintln!("LLM retry {}/{} host={host} error={error} delay={}ms", attempt + 1, RETRY_MAX_RETRIES, delay.as_millis());
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = cancel.changed() => return Ok(TurnOutcome { done: false, tool_calls: Vec::new() }),
                    }
                    attempt += 1;
                    continue;
                }
                eprintln!("LLM request failed: host={host} error={error}");
                return Err(if error.is_timeout() {
                    format!("模型连接超时（{host}）")
                } else if error.is_connect() {
                    format!("无法连接模型服务（{host}）：连接被拒绝或不可达")
                } else if error.is_request() {
                    format!("无法连接模型服务（{host}）：TLS/请求错误")
                } else {
                    format!("无法连接模型服务（{host}）：{error}")
                });
            }
        }
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
            // Watch notifications can be missed around receiver clones; polling
            // the value guarantees stop within 250ms regardless.
            _ = tokio::time::sleep(Duration::from_millis(250)) => {
                if *cancel.borrow() {
                    return Ok(TurnOutcome { done: false, tool_calls: Vec::new() });
                }
                continue;
            }
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
        // OpenCode (and similar gateways) reject generic SDK/HTTP-library UAs.
        .user_agent(concat!("Maestro-Agent/", env!("CARGO_PKG_VERSION")))
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
            session: None,
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

    fn config_for(base_url: &str, session: Option<&str>) -> ChatConfig {
        ChatConfig {
            api_key: "test-secret".into(),
            base_url: base_url.into(),
            model: "test-model".into(),
            provider: Provider::Openai,
            reasoning: None,
            max_output: None,
            session: session.map(str::to_string),
        }
    }

    #[test]
    fn opencode_gateway_gets_a_stable_session_header() {
        assert!(is_opencode_host("https://opencode.ai/zen/go/v1"));
        assert!(is_opencode_host("https://sub.opencode.ai/v1"));
        assert!(!is_opencode_host("https://api.openai.com/v1"));
        assert!(!is_opencode_host("https://notopencode.ai/v1"));

        let client = reqwest::Client::new();
        let url = "https://opencode.ai/zen/go/v1/chat/completions";
        let opencode = config_for("https://opencode.ai/zen/go/v1", Some("maestro-7"));
        let request = identity_headers(client.post(url), &opencode)
            .build()
            .unwrap();
        assert_eq!(request.headers().get("x-opencode-session").unwrap(), "maestro-7");

        let fallback = config_for("https://opencode.ai/zen/go/v1", None);
        let request = identity_headers(client.post(url), &fallback)
            .build()
            .unwrap();
        assert_eq!(request.headers().get("x-opencode-session").unwrap(), "maestro-utility");

        let other = config_for("https://api.openai.com/v1", Some("maestro-7"));
        let request = identity_headers(client.post(url), &other)
            .build()
            .unwrap();
        assert!(request.headers().get("x-opencode-session").is_none());
    }
}
