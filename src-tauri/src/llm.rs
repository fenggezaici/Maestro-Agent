//! OpenAI-compatible streaming transport. No SQLite or tool execution lives here.
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tokio::sync::watch;

use crate::{
    kernel::Kernel,
    model::{AppResult, Mode, RunMetrics, RunStatus, RunView, SessionDetail},
    workspace::WorkspaceService,
};

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
        }
    }
}

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

pub fn conversation(detail: &SessionDetail, run: &RunView) -> Vec<ChatMessage> {
    let policy = match run.mode {
        Mode::Plan => "你处于只读计划模式。只能分析、说明和提出计划，不得执行工具、修改文件或声称已应用变更。",
        Mode::Code | Mode::Execute => "你处于 Code 模式。提出代码修改和具体 Diff，每次变更须等待批准。当前未启用执行器，不得声称已应用修改。",
        Mode::Confirm => "你处于 Confirm 模式。每个工具或命令必须逐步确认。当前未启用工具执行器，不得声称已执行操作。",
        Mode::Auto => "你处于 Auto 策略。仅可使用实际开放且授权的能力；当前只有文本聊天，不得声称拥有文件写入或命令执行能力。",
    };
    let mut messages = vec![ChatMessage::text("system", policy)];
    let prior: Vec<_> = detail
        .runs
        .iter()
        .filter(|item| {
            item.id < run.id
                && item.session_id == run.session_id
                && item.status == RunStatus::Completed
                && !item.output.is_empty()
        })
        .collect();
    let mut budget = 131072usize;
    let mut included = Vec::new();
    for item in prior.into_iter().rev().take(32) {
        let size = item.prompt.len() + item.output.len();
        if size > budget {
            break;
        }
        budget -= size;
        included.push(item);
    }
    for item in included.into_iter().rev() {
        messages.push(ChatMessage::text("user", item.prompt.clone()));
        messages.push(ChatMessage::text("assistant", item.output.clone()));
    }
    messages.push(ChatMessage::text("user", run.prompt.clone()));
    messages
}

#[tauri::command(rename_all = "snake_case")]
// Tauri injects handles/state in addition to the explicit IPC arguments.
#[allow(clippy::too_many_arguments)]
pub async fn stream_chat(
    app: AppHandle,
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    workspace: State<'_, WorkspaceService>,
    messages: Vec<ChatMessage>,
    api_key: String,
    base_url: String,
    model: String,
    session_id: Option<i64>,
    provider: Option<Provider>,
    reasoning_effort: Option<String>,
) -> AppResult<i64> {
    if window.label() != "main" || app.get_webview_window("main").is_none() {
        return Err("Only the main window may start a chat".into());
    }
    let result = async {
        let config = ChatConfig {
            api_key,
            base_url,
            model,
            provider: provider.unwrap_or_default(),
            reasoning: reasoning_effort,
        };
        config.validate()?;
        let prompt = validate_messages(&messages)?;
        let id = match session_id {
            Some(id) => id,
            None => kernel.create_session(None).await?.id,
        };
        kernel
            .set_workspace_root(workspace.current_root())
            .await;
        kernel.inner().start_model(id, prompt, config).await
    }
    .await;
    if let Err(message) = &result {
        let _ = app.emit_to("main", "chat-error", message);
    }
    result
}

#[derive(Debug, PartialEq)]
pub enum StreamEvent {
    Token(String),
    Usage {
        input: Option<u64>,
        output: Option<u64>,
    },
    Done,
}

