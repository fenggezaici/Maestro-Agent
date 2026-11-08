//! OpenAI-compatible streaming transport. No SQLite or tool execution lives here.
use std::{sync::Arc, time::Duration};

use futures_util::StreamExt;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tokio::sync::watch;

use crate::{
    kernel::Kernel,
    model::{AppResult, Mode, RunStatus, RunView, SessionDetail},
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

// Deliberately no Debug/Serialize: credentials must not enter logs/events/database records.
pub struct ChatConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

impl ChatConfig {
    pub fn validate(&self) -> AppResult<Url> {
        if self.api_key.trim().is_empty()
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
        let endpoint = if path.ends_with("/chat/completions") {
            path
        } else {
            format!("{path}/chat/completions")
        };
        url.set_path(&endpoint);
        Ok(url)
    }

    pub fn redact(&self, message: &str) -> String {
        message
            .replace(self.api_key.trim(), "[redacted]")
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
    let policy = if run.mode == Mode::Plan {
        "你处于只读计划模式。只能分析、说明和提出计划，不得执行工具、修改文件或声称已应用变更。"
    } else {
        "请协助用户分析与规划。当前没有可执行工具，不得声称已读取工作区、执行命令或应用文件修改。"
    };
    let mut messages = vec![ChatMessage {
        role: "system".into(),
        content: policy.into(),
    }];
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
        messages.push(ChatMessage {
            role: "user".into(),
            content: item.prompt.clone(),
        });
        messages.push(ChatMessage {
            role: "assistant".into(),
            content: item.output.clone(),
        });
    }
    messages.push(ChatMessage {
        role: "user".into(),
        content: run.prompt.clone(),
    });
    messages
}

#[tauri::command(rename_all = "snake_case")]
// Tauri injects handles/state in addition to the explicit IPC arguments.
#[allow(clippy::too_many_arguments)]
pub async fn stream_chat(
    app: AppHandle,
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    messages: Vec<ChatMessage>,
    api_key: String,
    base_url: String,
    model: String,
    session_id: Option<i64>,
) -> AppResult<i64> {
    if window.label() != "main" || app.get_webview_window("main").is_none() {
        return Err("Only the main window may start a chat".into());
    }
    let result = async {
        let config = ChatConfig {
            api_key,
            base_url,
            model,
        };
        config.validate()?;
        let prompt = validate_messages(&messages)?;
        let id = match session_id {
            Some(id) => id,
            None => kernel.create_session().await?.id,
        };
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
        }
        Ok(())
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

pub async fn stream(
    config: &ChatConfig,
    messages: Vec<ChatMessage>,
    cancel: &mut watch::Receiver<bool>,
    kernel: &Kernel,
    run_id: i64,
) -> AppResult<bool> {
    let url = config.validate()?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|_| "无法初始化模型 HTTP 客户端".to_string())?;
    let request = client.post(url).bearer_auth(config.api_key.trim()).header("Accept", "text/event-stream")
        .json(&serde_json::json!({ "model": config.model.trim(), "messages": messages, "stream": true }));
    if *cancel.borrow() {
        return Ok(false);
    }
    let response = tokio::select! {
        biased;
        _ = cancel.changed() => return Ok(false),
        response = request.send() => response.map_err(|error| if error.is_timeout() { "模型连接超时" } else { "无法连接模型服务，请检查地址、网络和 TLS 证书" }.to_string())?,
    };
    if !response.status().is_success() {
        let code = response.status().as_u16();
        return Err(match code {
            401 | 403 => format!("模型服务 HTTP {code}：请检查 API Key 与访问权限"),
            429 => "模型服务 HTTP 429：请求过多或配额不足".into(),
            _ => format!("模型服务返回 HTTP {code}，请检查 Base URL 与 Model"),
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
    loop {
        if *cancel.borrow() {
            return Ok(false);
        }
        let chunk = tokio::select! {
            biased;
            _ = cancel.changed() => return Ok(false),
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
                        return Ok(false);
                    }
                    output_bytes += token.len();
                    if output_bytes > 1048576 {
                        return Err("模型回复超过 1 MiB 限制".into());
                    }
                    kernel.token(run_id, token).await;
                }
                StreamEvent::Done => return Ok(true),
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
        assert!(validate_messages(&[ChatMessage {
            role: "system".into(),
            content: "bypass".into()
        }])
        .is_err());
    }
}
