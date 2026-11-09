//! Minimal stdio MCP client: newline-delimited JSON-RPC 2.0 over child process
//! pipes. Only `initialize`, `notifications/initialized`, `tools/list` and
//! `tools/call` are implemented; no external MCP SDK dependency.
use std::{
    collections::BTreeMap,
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{oneshot, Mutex as AsyncMutex},
};

use crate::{
    extensions::skills::RecommendedItem,
    model::AppResult,
};

const PROTOCOL_VERSION: &str = "2025-03-26";
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_LINE_BYTES: usize = 1048576;
const MAX_TOOL_RESULT_BYTES: usize = 65536;
const MAX_SERVERS: usize = 32;
const MAX_TOOLS_PER_SERVER: usize = 256;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConfigFile {
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolView {
    pub name: String,
    pub description: String,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerView {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub disabled: bool,
    pub running: bool,
    pub tool_count: usize,
    pub error: Option<String>,
}

pub fn valid_server_name(name: &str) -> AppResult<&str> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.len() > 64
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("MCP 服务名必须为 1-64 位字母、数字、下划线或连字符".into());
    }
    Ok(trimmed)
}

fn valid_server_config(name: &str, config: &McpServerConfig) -> AppResult<McpServerConfig> {
    valid_server_name(name)?;
    if config.command.trim().is_empty() || config.command.len() > 1024 {
        return Err("MCP command 为空或过长".into());
    }
    if config.args.len() > 64 || config.args.iter().any(|arg| arg.len() > 1024) {
        return Err("MCP args 超过数量/长度限制".into());
    }
    if config.env.len() > 64
        || config
            .env
            .iter()
            .any(|(key, value)| key.is_empty() || key.len() > 128 || value.len() > 4096)
    {
        return Err("MCP env 超过数量/长度限制".into());
    }
    Ok(config.clone())
}

/// Read `mcp.json`; a missing file yields an empty config, a corrupt file errors.
pub fn load_config(path: &PathBuf) -> AppResult<McpConfigFile> {
    if !path.is_file() {
        return Ok(McpConfigFile::default());
    }
    let content = fs::read_to_string(path).map_err(|_| "无法读取 mcp.json".to_string())?;
    serde_json::from_str(&content).map_err(|_| "mcp.json 格式无效".to_string())
}

fn save_config(path: &PathBuf, config: &McpConfigFile) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "无法创建配置目录".to_string())?;
    }
    let content = serde_json::to_string_pretty(config).map_err(|_| "无法序列化配置".to_string())?;
    fs::write(path, content).map_err(|_| "无法写入 mcp.json".to_string())
}

/// Split `mcp__<server>__<tool>` into (server, tool); returns None otherwise.
pub fn split_mcp_tool(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    if server.is_empty() || tool.is_empty() || tool.contains("__") {
        return None;
    }
    Some((server, tool))
}

struct Pending {
    replies: HashMap<u64, oneshot::Sender<AppResult<Value>>>,
}

struct RunningServer {
    child: Child,
    stdin: ChildStdin,
    pending: Arc<Mutex<Pending>>,
    next_id: AtomicU64,
    tools: Vec<McpToolView>,
    /// Set when the reader task died or the handshake/tool list failed.
    failed: Option<String>,
}

