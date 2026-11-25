//! MCP transports: stdio child pipes, Streamable HTTP, and legacy HTTP+SSE.
//! All three carry the same JSON-RPC 2.0 payloads; `RunningServer::request`
//! hides the wire differences behind one interface.
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin},
    sync::{oneshot, watch},
};

use crate::domain::{AppResult, McpToolView};

pub const PROTOCOL_VERSION: &str = "2025-03-26";
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_LINE_BYTES: usize = 1048576;
/// SSE frames / HTTP bodies are still JSON-RPC payloads — same cap.
const MAX_HTTP_BYTES: usize = 4 * 1048576;

pub struct Pending {
    pub replies: HashMap<u64, oneshot::Sender<AppResult<Value>>>,
}

/// One spawned/connected server's transport state.
#[allow(clippy::large_enum_variant)]
pub enum Transport {
    Stdio {
        child: tokio::sync::Mutex<Child>,
        stdin: tokio::sync::Mutex<ChildStdin>,
    },
    /// Streamable HTTP: POST JSON-RPC to one endpoint. The server may answer
    /// `application/json` or `text/event-stream`; `Mcp-Session-Id` from the
    /// initialize response is replayed on every later request.
    Http {
        client: reqwest::Client,
        url: String,
        headers: reqwest::header::HeaderMap,
        session_id: tokio::sync::Mutex<Option<String>>,
    },
    /// Legacy HTTP+SSE: GET stream emits `endpoint` (POST target) once, then
    /// `message` events carrying JSON-RPC responses keyed by id.
    Sse {
        client: reqwest::Client,
        post_url: String,
        headers: reqwest::header::HeaderMap,
        /// Held so dropping the server aborts the stream reader.
        reader: tokio::task::AbortHandle,
    },
}

/// A running server. Shared through `Arc` so the server map lock only
/// covers lookup — a slow request must never serialize `tool_specs`,
/// `running_tools` or other gate reads behind it.
pub struct RunningServer {
    pub transport: Transport,
    pub pending: Arc<Mutex<Pending>>,
    pub next_id: AtomicU64,
    pub tools: Vec<McpToolView>,
    /// Whether the user explicitly trusts this server's readOnlyHint.
    /// Atomic so `set_trust` can flip it while Arc clones are in flight.
    pub trust: AtomicBool,
    /// Set when the reader task died or the handshake/tool list failed.
    pub failed: Option<String>,
}

fn jsonrpc_error(value: &Value) -> Option<String> {
    value.get("error").map(|error| {
        error
            .get("message")
            .and_then(|message| message.as_str())
            .unwrap_or("MCP 调用失败")
            .to_string()
    })
}

/// Parse one JSON-RPC response out of either a plain JSON body or an SSE body
/// (`data:` frames). Returns the response Value whose `id` matches.
fn parse_http_body(content_type: &str, body: &[u8], id: u64) -> AppResult<Value> {
    if content_type.contains("text/event-stream") {
        let text = String::from_utf8_lossy(body);
        for block in text.split("\n\n") {
            for line in block.lines() {
                let Some(data) = line.strip_prefix("data:") else { continue };
                let Ok(value) = serde_json::from_str::<Value>(data.trim()) else { continue };
                if value.get("id").and_then(|v| v.as_u64()) == Some(id) {
                    if let Some(error) = jsonrpc_error(&value) {
                        return Err(error);
                    }
                    return Ok(value.get("result").cloned().unwrap_or(Value::Null));
                }
            }
        }
        Err("HTTP/SSE 响应中没有匹配的 JSON-RPC 应答".into())
    } else {
        let value: Value = serde_json::from_slice(body).map_err(|_| "无法解析 MCP HTTP 响应".to_string())?;
        if let Some(error) = jsonrpc_error(&value) {
            return Err(error);
        }
        Ok(value.get("result").cloned().unwrap_or(Value::Null))
    }
}

pub(crate) fn auth_hint(status: reqwest::StatusCode) -> Option<String> {
    match status.as_u16() {
        401 | 403 => Some("远端要求授权：请在请求头加 `Authorization: Bearer <token>`（GitHub MCP 用 PAT）".to_string()),
        _ => None,
    }
}