#[derive(Default)]
pub struct SseDecoder {
    bytes: Vec<u8>,
    data: Vec<String>,
    done: bool,
    saw_finish: bool,
    first_line: bool,
    skip_lf: bool,
    /// Tool calls accumulate across SSE chunks keyed by their stream index
    /// (OpenAI `delta.tool_calls[].index`, Anthropic `content_block` index).
    tool_calls: std::collections::BTreeMap<u64, ToolCallSpec>,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self {
            first_line: true,
            ..Self::default()
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> AppResult<Vec<StreamEvent>> {
        if self.done {
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        for &byte in chunk {
            if self.skip_lf {
                self.skip_lf = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if byte == b'\n' || byte == b'\r' {
                let line = std::mem::take(&mut self.bytes);
                self.line(&line, &mut events)?;
                self.skip_lf = byte == b'\r';
                if self.done {
                    break;
                }
            } else {
                self.bytes.push(byte);
                if self.bytes.len() > 1048576 {
                    return Err("SSE frame 超过大小限制".into());
                }
            }
        }
        Ok(events)
    }

    fn line(&mut self, bytes: &[u8], events: &mut Vec<StreamEvent>) -> AppResult<()> {
        let line = std::str::from_utf8(bytes)
            .map_err(|_| "SSE 包含无效 UTF-8".to_string())?
            .trim_end_matches('\r');
        let line = if self.first_line {
            self.first_line = false;
            line.trim_start_matches('\u{feff}')
        } else {
            line
        };
        if line.is_empty() {
            return self.dispatch(events);
        }
        if let Some(data) = line.strip_prefix("data:") {
            self.data
                .push(data.strip_prefix(' ').unwrap_or(data).to_string());
            if self.data.iter().map(String::len).sum::<usize>() > 1048576 {
                return Err("SSE data 超过大小限制".into());
            }
        }
        Ok(())
    }

    fn dispatch(&mut self, events: &mut Vec<StreamEvent>) -> AppResult<()> {
        if self.data.is_empty() || self.done {
            return Ok(());
        }
        let data = std::mem::take(&mut self.data).join("\n");
        if data.trim() == "[DONE]" {
            self.done = true;
            events.push(StreamEvent::Done);
            return Ok(());
        }
        let value: serde_json::Value =
            serde_json::from_str(&data).map_err(|_| "模型返回了无法解析的 SSE JSON".to_string())?;
        if let Some(error) = value.get("error") {
            return Err(error
                .get("message")
                .and_then(|message| message.as_str())
                .unwrap_or("模型服务返回错误")
                .to_string());
        }
        if let Some(usage) = value
            .get("usage")
            .or_else(|| value.pointer("/message/usage"))
        {
            events.push(StreamEvent::Usage {
                input: usage
                    .get("prompt_tokens")
                    .or_else(|| usage.get("input_tokens"))
                    .and_then(|value| value.as_u64()),
                output: usage
                    .get("completion_tokens")
                    .or_else(|| usage.get("output_tokens"))
                    .and_then(|value| value.as_u64()),
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
                return Ok(());
            }
            Some("content_block_delta") => {
                if let Some(partial) = value
                    .pointer("/delta/partial_json")
                    .and_then(|text| text.as_str())
                {
                    let index = value.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                    self.tool_calls.entry(index).or_default().arguments.push_str(partial);
                    return Ok(());
                }
                if let Some(text) = value.pointer("/delta/text").and_then(|text| text.as_str()) {
                    events.push(StreamEvent::Token(text.into()));
                }
                return Ok(());
            }
            Some("message_stop") => {
                self.done = true;
                events.push(StreamEvent::Done);
                return Ok(());
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
                events.push(StreamEvent::Token(content.into()));
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
        Ok(())
    }

    /// Calls accumulated across the stream; valid once the turn completed.
    pub fn drain_tool_calls(&mut self) -> Vec<ToolCallSpec> {
        std::mem::take(&mut self.tool_calls)
            .into_values()
            .filter(|call| !call.name.is_empty())
            .collect()
    }

    pub fn finish(&mut self) -> AppResult<Vec<StreamEvent>> {
        let mut events = Vec::new();
        if !self.bytes.is_empty() && !self.done {
            let line = std::mem::take(&mut self.bytes);
            self.line(&line, &mut events)?;
        }
        self.dispatch(&mut events)?;
        if !self.done {
            if !self.saw_finish {
                return Err("模型流提前结束，未收到结束标记".into());
            }
            self.done = true;
            events.push(StreamEvent::Done);
        }
        Ok(events)
    }
}

/// One model turn: either the stream completed (`done`, possibly with
/// `tool_calls` to execute) or it was cancelled.
pub struct TurnOutcome {
    pub done: bool,
    pub tool_calls: Vec<ToolCallSpec>,
}

/// OpenAI wire format: assistant tool_calls become function objects; tool
/// results travel as `role: "tool"` messages with `tool_call_id`.
fn openai_messages(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|message| {
            if let Some(calls) = &message.tool_calls {
                serde_json::json!({
                    "role": "assistant",
                    "content": if message.content.is_empty() { serde_json::Value::Null } else { message.content.clone().into() },
                    "tool_calls": calls.iter().map(|call| serde_json::json!({
                        "id": call.id, "type": "function",
                        "function": { "name": call.name, "arguments": call.arguments },
                    })).collect::<Vec<_>>(),
                })
            } else if message.role == "tool" {
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": message.tool_call_id.clone().unwrap_or_default(),
                    "content": message.content,
                })
            } else {
                serde_json::json!({ "role": message.role, "content": message.content })
            }
        })
        .collect()
}

/// Anthropic wire format: tool_use blocks inside assistant content arrays and
/// tool_result blocks folded into following user messages.
fn anthropic_messages(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = Vec::new();
    for message in messages {
        match message.role.as_str() {
            "system" => {}
            "tool" => {
                let block = serde_json::json!({
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
                    _ => out.push(serde_json::json!({ "role": "user", "content": [block] })),
                }
            }
            "assistant" if message.tool_calls.is_some() => {
                let mut content = Vec::new();
                if !message.content.is_empty() {
                    content.push(serde_json::json!({ "type": "text", "text": message.content }));
                }
                for call in message.tool_calls.iter().flatten() {
                    content.push(serde_json::json!({
                        "type": "tool_use", "id": call.id, "name": call.name,
                        "input": serde_json::from_str::<serde_json::Value>(&call.arguments)
                            .unwrap_or_else(|_| serde_json::json!({})),
                    }));
                }
                out.push(serde_json::json!({ "role": "assistant", "content": content }));
            }
            role => out.push(serde_json::json!({ "role": role, "content": message.content })),
        }
    }
    out
}

/// Convert our neutral `{name, description, parameters}` definitions into each
/// provider's tools schema.
fn provider_tools(tools: &[serde_json::Value], provider: Provider) -> serde_json::Value {
    match provider {
        Provider::Anthropic => tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.get("name").and_then(|v| v.as_str()).unwrap_or_default(),
                    "description": tool.get("description").and_then(|v| v.as_str()).unwrap_or_default(),
                    "input_schema": tool.get("parameters").cloned().unwrap_or_else(|| serde_json::json!({"type":"object"})),
                })
            })
            .collect(),
        _ => tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.get("name").and_then(|v| v.as_str()).unwrap_or_default(),
                        "description": tool.get("description").and_then(|v| v.as_str()).unwrap_or_default(),
                        "parameters": tool.get("parameters").cloned().unwrap_or_else(|| serde_json::json!({"type":"object"})),
                    },
                })
            })
            .collect(),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FetchedModel {
    pub id: String,
    pub context: Option<u64>,
}