impl RunningServer {
    async fn request(&mut self, method: &str, params: Value, timeout: Duration) -> AppResult<Value> {
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

    async fn notify(&mut self, method: &str, params: Value) -> AppResult<()> {
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
async fn read_loop(mut stdout: BufReader<tokio::process::ChildStdout>, pending: Arc<Mutex<Pending>>) {
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

#[derive(Default)]
pub struct McpManager {
    config_path: Mutex<Option<PathBuf>>,
    running: AsyncMutex<HashMap<String, RunningServer>>,
    /// Last startup error per server, surfaced in list_mcp_servers.
    errors: Mutex<HashMap<String, String>>,
}

impl McpManager {
    pub fn set_config_path(&self, path: PathBuf) {
        *self.config_path.lock().unwrap() = Some(path);
    }

    fn config_path(&self) -> AppResult<PathBuf> {
        self.config_path
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "MCP 配置目录未初始化".to_string())
    }

    fn load(&self) -> AppResult<McpConfigFile> {
        load_config(&self.config_path()?)
    }

    fn save(&self, config: &McpConfigFile) -> AppResult<()> {
        save_config(&self.config_path()?, config)
    }

    pub fn list(&self) -> AppResult<Vec<McpServerView>> {
        let config = self.load()?;
        let errors = self.errors.lock().unwrap();
        // Async mutex cannot be held here synchronously; collect running names first.
        let running_names: Vec<String> = self
            .running
            .try_lock()
            .map(|servers| servers.keys().cloned().collect())
            .unwrap_or_default();
        Ok(config
            .mcp_servers
            .iter()
            .map(|(name, server)| {
                let running = running_names.contains(name);
                McpServerView {
                    name: name.clone(),
                    command: server.command.clone(),
                    args: server.args.clone(),
                    disabled: server.disabled,
                    running,
                    tool_count: 0, // Filled by the async list_servers caller.
                    error: errors.get(name).cloned(),
                }
            })
            .collect())
    }

    pub async fn list_servers(&self) -> AppResult<Vec<McpServerView>> {
        let mut views = self.list()?;
        let running = self.running.lock().await;
        for view in &mut views {
            if let Some(server) = running.get(&view.name) {
                view.running = true;
                view.tool_count = server.tools.len();
                if let Some(error) = &server.failed {
                    view.error = Some(error.clone());
                }
            }
        }
        Ok(views)
    }

    /// Merge a config entry (from UI or imported JSON) into mcp.json.
    pub fn upsert_server(&self, name: &str, config: McpServerConfig) -> AppResult<()> {
        let name = valid_server_name(name)?.to_string();
        let config = valid_server_config(&name, &config)?;
        let mut file = self.load()?;
        if file.mcp_servers.len() >= MAX_SERVERS && !file.mcp_servers.contains_key(&name) {
            return Err("MCP 服务数量超过上限".into());
        }
        file.mcp_servers.insert(name, config);
        self.save(&file)
    }

    pub async fn remove_server(&self, name: &str) -> AppResult<()> {
        self.stop(name).await?;
        let mut file = self.load()?;
        file.mcp_servers.remove(name);
        self.save(&file)
    }

    pub fn import_json(&self, json_path: &str) -> AppResult<usize> {
        let source = std::path::Path::new(json_path)
            .canonicalize()
            .map_err(|_| "导入的 JSON 文件不存在".to_string())?;
        let content = fs::read_to_string(&source).map_err(|_| "无法读取导入文件".to_string())?;
        let file: McpConfigFile =
            serde_json::from_str(&content).map_err(|_| "导入文件不是有效的 MCP 配置".to_string())?;
        if file.mcp_servers.is_empty() {
            return Err("导入文件中没有 mcpServers 条目".into());
        }
        let mut merged = self.load()?;
        let mut count = 0;
        for (name, server) in &file.mcp_servers {
            let config = valid_server_config(name, server)?;
            if merged.mcp_servers.len() >= MAX_SERVERS && !merged.mcp_servers.contains_key(name) {
                return Err("MCP 服务数量超过上限".into());
            }
            merged.mcp_servers.insert(name.clone(), config);
            count += 1;
        }
        self.save(&merged)?;
        Ok(count)
    }

    pub async fn start(&self, name: &str) -> AppResult<McpServerView> {
        let name = valid_server_name(name)?.to_string();
        let config = {
            let file = self.load()?;
            let server = file
                .mcp_servers
                .get(&name)
                .ok_or_else(|| format!("未配置的 MCP 服务：{name}"))?;
            valid_server_config(&name, server)?
        };
        if config.disabled {
            return Err(format!("MCP 服务 {name} 已被标记为禁用"));
        }
        // Already running: report current state.
        {
            let running = self.running.lock().await;
            if let Some(server) = running.get(&name) {
                return Ok(McpServerView {
                    name,
                    command: config.command,
                    args: config.args,
                    disabled: false,
                    running: true,
                    tool_count: server.tools.len(),
                    error: server.failed.clone(),
                });
            }
        }
        match self.spawn(&name, &config).await {
            Ok(server) => {
                let tools = server.tools.len();
                let failed = server.failed.clone();
                self.running.lock().await.insert(name.clone(), server);
                self.errors.lock().unwrap().remove(&name);
                Ok(McpServerView {
                    name,
                    command: config.command,
                    args: config.args,
                    disabled: false,
                    running: true,
                    tool_count: tools,
                    error: failed,
                })
            }
            Err(error) => {
                self.errors.lock().unwrap().insert(name.clone(), error.clone());
                Err(error)
            }
        }
    }

    async fn spawn(&self, name: &str, config: &McpServerConfig) -> AppResult<RunningServer> {
        let mut child = Command::new(config.command.trim())
            .args(&config.args)
            .envs(&config.env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| format!("无法启动 MCP 服务 {name}（{}）", config.command))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "无法获取 MCP 子进程 stdout".to_string())?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "无法获取 MCP 子进程 stdin".to_string())?;
        let pending = Arc::new(Mutex::new(Pending {
            replies: HashMap::new(),
        }));
        tokio::spawn(read_loop(BufReader::new(stdout), pending.clone()));
        let mut server = RunningServer {
            child,
            stdin,
            pending,
            next_id: AtomicU64::new(0),
            tools: Vec::new(),
            failed: None,
        };
        let handshake = async {
            server
                .request(
                    "initialize",
                    json!({
                        "protocolVersion": PROTOCOL_VERSION,
                        "capabilities": {},
                        "clientInfo": { "name": "maestro", "version": env!("CARGO_PKG_VERSION") },
                    }),
                    HANDSHAKE_TIMEOUT,
                )
                .await?;
            server
                .notify("notifications/initialized", json!({}))
                .await?;
            let listed = server
                .request("tools/list", json!({}), HANDSHAKE_TIMEOUT)
                .await?;
            Ok::<_, String>(listed)
        };
        match handshake.await {
            Ok(listed) => {
                server.tools = parse_tools(&listed);
                Ok(server)
            }
            Err(error) => {
                let _ = server.child.kill().await;
                Err(error)
            }
        }
    }

    pub async fn stop(&self, name: &str) -> AppResult<()> {
        if let Some(mut server) = self.running.lock().await.remove(name) {
            for (_id, tx) in server.pending.lock().unwrap().replies.drain() {
                let _ = tx.send(Err("MCP 服务已停止".into()));
            }
            let _ = server.child.kill().await;
        }
        Ok(())
    }

    /// Every exposed tool across running servers, namespaced `mcp__server__tool`.
    pub async fn tool_definitions(&self) -> Vec<(String, String, Value, bool)> {
        self.running
            .lock()
            .await
            .iter()
            .flat_map(|(server_name, server)| {
                server
                    .tools
                    .iter()
                    .map(move |tool| {
                        (
                            format!("mcp__{server_name}__{}", tool.name),
                            tool.description.clone(),
                            Value::Null,
                            tool.read_only,
                        )
                    })
            })
            .collect()
    }

    pub async fn tool_schemas(&self) -> Vec<Value> {
        self.running
            .lock()
            .await
            .iter()
            .flat_map(|(server_name, server)| {
                server.tools.iter().map(move |tool| {
                    (
                        format!("mcp__{server_name}__{}", tool.name),
                        tool.description.clone(),
                        tool.read_only,
                    )
                })
            })
            .map(|(name, description, _read_only)| {
                json!({ "name": name, "description": description })
            })
            .collect()
    }

    pub async fn call_tool(&self, qualified: &str, arguments: Value) -> AppResult<String> {
        let (server_name, tool_name) =
            split_mcp_tool(qualified).ok_or_else(|| format!("非法 MCP 工具名：{qualified}"))?;
        let mut running = self.running.lock().await;
        let server = running
            .get_mut(server_name)
            .ok_or_else(|| format!("MCP 服务 {server_name} 未在运行"))?;
        if !server.tools.iter().any(|tool| tool.name == tool_name) {
            return Err(format!("MCP 服务 {server_name} 没有工具 {tool_name}"));
        }
        let result = server
            .request(
                "tools/call",
                json!({ "name": tool_name, "arguments": arguments }),
                CALL_TIMEOUT,
            )
            .await?;
        Ok(tool_result_text(&result))
    }

    /// Report the readOnlyHint classification for authorization checks.
    pub async fn tool_is_read_only(&self, qualified: &str) -> bool {
        match split_mcp_tool(qualified) {
            Some((server_name, tool_name)) => self
                .running
                .lock()
                .await
                .get(server_name)
                .and_then(|server| server.tools.iter().find(|tool| tool.name == tool_name))
                .map(|tool| tool.read_only)
                .unwrap_or(false),
            None => false,
        }
    }

    pub async fn shutdown(&self) {
        let mut running = self.running.lock().await;
        for (_name, mut server) in running.drain() {
            let _ = server.child.kill().await;
        }
    }
}

fn parse_tools(listed: &Value) -> Vec<McpToolView> {
    listed
        .get("tools")
        .and_then(|tools| tools.as_array())
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| {
                    let name = tool.get("name")?.as_str()?.to_string();
                    if name.len() > 128 {
                        return None;
                    }
                    Some(McpToolView {
                        name,
                        description: tool
                            .get("description")
                            .and_then(|d| d.as_str())
                            .unwrap_or_default()
                            .chars()
                            .take(512)
                            .collect(),
                        read_only: tool
                            .pointer("/annotations/readOnlyHint")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                    })
                })
                .take(MAX_TOOLS_PER_SERVER)
                .collect()
        })
        .unwrap_or_default()
}

