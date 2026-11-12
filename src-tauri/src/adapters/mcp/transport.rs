//! Stdio transport: child-process pipes carrying newline-delimited JSON-RPC 2.0.
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin},
    sync::oneshot,
};

use crate::domain::{AppResult, McpToolView};

pub const PROTOCOL_VERSION: &str = "2025-03-26";
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_LINE_BYTES: usize = 1048576;

pub struct Pending {
    pub replies: HashMap<u64, oneshot::Sender<AppResult<Value>>>,
}

pub struct RunningServer {
    pub child: Child,
    pub stdin: ChildStdin,
    pub pending: Arc<Mutex<Pending>>,
    pub next_id: AtomicU64,
    pub tools: Vec<McpToolView>,
    /// Whether the user explicitly trusts this server's readOnlyHint.
    pub trust: bool,
    /// Set when the reader task died or the handshake/tool list failed.
    pub failed: Option<String>,
}

impl RunningServer {
    pub async fn request(&mut self, method: &str, params: Value, timeout: Duration) -> AppResult<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().replies.insert(id, tx);
        let line = serde_json::to_string(&json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params,
        }))
        .map_err(|_| "无法序列化 MCP 请求".to_string())?;
        if line.len() > MAX_LINE_BYTES {
            return Err("MCP 请求过大".into());
        }
        if let Err(_error) = self.stdin.write_all(line.as_bytes()).await {
            self.pending.lock().unwrap().replies.remove(&id);
            return Err("无法写入 MCP 子进程 stdin".into());
        }
        if let Err(_error) = self.stdin.write_all(b"\n").await {
            self.pending.lock().unwrap().replies.remove(&id);
            return Err("无法写入 MCP 子进程 stdin".into());
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("MCP 子进程连接已关闭".into()),
            Err(_) => {
                self.pending.lock().unwrap().replies.remove(&id);
                Err(format!("MCP {method} 超时"))
            }
        }
    }

    pub async fn notify(&mut self, method: &str, params: Value) -> AppResult<()> {
        let line = serde_json::to_string(&json!({
            "jsonrpc": "2.0", "method": method, "params": params,
        }))
        .map_err(|_| "无法序列化 MCP 通知".to_string())?;
        self.stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .map_err(|_| "无法写入 MCP 子进程 stdin".to_string())
    }
}

/// Reader task: route response lines to pending callers; ignore notifications.
pub async fn read_loop(mut stdout: BufReader<tokio::process::ChildStdout>, pending: Arc<Mutex<Pending>>) {
    let mut line = String::new();
    loop {
        line.clear();
        match stdout.read_line(&mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if line.len() > MAX_LINE_BYTES {
                    break;
                }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
                    continue;
                };
                let Some(id) = value.get("id").and_then(|id| id.as_u64()) else {
                    continue;
                };
                let result = if let Some(error) = value.get("error") {
                    Err(error
                        .get("message")
                        .and_then(|message| message.as_str())
                        .unwrap_or("MCP 调用失败")
                        .to_string())
                } else {
                    Ok(value.get("result").cloned().unwrap_or(Value::Null))
                };
                if let Some(tx) = pending.lock().unwrap().replies.remove(&id) {
                    let _ = tx.send(result);
                }
            }
        }
    }
    // Fail every outstanding caller when the process exits.
    for (_id, tx) in pending.lock().unwrap().replies.drain() {
        let _ = tx.send(Err("MCP 子进程已退出".into()));
    }
}