impl RunningServer {
    /// One JSON-RPC request/response round. `cancel` (the run's abort signal)
    /// interrupts the wait without waiting out `timeout` — a hung server can
    /// no longer pin a stopping run for the full call timeout. Cancellation
    /// removes the pending entry so `replies` never accumulates dead ids.
    pub async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: Option<watch::Receiver<bool>>,
    ) -> AppResult<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let payload = json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params,
        });
        if serde_json::to_string(&payload).map(|line| line.len()).unwrap_or(usize::MAX) > MAX_LINE_BYTES {
            return Err("MCP 请求过大".into());
        }
        match &self.transport {
            Transport::Stdio { stdin, .. } => self.stdio_request(id, payload, stdin, timeout, cancel).await,
            Transport::Http { .. } => self.http_request(id, payload, method, timeout, cancel).await,
            Transport::Sse { client, post_url, headers, .. } => self.sse_request(id, payload, client, post_url, headers, timeout, cancel).await,
        }
    }

    async fn stdio_request(
        &self,
        id: u64,
        payload: Value,
        stdin: &tokio::sync::Mutex<ChildStdin>,
        timeout: Duration,
        cancel: Option<watch::Receiver<bool>>,
    ) -> AppResult<Value> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().replies.insert(id, tx);
        let line = serde_json::to_string(&payload).map_err(|_| "无法序列化 MCP 请求".to_string())?;
        {
            let mut stdin = stdin.lock().await;
            if stdin.write_all(line.as_bytes()).await.is_err()
                || stdin.write_all(b"\n").await.is_err()
            {
                self.pending.lock().unwrap().replies.remove(&id);
                return Err("无法写入 MCP 子进程 stdin".into());
            }
        }
        let outcome = race_wait(rx, timeout, cancel, &format!("MCP {id}")).await;
        if outcome.is_err() {
            self.pending.lock().unwrap().replies.remove(&id);
        }
        outcome
    }

    /// One HTTP POST round: returns (status, content-type, body). Captures
    /// `Mcp-Session-Id` response headers into `session_id`.
    async fn http_send(
        &self,
        payload: &Value,
    ) -> AppResult<(reqwest::StatusCode, String, Vec<u8>)> {
        let Transport::Http { client, url, headers, session_id } = &self.transport else {
            unreachable!();
        };
        let mut request = client
            .post(url)
            .header(reqwest::header::ACCEPT, "application/json, text/event-stream")
            .header("MCP-Protocol-Version", PROTOCOL_VERSION)
            .headers(headers.clone())
            .json(payload);
        if let Some(session) = session_id.lock().await.clone() {
            request = request.header("Mcp-Session-Id", session);
        }
        let response = request.send().await.map_err(|error| format!("MCP HTTP 请求失败：{error}"))?;
        // initialize 的 session id 在响应头里，之后每个请求都要带上。
        if let Some(session) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
        {
            *session_id.lock().await = Some(session);
        }
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/json")
            .to_string();
        let body = response.bytes().await.map_err(|error| format!("MCP HTTP 读取失败：{error}"))?.to_vec();
        if body.len() > MAX_HTTP_BYTES {
            return Err("MCP HTTP 响应过大".into());
        }
        Ok((status, content_type, body))
    }

    async fn http_request(
        &self,
        id: u64,
        payload: Value,
        method: &str,
        timeout: Duration,
        cancel: Option<watch::Receiver<bool>>,
    ) -> AppResult<Value> {
        let method = method.to_string();
        let send = async {
            let mut attempt = self.http_send(&payload).await;
            // 会话被服务端回收（404）：重新 initialize 拿新 session 后重试一次。
            if attempt.as_ref().map(|(status, ..)| *status) == Ok(reqwest::StatusCode::NOT_FOUND)
                && method != "initialize"
            {
                let Transport::Http { session_id, .. } = &self.transport else { unreachable!() };
                *session_id.lock().await = None;
                let init_payload = json!({
                    "jsonrpc": "2.0", "id": self.next_id.fetch_add(1, Ordering::Relaxed) + 1,
                    "method": "initialize",
                    "params": { "protocolVersion": PROTOCOL_VERSION, "capabilities": {},
                        "clientInfo": { "name": "maestro", "version": env!("CARGO_PKG_VERSION") } },
                });
                let _ = self.http_send(&init_payload).await;
                attempt = self.http_send(&payload).await;
            }
            let (status, content_type, body) = attempt?;
            if let Some(hint) = auth_hint(status) {
                return Err(hint);
            }
            if status.as_u16() == 202 {
                return Ok(Value::Null);
            }
            if !status.is_success() {
                return Err(format!("MCP HTTP 响应 {status}"));
            }
            parse_http_body(&content_type, &body, id)
        };
        // HTTP 是同步请求/应答：不走 pending map，但同样要响应取消与超时。
        race_future(send, timeout, cancel, &format!("MCP {method}")).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn sse_request(
        &self,
        id: u64,
        payload: Value,
        client: &reqwest::Client,
        post_url: &str,
        headers: &reqwest::header::HeaderMap,
        timeout: Duration,
        cancel: Option<watch::Receiver<bool>>,
    ) -> AppResult<Value> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().replies.insert(id, tx);
        let sent = client.post(post_url).headers(headers.clone()).json(&payload).send().await;
        match sent {
            Err(error) => {
                self.pending.lock().unwrap().replies.remove(&id);
                return Err(format!("MCP SSE POST 失败：{error}"));
            }
            Ok(response) if response.status() == reqwest::StatusCode::UNAUTHORIZED || response.status() == reqwest::StatusCode::FORBIDDEN => {
                self.pending.lock().unwrap().replies.remove(&id);
                return Err(auth_hint(response.status()).unwrap());
            }
            Ok(response) if !response.status().is_success() && response.status().as_u16() != 202 => {
                self.pending.lock().unwrap().replies.remove(&id);
                return Err(format!("MCP SSE POST 响应 {}", response.status()));
            }
            _ => {}
        }
        let outcome = race_wait(rx, timeout, cancel, "MCP SSE 调用").await;
        if outcome.is_err() {
            self.pending.lock().unwrap().replies.remove(&id);
        }
        outcome
    }

    pub async fn notify(&self, method: &str, params: Value) -> AppResult<()> {
        let payload = json!({
            "jsonrpc": "2.0", "method": method, "params": params,
        });
        match &self.transport {
            Transport::Stdio { stdin, .. } => {
                let line = serde_json::to_string(&payload).map_err(|_| "无法序列化 MCP 通知".to_string())?;
                stdin
                    .lock()
                    .await
                    .write_all(format!("{line}\n").as_bytes())
                    .await
                    .map_err(|_| "无法写入 MCP 子进程 stdin".to_string())
            }
            Transport::Http { client, url, headers, session_id, .. } => {
                let mut request = client
                    .post(url)
                    .header(reqwest::header::ACCEPT, "application/json, text/event-stream")
                    .header("MCP-Protocol-Version", PROTOCOL_VERSION)
                    .headers(headers.clone())
                    .json(&payload);
                if let Some(session) = session_id.lock().await.clone() {
                    request = request.header("Mcp-Session-Id", session);
                }
                let response = request.send().await.map_err(|error| format!("MCP HTTP 通知失败：{error}"))?;
                if let Some(hint) = auth_hint(response.status()) {
                    return Err(hint);
                }
                if !response.status().is_success() && response.status().as_u16() != 202 {
                    return Err(format!("MCP HTTP 通知响应 {}", response.status()));
                }
                Ok(())
            }
            Transport::Sse { client, post_url, headers, .. } => {
                client
                    .post(post_url)
                    .headers(headers.clone())
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|error| format!("MCP SSE 通知失败：{error}"))?;
                Ok(())
            }
        }
    }

    /// Terminate the transport: kill the stdio child or abort the SSE reader.
    pub async fn shutdown(&self) {
        match &self.transport {
            Transport::Stdio { child, .. } => {
                let _ = child.lock().await.kill().await;
            }
            Transport::Sse { reader, .. } => reader.abort(),
            Transport::Http { .. } => {}
        }
    }

    /// Exit code of the stdio child once it is gone (after `shutdown` or a
    /// self-exit). Surfaces "killed by us" vs "died on its own" in errors.
    pub async fn exit_status(&self) -> Option<i32> {
        let Transport::Stdio { child, .. } = &self.transport else {
            return None;
        };
        match tokio::time::timeout(Duration::from_secs(2), child.lock().await.wait()).await {
            Ok(Ok(status)) => status.code(),
            _ => None,
        }
    }
}

