//! Minimal stdio MCP client: newline-delimited JSON-RPC 2.0 over child process
//! pipes. Only `initialize`, `notifications/initialized`, `tools/list` and
//! `tools/call` are implemented; no external MCP SDK dependency.
mod config;
mod tools;
mod transport;

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
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

/// Windows cannot CreateProcess a `.cmd`/`.bat` shim, so resolve the real
/// launcher through PATHEXT and report whether it must run under `cmd /C`.
/// `workspace_root` mirrors exec::resolve_program: a PATH directory inside the
/// workspace must never supply the launcher — a cloned repo could plant
/// `npx.cmd`/`node.exe` and hijack every MCP server start.
#[cfg(windows)]
fn resolve_program(command: &str, path_var: Option<&std::ffi::OsStr>, workspace_root: Option<&Path>) -> (PathBuf, bool) {
    let direct = PathBuf::from(command);
    let needs_shell = |path: &PathBuf| {
        path.extension()
            .map(|ext| matches!(ext.to_string_lossy().to_ascii_lowercase().as_str(), "cmd" | "bat"))
            .unwrap_or(false)
    };
    if direct.extension().is_some() {
        let shell = needs_shell(&direct);
        return (direct, shell);
    }
    let root = workspace_root.and_then(|root| root.canonicalize().ok());
    if let Some(path) = path_var {
        for dir in std::env::split_paths(path) {
            if dir.as_os_str().is_empty() {
                // An empty PATH entry means "current directory" on Windows.
                continue;
            }
            if let Some(root) = &root {
                let inside_workspace = dir
                    .canonicalize()
                    .map(|resolved| resolved.starts_with(root))
                    .unwrap_or(false);
                if inside_workspace {
                    continue;
                }
            }
            for extension in ["exe", "cmd", "bat"] {
                let candidate = dir.join(format!("{command}.{extension}"));
                if candidate.is_file() {
                    let shell = needs_shell(&candidate);
                    return (candidate, shell);
                }
            }
        }
    }
    (direct, false)
}

#[cfg(not(windows))]
fn resolve_program(command: &str, _path_var: Option<&std::ffi::OsStr>) -> (PathBuf, bool) {
    (PathBuf::from(command), false)
}

#[derive(Default)]
pub struct McpManager {
    config_path: Mutex<Option<PathBuf>>,
    // Servers are shared by Arc: the map lock only covers lookup, so a slow
    // request never serializes tool_specs/running_tools/gate reads behind it.
    running: AsyncMutex<HashMap<String, Arc<RunningServer>>>,
    /// Last startup error per server, surfaced in list_mcp_servers.
    errors: Mutex<HashMap<String, String>>,
    /// Current workspace root: PATH directories inside it are excluded from
    /// launcher resolution (repo-planted `npx.cmd` hijack prevention).
    workspace_root: Mutex<Option<PathBuf>>,
}

impl McpManager {
    pub fn set_config_path(&self, path: PathBuf) {
        *self.config_path.lock().unwrap() = Some(path);
    }