/// Flatten an MCP tools/call result into bounded display text.
fn tool_result_text(result: &Value) -> String {
    let text = result
        .get("content")
        .and_then(|content| content.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(|text| text.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|joined| !joined.is_empty())
        .unwrap_or_else(|| serde_json::to_string(result).unwrap_or_default());
    text.chars().take(MAX_TOOL_RESULT_BYTES).collect()
}

/// Built-in community recommendations; `install_payload` is a mcp.json entry.
pub fn recommended_mcps() -> Vec<RecommendedItem> {
    let entry = |id: &str, name: &str, description: &str, url: &str, command: &str, args: &[&str]| {
        RecommendedItem {
            id: id.into(),
            kind: "mcp".into(),
            name: name.into(),
            description: description.into(),
            source_url: url.into(),
            install_payload: serde_json::to_string(&serde_json::json!({
                "command": command,
                "args": args,
            }))
            .unwrap_or_default(),
        }
    };
    vec![
        entry("filesystem", "FileSystem", "官方文件系统服务：受控目录内的文件读写与目录遍历。", "https://github.com/modelcontextprotocol/servers", "npx", &["-y", "@modelcontextprotocol/server-filesystem"]),
        entry("github", "GitHub", "官方 GitHub 服务：Issue、PR、仓库内容的查询与管理。", "https://github.com/modelcontextprotocol/servers", "npx", &["-y", "@modelcontextprotocol/server-github"]),
        entry("postgres", "PostgreSQL", "官方 PostgreSQL 服务：只读查询与 schema 检查。", "https://github.com/modelcontextprotocol/servers", "npx", &["-y", "@modelcontextprotocol/server-postgres"]),
        entry("puppeteer", "Puppeteer", "官方浏览器自动化服务：页面导航、截图与数据抓取。", "https://github.com/modelcontextprotocol/servers", "npx", &["-y", "@modelcontextprotocol/server-puppeteer"]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_tool_names_split_once() {
        assert_eq!(split_mcp_tool("mcp__fs__read_file"), Some(("fs", "read_file")));
        assert_eq!(split_mcp_tool("mcp__a-b_c__t"), Some(("a-b_c", "t")));
        assert_eq!(split_mcp_tool("load_skill"), None);
        assert_eq!(split_mcp_tool("mcp__fs"), None);
        assert_eq!(split_mcp_tool("mcp____x"), None);
        assert_eq!(split_mcp_tool("mcp__fs__a__b"), None);
    }

    #[test]
    fn config_roundtrips_and_validates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        assert!(load_config(&path).unwrap().mcp_servers.is_empty());
        fs::write(&path, r#"{"mcpServers":{"fs":{"command":"npx","args":["-y","server-fs"]}}}"#).unwrap();
        let file = load_config(&path).unwrap();
        assert_eq!(file.mcp_servers["fs"].args.len(), 2);
        fs::write(&path, "not json").unwrap();
        assert!(load_config(&path).is_err());
        assert!(valid_server_name("bad name!").is_err());
        assert!(valid_server_name("good-name_1").is_ok());
    }

    #[test]
    fn import_merges_validated_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        let manager = McpManager::default();
        manager.set_config_path(path);
        let source = dir.path().join("import.json");
        fs::write(&source, r#"{"mcpServers":{"a":{"command":"node","args":["s.js"]},"b":{"command":"python"}}}"#).unwrap();
        assert_eq!(manager.import_json(source.to_str().unwrap()).unwrap(), 2);
        let file = manager.load().unwrap();
        assert!(file.mcp_servers.contains_key("a") && file.mcp_servers.contains_key("b"));
        fs::write(&source, r#"{"mcpServers":{"bad name!":{"command":"x"}}}"#).unwrap();
        assert!(manager.import_json(source.to_str().unwrap()).is_err());
    }

    #[test]
    fn tool_result_text_flattens_content_blocks() {
        let result = json!({ "content": [{"type":"text","text":"hello"},{"type":"text","text":"world"}] });
        assert_eq!(tool_result_text(&result), "hello\nworld");
        let fallback = json!({ "structured": true });
        assert!(tool_result_text(&fallback).contains("structured"));
    }

    #[test]
    fn parses_tools_with_read_only_annotations() {
        let listed = json!({ "tools": [
            {"name":"read_file","description":"reads","annotations":{"readOnlyHint":true}},
            {"name":"write_file","description":"writes"},
        ]});
        let tools = parse_tools(&listed);
        assert_eq!(tools.len(), 2);
        assert!(tools[0].read_only && !tools[1].read_only);
        assert!(parse_tools(&json!({})).is_empty());
    }
}