/// Wait on a stdio/SSE oneshot reply with cancel + timeout.
async fn race_wait(
    rx: oneshot::Receiver<AppResult<Value>>,
    timeout: Duration,
    cancel: Option<watch::Receiver<bool>>,
    label: &str,
) -> AppResult<Value> {
    race_future(
        async move {
            match rx.await {
                Ok(result) => result,
                Err(_) => Err("MCP 服务连接已关闭".into()),
            }
        },
        timeout,
        cancel,
        label,
    )
    .await
}

async fn race_future<F: std::future::Future<Output = AppResult<Value>>>(
    future: F,
    timeout: Duration,
    cancel: Option<watch::Receiver<bool>>,
    label: &str,
) -> AppResult<Value> {
    let reply = tokio::time::timeout(timeout, future);
    match cancel {
        Some(mut cancel) => {
            // borrow() first: a receiver subscribed after send_replace
            // would otherwise miss the already-delivered signal.
            if *cancel.borrow() {
                Err("MCP 调用已取消".into())
            } else {
                tokio::select! {
                    biased;
                    _ = cancel.changed() => Err("MCP 调用已取消".into()),
                    result = reply => match result {
                        Ok(result) => result,
                        Err(_) => Err(format!("{label} 超时")),
                    },
                }
            }
        }
        None => match reply.await {
            Ok(result) => result,
            Err(_) => Err(format!("{label} 超时")),
        },
    }
}

