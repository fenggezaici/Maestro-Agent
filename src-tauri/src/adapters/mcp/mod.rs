//! Minimal stdio MCP client: newline-delimited JSON-RPC 2.0 over child process
//! pipes. Only `initialize`, `notifications/initialized`, `tools/list` and
//! `tools/call` are implemented; no external MCP SDK dependency.
mod config;
mod tools;
mod transport;

use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc, Mutex},
};

use serde_json::{json, Value};
use tokio::{
    io::BufReader,
    process::Command,
    sync::Mutex as AsyncMutex,
};

use crate::domain::{
    AppResult, McpConfigFile, McpServerConfig, McpServerView, RecommendedItem, ToolSpec,
};

use config::{
    load_config, save_config, valid_server_config, valid_server_name,
};
use tools::{call_result_text, parse_tools, split_mcp_tool};
use transport::{
    read_loop, Pending, RunningServer, CALL_TIMEOUT, HANDSHAKE_TIMEOUT, PROTOCOL_VERSION,
};

pub use tools::split_mcp_tool as split_tool_name;

const MAX_SERVERS: usize = 32;

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

    /// Every exposed tool across running servers as standard `ToolSpec`s,
    /// namespaced `mcp__server__tool` and carrying the real `inputSchema`.
    pub async fn tool_specs(&self) -> Vec<ToolSpec> {
        self.running
            .lock()
            .await
            .iter()
            .flat_map(|(server_name, server)| {
                server.tools.iter().map(move |tool| ToolSpec {
                    name: format!("mcp__{server_name}__{}", tool.name),
                    description: tool.description.clone(),
                    parameters: tool.input_schema.clone(),
                })
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
        call_result_text(&result)
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
}