/// Query the provider for its real model list. OpenAI-compatible providers expose
/// GET {base}/models (ids only — context stays None, never invented). Ollama uses
/// GET {base}/api/tags plus POST /api/show per model for a real context length.
pub async fn fetch_models(config: &ChatConfig) -> AppResult<Vec<FetchedModel>> {
    if config.api_key.trim().is_empty() && !matches!(config.provider, Provider::Ollama) {
        return Err("API Key 不能为空".into());
    }
    let base = Url::parse(config.base_url.trim()).map_err(|_| "Base URL 格式无效".to_string())?;
    if !matches!(base.scheme(), "http" | "https") || base.host_str().is_none() {
        return Err("Base URL 必须为 HTTP(S)".into());
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "无法初始化模型 HTTP 客户端".to_string())?;
    let root = format!("{}{}", base.origin().ascii_serialization(), base.path().trim_end_matches('/'));

    if matches!(config.provider, Provider::Ollama) {
        let response = client
            .get(format!("{root}/api/tags"))
            .send()
            .await
            .map_err(|_| "无法连接 Ollama，请确认服务已启动".to_string())?;
        if !response.status().is_success() {
            return Err(format!("Ollama 返回 HTTP {}", response.status().as_u16()));
        }
        let body: serde_json::Value = response.json().await.map_err(|_| "Ollama 响应无法解析".to_string())?;
        let ids: Vec<String> = body["models"]
            .as_array()
            .map(|items| items.iter().filter_map(|item| item["name"].as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let mut models = Vec::with_capacity(ids.len());
        for id in ids {
            // model_info holds e.g. "llama.context_length" — a real context window.
            let context = match client
                .post(format!("{root}/api/show"))
                .json(&serde_json::json!({ "name": id }))
                .send()
                .await
            {
                Ok(response) => response
                    .json::<serde_json::Value>()
                    .await
                    .ok()
                    .and_then(|body| {
                        body["model_info"].as_object().and_then(|info| {
                            info.iter()
                                .find(|(key, _)| key.ends_with("context_length"))
                                .and_then(|(_, value)| value.as_u64())
                        })
                    }),
                Err(_) => None,
            };
            models.push(FetchedModel { id, context });
        }
        return Ok(models);
    }

    let mut request = client.get(format!("{root}/models"));
    if let Ok(value) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", config.api_key.trim())) {
        request = request.header(reqwest::header::AUTHORIZATION, value);
    }
    let response = request.send().await.map_err(|_| "无法连接 Provider，检查 Base URL 与网络".to_string())?;
    if !response.status().is_success() {
        return Err(format!("Provider 返回 HTTP {}", response.status().as_u16()));
    }
    let body: serde_json::Value = response.json().await.map_err(|_| "模型列表响应无法解析".to_string())?;
    Ok(body["data"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["id"].as_str().map(str::to_string))
                .map(|id| FetchedModel { id, context: None })
                .collect()
        })
        .unwrap_or_default())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CatalogModel {
    pub id: String,
    pub context: Option<u64>,
    pub reasoning: bool,
}