/// Append a diagnostic line to a bounded tail buffer (same 4KB policy as the
/// stderr drain). Older bytes are dropped so the tail stays the newest.
fn push_diag(tail: &Arc<Mutex<String>>, text: &str) {
    let mut tail = tail.lock().unwrap();
    tail.push_str(text);
    if !text.ends_with('\n') {
        tail.push('\n');
    }
    if tail.len() > 4096 {
        let keep = tail.len() - 4096;
        tail.drain(..keep);
    }
}

/// Stdio reader task: route response lines to pending callers; ignore
/// notifications. Non-JSON stdout lines go to `diag` — several servers log
/// fatal errors via console.log, and dropping them made silent exits
/// undebuggable. Drains pending with "子进程已退出" when the process dies.
pub async fn read_loop(
    mut stdout: BufReader<tokio::process::ChildStdout>,
    pending: Arc<Mutex<Pending>>,
    diag: Arc<Mutex<String>>,
) {
    let mut line = String::new();
    loop {
        line.clear();
        match stdout.read_line(&mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if line.len() > MAX_LINE_BYTES {
                    push_diag(
                        &diag,
                        &format!("[丢弃超长 stdout 行：{} 字节]", line.len()),
                    );
                    break;
                }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(trimmed) {
                    Ok(value) => route_reply(&value, &pending),
                    Err(_) => {
                        // console.log / banner / fatal output — keep for error reports.
                        if trimmed.len() <= 2048 {
                            push_diag(&diag, trimmed);
                        } else {
                            push_diag(&diag, &format!("[非 JSON stdout 行：{} 字节]", trimmed.len()));
                        }
                    }
                }
            }
        }
    }
    // Fail every outstanding caller when the process exits.
    for (_id, tx) in pending.lock().unwrap().replies.drain() {
        let _ = tx.send(Err("MCP 子进程已退出".into()));
    }
}

fn route_reply(value: &Value, pending: &Arc<Mutex<Pending>>) {
    let Some(id) = value.get("id").and_then(|id| id.as_u64()) else {
        return;
    };
    let result = match jsonrpc_error(value) {
        Some(error) => Err(error),
        None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
    };
    if let Some(tx) = pending.lock().unwrap().replies.remove(&id) {
        let _ = tx.send(result);
    }
}

/// Legacy SSE reader task: consumes the GET stream. The first `endpoint`
/// event resolves the POST URL (reported via `endpoint_tx`); every `data:`
/// frame afterwards is a JSON-RPC payload routed by id to `pending`.
pub async fn sse_read_loop(
    response: reqwest::Response,
    base_url: String,
    pending: Arc<Mutex<Pending>>,
    endpoint_tx: oneshot::Sender<AppResult<String>>,
) {
    use futures_util::StreamExt;
    let mut endpoint_tx = Some(endpoint_tx);
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    'outer: while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else { break };
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        // SSE 帧以空行分隔；`data:` 行承载 JSON-RPC 应答。
        while let Some(end) = buffer.find("\n\n") {
            let frame = buffer[..end].to_string();
            buffer.drain(..end + 2);
            let mut event = String::new();
            let mut datas: Vec<Value> = Vec::new();
            let mut endpoint_data: Option<String> = None;
            for line in frame.lines() {
                if let Some(rest) = line.strip_prefix("event:") {
                    event = rest.trim().to_string();
                } else if let Some(rest) = line.strip_prefix("data:") {
                    if event == "endpoint" {
                        endpoint_data = Some(rest.trim().to_string());
                    } else if let Ok(value) = serde_json::from_str::<Value>(rest.trim()) {
                        datas.push(value);
                    }
                }
            }
            if let Some(tx) = endpoint_tx.take() {
                if event == "endpoint" {
                    let result = endpoint_data
                        .filter(|data| !data.is_empty())
                        .ok_or_else(|| "MCP SSE endpoint 事件为空".to_string())
                        .and_then(|data| resolve_endpoint(&base_url, &data));
                    let _ = tx.send(result);
                }
            }
            for value in datas {
                route_reply(&value, &pending);
            }
            if buffer.len() > MAX_HTTP_BYTES {
                break 'outer;
            }
        }
    }
    if let Some(tx) = endpoint_tx.take() {
        let _ = tx.send(Err("MCP SSE 流在 endpoint 事件前结束".into()));
    }
    for (_id, tx) in pending.lock().unwrap().replies.drain() {
        let _ = tx.send(Err("MCP SSE 连接已断开".into()));
    }
}

fn resolve_endpoint(base: &str, endpoint: &str) -> AppResult<String> {
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        return Ok(endpoint.to_string());
    }
    // 相对路径（如 `/messages?session_id=…`）按 base 的 origin 拼。
    let base = reqwest::Url::parse(base).map_err(|_| format!("非法 MCP URL：{base}"))?;
    base.join(endpoint)
        .map(|url| url.to_string())
        .map_err(|error| format!("无法解析 MCP SSE endpoint：{error}"))
}