    pub fn set_workspace_root(&self, root: Option<PathBuf>) {
        *self.workspace_root.lock().unwrap() = root;
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
                    env: server.env.clone(),
                    disabled: server.disabled,
                    auto_start: server.auto_start,
                    trust: server.trust,
                    running,
                    tool_count: 0, // Filled by the async list_servers caller.
                    tools: Vec::new(),
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
                view.tools = server.tools.clone();
                if let Some(error) = &server.failed {
                    view.error = Some(error.clone());
                }
            }
        }
        Ok(views)
    }

    /// 运行中 Server -> 已列出工具名（架构求值快照用）。
    pub async fn running_tools(&self) -> std::collections::BTreeMap<String, Vec<String>> {
        self.running
            .lock()
            .await
            .iter()
            .map(|(name, server)| {
                (
                    name.clone(),
                    server.tools.iter().map(|tool| tool.name.clone()).collect(),
                )
            })
            .collect()
    }

    /// Merge a config entry (from UI or imported JSON) into mcp.json.
    pub fn upsert_server(&self, name: &str, mut config: McpServerConfig) -> AppResult<()> {
        let name = valid_server_name(name)?.to_string();
        config = valid_server_config(&name, &config)?;
        let mut file = self.load()?;
        if file.mcp_servers.len() >= MAX_SERVERS && !file.mcp_servers.contains_key(&name) {
            return Err("MCP 服务数量超过上限".into());
        }
        // Editing must not silently re-enable a server the user turned off.
        let disabled = file.mcp_servers.get(&name).map(|server| server.disabled).unwrap_or(false);
        let auto_start = file.mcp_servers.get(&name).map(|server| server.auto_start).unwrap_or(config.auto_start);
        config.disabled = disabled;
        config.auto_start = auto_start;
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
                    env: config.env,
                    disabled: false,
                    auto_start: config.auto_start,
                    trust: config.trust,
                    running: true,
                    tool_count: server.tools.len(),
                    tools: server.tools.clone(),
                    error: server.failed.clone(),
                });
            }
        }
        match self.spawn(&name, &config).await {
            Ok(server) => {
                let tools = server.tools.clone();
                let failed = server.failed.clone();
                self.running
                    .lock()
                    .await
                    .insert(name.clone(), Arc::new(server));
                self.errors.lock().unwrap().remove(&name);
                Ok(McpServerView {
                    name,
                    command: config.command,
                    args: config.args,
                    env: config.env,
                    disabled: false,
                    auto_start: config.auto_start,
                    trust: config.trust,
                    running: true,
                    tool_count: tools.len(),
                    tools,
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
        let path_var = std::env::var_os("PATH");
        let workspace_root = self.workspace_root.lock().unwrap().clone();
        let (program, needs_shell) = resolve_program(config.command.trim(), path_var.as_deref(), workspace_root.as_deref());
        let mut builder = if needs_shell {
            let mut command = Command::new("cmd");
            command.arg("/C").arg(&program);
            command
        } else {
            Command::new(&program)
        };
        let mut child = builder
            .args(&config.args)
            .envs(&config.env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| format!("无法启动 MCP 服务 {name}（{}）：{error}", config.command))?;
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
            child: AsyncMutex::new(child),
            stdin: AsyncMutex::new(stdin),
            pending,
            next_id: AtomicU64::new(0),
            tools: Vec::new(),
            trust: AtomicBool::new(config.trust),
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
                    None,
                )
                .await?;
            server
                .notify("notifications/initialized", json!({}))
                .await?;
            let listed = server
                .request("tools/list", json!({}), HANDSHAKE_TIMEOUT, None)
                .await?;
            Ok::<_, String>(listed)
        };
        match handshake.await {
            Ok(listed) => {
                server.tools = parse_tools(&listed);
                Ok(server)
            }
            Err(error) => {
                let _ = server.child.lock().await.kill().await;
                Err(error)
            }
        }
    }

    pub async fn stop(&self, name: &str) -> AppResult<()> {
        if let Some(server) = self.running.lock().await.remove(name) {
            for (_id, tx) in server.pending.lock().unwrap().replies.drain() {
                let _ = tx.send(Err("MCP 服务已停止".into()));
            }
            let _ = server.child.lock().await.kill().await;
        }
        Ok(())
    }

    fn set_enabled(&self, name: &str, enabled: bool) -> AppResult<()> {
        let name = valid_server_name(name)?.to_string();
        let mut file = self.load()?;
        let server = file
            .mcp_servers
            .get_mut(&name)
            .ok_or_else(|| format!("未配置的 MCP 服务：{name}"))?;
        server.disabled = !enabled;
        server.auto_start = enabled;
        self.save(&file)
    }

    /// Explicit enable from the UI: persist the intent, then start the child.
    pub async fn enable(&self, name: &str) -> AppResult<McpServerView> {
        self.set_enabled(name, true)?;
        self.start(name).await
    }

    /// Explicit disable from the UI: persist the intent, then stop the child.
    pub async fn disable(&self, name: &str) -> AppResult<McpServerView> {
        self.set_enabled(name, false)?;
        self.stop(name).await?;
        self.list_servers()
            .await?
            .into_iter()
            .find(|server| server.name == name)
            .ok_or_else(|| format!("未配置的 MCP 服务：{name}"))
    }

    /// Startup restore: start every configured server the user left enabled.
    /// Failures stay per-server in `errors` and never block the app.
    pub async fn start_enabled(&self) {
        let names: Vec<String> = match self.load() {
            Ok(file) => file
                .mcp_servers
                .iter()
                .filter(|(_, server)| server.auto_start && !server.disabled)
                .map(|(name, _)| name.clone())
                .collect(),
            Err(_) => return,
        };
        for name in names {
            let _ = self.start(&name).await;
        }
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

    pub async fn call_tool(
        &self,
        qualified: &str,
        arguments: Value,
        cancel: Option<tokio::sync::watch::Receiver<bool>>,
    ) -> AppResult<String> {
        let (server_name, tool_name) =
            split_mcp_tool(qualified).ok_or_else(|| format!("非法 MCP 工具名：{qualified}"))?;
        // Clone the Arc and drop the map lock before awaiting the request —
        // a hung server must not serialize unrelated gate reads, and the
        // run's cancel signal reaches the wait directly.
        let server = self
            .running
            .lock()
            .await
            .get(server_name)
            .cloned()
            .ok_or_else(|| format!("MCP 服务 {server_name} 未在运行"))?;
        if !server.tools.iter().any(|tool| tool.name == tool_name) {
            return Err(format!("MCP 服务 {server_name} 没有工具 {tool_name}"));
        }
        let result = server
            .request(
                "tools/call",
                json!({ "name": tool_name, "arguments": arguments }),
                CALL_TIMEOUT,
                cancel,
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
                .and_then(|server| {
                    server
                        .tools
                        .iter()
                        .find(|tool| tool.name == tool_name)
                        .map(|tool| (server.trust.load(Ordering::Relaxed), tool.read_only))
                })
                .map(|(trust, read_only)| tools::effective_read_only(trust, read_only))
                .unwrap_or(false),
            None => false,
        }
    }

    /// Toggle whether this server's `readOnlyHint` may be honored.
    pub async fn set_trust(&self, name: &str, trust: bool) -> AppResult<()> {
        let name = valid_server_name(name)?.to_string();
        let mut file = self.load()?;
        let server = file
            .mcp_servers
            .get_mut(&name)
            .ok_or_else(|| format!("MCP 服务 {name} 不存在"))?;
        server.trust = trust;
        self.save(&file)?;
        if let Some(running) = self.running.lock().await.get(&name) {
            running.trust.store(trust, Ordering::Relaxed);
        }
        Ok(())
    }

    pub async fn shutdown(&self) {
        let mut running = self.running.lock().await;
        for (_name, server) in running.drain() {
            let _ = server.child.lock().await.kill().await;
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

    #[cfg(windows)]
    #[test]
    fn resolves_cmd_shims_through_path() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("fake-npx.cmd"), "@echo off\r\n").unwrap();
        let (path, needs_shell) = resolve_program("fake-npx", Some(dir.path().as_os_str()), None);
        assert!(needs_shell);
        assert!(path.ends_with("fake-npx.cmd"));

        let (direct, needs_shell) = resolve_program("C:/tools/run.exe", None, None);
        assert!(!needs_shell);
        assert!(direct.ends_with("run.exe"));

        // A PATH directory inside the workspace must not supply the launcher —
        // the planted shim is skipped and resolution falls through.
        let workspace = tempfile::tempdir().unwrap();
        let fake_dir = workspace.path().join("bin");
        fs::create_dir_all(&fake_dir).unwrap();
        fs::write(fake_dir.join("fake-npx.cmd"), "@echo off\r\n").unwrap();
        let path_var = std::ffi::OsString::from(&fake_dir);
        let (fallback, _) = resolve_program("fake-npx", Some(&path_var), Some(workspace.path()));
        assert_eq!(fallback, PathBuf::from("fake-npx"));
    }

    #[tokio::test]
    async fn disabling_persists_intent_and_editing_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        let manager = McpManager::default();
        manager.set_config_path(path);
        let config = |trust: bool| McpServerConfig {
            command: "node".into(),
            args: vec![],
            env: Default::default(),
            disabled: false,
            auto_start: false,
            trust,
        };
        manager.upsert_server("fs", config(false)).unwrap();

        let view = manager.disable("fs").await.unwrap();
        assert!(view.disabled && !view.running);
        assert!(manager.load().unwrap().mcp_servers["fs"].disabled);
        assert!(manager.start("fs").await.is_err());

        // Editing the entry must not re-enable it, but must apply the new trust.
        manager.upsert_server("fs", config(true)).unwrap();
        let file = manager.load().unwrap();
        assert!(file.mcp_servers["fs"].disabled);
        assert!(file.mcp_servers["fs"].trust);

        // start_enabled skips disabled entries without spawning anything.
        manager.start_enabled().await;
        assert!(!manager.list_servers().await.unwrap()[0].running);
    }

    /// A stdio child that never produces a valid response: anything written
    /// to it yields no JSON-RPC reply, so a request stays pending until
    /// timeout or cancel.
    fn silent_server() -> Arc<RunningServer> {
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("cmd");
            command.args(["/C", "findstr", "x"]);
            command
        };
        // `cat` echoes stdin back as non-JSON lines: the reader drops them.
        #[cfg(unix)]
        let mut command = Command::new("cat");
        let mut child = command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn silent child");
        let stdout = child.stdout.take().unwrap();
        let stdin = child.stdin.take().unwrap();
        let pending = Arc::new(Mutex::new(Pending {
            replies: HashMap::new(),
        }));
        tokio::spawn(read_loop(BufReader::new(stdout), pending.clone()));
        Arc::new(RunningServer {
            child: AsyncMutex::new(child),
            stdin: AsyncMutex::new(stdin),
            pending,
            next_id: AtomicU64::new(0),
            tools: vec![crate::domain::McpToolView {
                name: "slow".into(),
                description: String::new(),
                input_schema: json!({}),
                read_only: false,
            }],
            trust: AtomicBool::new(false),
            failed: None,
        })
    }

    #[tokio::test]
    async fn call_tool_honours_the_run_cancel_signal() {
        let manager = Arc::new(McpManager::default());
        let server = silent_server();
        manager.running.lock().await.insert("slow".into(), server.clone());
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let call = tokio::spawn({
            let manager = manager.clone();
            async move {
                manager
                    .call_tool("mcp__slow__slow", json!({}), Some(cancel_rx))
                    .await
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        cancel_tx.send(true).unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), call)
            .await
            .expect("cancelled call must return promptly")
            .unwrap();
        let error = result.unwrap_err();
        assert!(error.contains("已取消"), "{error}");
        assert!(server.pending.lock().unwrap().replies.is_empty());
    }

    #[tokio::test]
    async fn call_tool_returns_immediately_when_already_cancelled() {
        let manager = McpManager::default();
        let server = silent_server();
        manager.running.lock().await.insert("slow".into(), server);
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(true);
        let error = manager
            .call_tool("mcp__slow__slow", json!({}), Some(cancel_rx))
            .await
            .unwrap_err();
        assert!(error.contains("已取消"), "{error}");
    }

    #[tokio::test]
    async fn an_in_flight_call_does_not_block_gate_reads_or_stop() {
        let manager = Arc::new(McpManager::default());
        let server = silent_server();
        manager.running.lock().await.insert("slow".into(), server);
        let worker = tokio::spawn({
            let manager = manager.clone();
            async move { manager.call_tool("mcp__slow__slow", json!({}), None).await }
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        // The map lock must not be held across the request: every read path
        // that gate evaluation and tool listing depend on stays responsive.
        tokio::time::timeout(std::time::Duration::from_secs(2), manager.tool_specs())
            .await
            .expect("tool_specs blocked behind an in-flight call");
        tokio::time::timeout(std::time::Duration::from_secs(2), manager.running_tools())
            .await
            .expect("running_tools blocked behind an in-flight call");
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            manager.tool_is_read_only("mcp__slow__slow"),
        )
        .await
        .expect("tool_is_read_only blocked behind an in-flight call");
        // stop() must also resolve promptly and fail the in-flight caller.
        tokio::time::timeout(std::time::Duration::from_secs(2), manager.stop("slow"))
            .await
            .expect("stop must not wait for the in-flight call")
            .unwrap();
        assert!(tokio::time::timeout(std::time::Duration::from_secs(2), worker)
            .await
            .expect("stopped call must return promptly")
            .unwrap()
            .is_err());
    }
}