/// Fetch the public models.dev catalog (same source opencode and similar tools
/// use for model metadata) and flatten every provider's models into a slim list.
/// The caller caches the result client-side for ~24h.
pub async fn model_catalog() -> AppResult<Vec<CatalogModel>> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "无法初始化模型目录 HTTP 客户端".to_string())?;
    let body = client
        .get("https://models.dev/api.json")
        .send()
        .await
        .map_err(|_| "无法连接 models.dev 模型目录".to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|_| "models.dev 响应无法解析".to_string())?;
    let mut models = Vec::new();
    if let Some(providers) = body.as_object() {
        for provider in providers.values() {
            if let Some(entries) = provider["models"].as_object() {
                for (id, meta) in entries {
                    models.push(CatalogModel {
                        id: id.clone(),
                        context: meta["limit"]["context"].as_u64(),
                        reasoning: meta["reasoning"].as_bool().unwrap_or(false),
                    });
                }
            }
        }
    }
    Ok(models)
}

pub async fn stream(
    config: &ChatConfig,
    messages: Vec<ChatMessage>,
    tools: &[serde_json::Value],
    cancel: &mut watch::Receiver<bool>,
    kernel: &Kernel,
    run_id: i64,
) -> AppResult<TurnOutcome> {
    let url = config.validate()?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|_| "无法初始化模型 HTTP 客户端".to_string())?;
    let mut payload = serde_json::json!({ "model": config.model.trim(), "stream": true });
    if !tools.is_empty() {
        payload["tools"] = provider_tools(tools, config.provider);
    }
    if matches!(config.provider, Provider::Anthropic) {
        let system = messages
            .iter()
            .find(|message| message.role == "system")
            .map(|message| message.content.clone())
            .unwrap_or_default();
        payload["system"] = system.into();
        payload["messages"] = anthropic_messages(&messages).into();
        payload["max_tokens"] = 4096.into();
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
            payload["thinking"] = serde_json::json!({ "type": "enabled", "budget_tokens": budget });
            payload["max_tokens"] = (budget + 4096).into();
        }
    } else {
        payload["messages"] = openai_messages(&messages).into();
        payload["stream_options"] = serde_json::json!({ "include_usage": true });
        if let Some(level) = &config.reasoning {
            if !matches!(config.provider, Provider::Openai)
                || !(config.model.starts_with("o3") || config.model.starts_with("gpt-5"))
                || !matches!(level.as_str(), "low" | "medium" | "high" | "xhigh")
            {
                return Err("Unsupported reasoning strength for this provider/model".into());
            }
            if level == "xhigh" && !config.model.starts_with("gpt-5.2") {
                return Err("Extra High is not supported by this model mapping".into());
            }
            payload["reasoning_effort"] = level.clone().into();
        }
    }
    let mut request = client.post(url).header("Accept", "text/event-stream");
    if matches!(config.provider, Provider::Anthropic) {
        let mut key = reqwest::header::HeaderValue::from_str(config.api_key.trim())
            .map_err(|_| "Invalid Anthropic API key header")?;
        key.set_sensitive(true);
        request = request
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01");
    } else if !config.api_key.is_empty() {
        request = request.bearer_auth(config.api_key.trim());
    }
    let request = request.json(&payload);
    let started = Instant::now();
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
    let mut reported = Instant::now();
    let session_id = kernel
        .snapshot()
        .await
        .active_run
        .as_ref()
        .filter(|run| run.id == run_id)
        .map(|run| run.session_id)
        .ok_or("Run no longer active")?;
    let mut metrics = RunMetrics {
        run_id,
        session_id,
        provider: config.provider.name().into(),
        model: config.model.clone(),
        input_tokens: None,
        output_tokens: None,
        elapsed_ms: 0,
        first_token_ms: None,
    };
    loop {
        if *cancel.borrow() {
            return Ok(TurnOutcome { done: false, tool_calls: Vec::new() });
        }
        let chunk = tokio::select! {
            biased;
            _ = cancel.changed() => return Ok(TurnOutcome { done: false, tool_calls: Vec::new() }),
            chunk = tokio::time::timeout(Duration::from_secs(60), body.next()) => chunk.map_err(|_| "等待模型 token 超时".to_string())?,
        };
        let events = match chunk {
            Some(Ok(bytes)) => decoder.push(&bytes)?,
            Some(Err(_)) => return Err("读取模型流失败，网络连接已中断".into()),
            None => decoder.finish()?,
        };
        for event in events {
            match event {
                StreamEvent::Token(token) => {
                    if *cancel.borrow() {
                        return Ok(TurnOutcome { done: false, tool_calls: Vec::new() });
                    }
                    output_bytes += token.len();
                    if metrics.first_token_ms.is_none() {
                        metrics.first_token_ms = Some(started.elapsed().as_millis() as u64);
                    }
                    if output_bytes > 1048576 {
                        return Err("模型回复超过 1 MiB 限制".into());
                    }
                    kernel.token(run_id, token).await;
                    if reported.elapsed() >= Duration::from_millis(200) {
                        metrics.elapsed_ms = started.elapsed().as_millis() as u64;
                        kernel.report_metrics(metrics.clone(), false).await?;
                        reported = Instant::now();
                    }
                }
                StreamEvent::Usage { input, output } => {
                    metrics.input_tokens = input.or(metrics.input_tokens);
                    metrics.output_tokens = output.or(metrics.output_tokens);
                }
                StreamEvent::Done => {
                    metrics.elapsed_ms = started.elapsed().as_millis() as u64;
                    kernel.report_metrics(metrics, true).await?;
                    return Ok(TurnOutcome {
                        done: true,
                        tool_calls: decoder.drain_tool_calls(),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_utf8_across_every_byte_boundary_and_crlf_frames() {
        let wire = "\u{feff}: heartbeat\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"你好🌍\"}}]}\r\n\r\ndata: [DONE]\r\n\r\n";
        let mut decoder = SseDecoder::new();
        let mut events = Vec::new();
        for byte in wire.as_bytes() {
            events.extend(decoder.push(&[*byte]).unwrap());
        }
        assert_eq!(
            events,
            vec![StreamEvent::Token("你好🌍".into()), StreamEvent::Done]
        );
        assert!(decoder.finish().unwrap().is_empty());
    }

    #[test]
    fn rejects_corrupt_provider_events_and_truncated_streams() {
        assert!(SseDecoder::new().push(b"data: invalid\n\n").is_err());
        assert!(SseDecoder::new()
            .push(b"data: {\"error\":{\"message\":\"denied\"}}\n\n")
            .is_err());
        let mut decoder = SseDecoder::new();
        decoder
            .push(b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n")
            .unwrap();
        assert!(decoder.finish().is_err());
        let mut decoder = SseDecoder::new();
        decoder
            .push(b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n")
            .unwrap();
        assert_eq!(decoder.finish().unwrap(), vec![StreamEvent::Done]);
    }

    #[test]
    fn accepts_all_sse_line_endings_and_multiline_data() {
        for ending in ["\r", "\n", "\r\n"] {
            let wire = format!("data: {{\"choices\":{ending}data: [{{\"delta\":{{\"content\":\"split\"}}}}]}}{ending}{ending}data: [DONE]{ending}{ending}");
            let mut decoder = SseDecoder::new();
            let mut events = Vec::new();
            for byte in wire.bytes() {
                events.extend(decoder.push(&[byte]).unwrap());
            }
            assert_eq!(
                events,
                vec![StreamEvent::Token("split".into()), StreamEvent::Done]
            );
        }
    }

    #[test]
    fn normalizes_endpoint_rejects_unsafe_credentials_and_redacts_key() {
        let mut config = ChatConfig {
            api_key: "test-secret".into(),
            base_url: "https://example.com/v1/".into(),
            model: "test-model".into(),
            provider: Provider::Openai,
            reasoning: None,
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
        assert!(validate_messages(&[ChatMessage::text("system", "bypass")]).is_err());
    }
}
