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
    time::Duration,
};

use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::{oneshot, Mutex as AsyncMutex},
};

use crate::domain::{
    AppResult, McpConfigFile, McpServerConfig, McpServerView, McpTransport, RecommendedItem, ToolSpec,
};

use config::{
    load_config, save_config, valid_server_config, valid_server_name,
};
use tools::{call_result_text, parse_tools, split_mcp_tool};
use transport::{
    read_loop, sse_read_loop, Pending, RunningServer, Transport, CALL_TIMEOUT, HANDSHAKE_TIMEOUT, PROTOCOL_VERSION,
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

/// Host-orchestration vars that poison grandchild toolchains. When maestro is
/// launched from `pnpm`/`npm`/an IDE, the inherited env carries vars meant
/// only for the host — `npm_execpath` redirects every npm call, `NODE_OPTIONS`
/// injects flags into every node child, `ELECTRON_RUN_AS_NODE` breaks Electron
/// shims. MCP servers spawn their own toolchain (npx → npm → node), so these
/// must not leak into the child. Legitimate npm_config_* (registry mirrors)
/// and anything the user sets in `env` are kept — envs() runs after this.
fn sanitize_mcp_child_env(builder: &mut Command) {
    const DENY: &[&str] = &[
        "NODE_OPTIONS",
        "ELECTRON_RUN_AS_NODE",
        "ELECTRON_NO_ATTACH_CONSOLE",
        "ELECTRON_FORCE_RUN_AS_NODE",
        "npm_execpath",
        "npm_node_execpath",
        "npm_command",
        "INIT_CWD",
        "PNPM_PACKAGE_NAME",
        "PNPM_HOME",
        "COREPACK_ROOT",
    ];
    for key in DENY {
        builder.env_remove(key);
    }
    let prefixed: Vec<std::ffi::OsString> = std::env::vars_os()
        .map(|(key, _)| key)
        .filter(|key| key.to_string_lossy().starts_with("pnpm_config_"))
        .collect();
    for key in prefixed {
        builder.env_remove(&key);
    }
    #[cfg(windows)]
    {
        // `pnpm tauri dev`/vcvars 会把继承的 PATH 撑到 8K+（大量重复条目），
        // 越过 cmd.exe ~8191 的 PATH 搜索上限后内层 `cmd /c <name>` 按名
        // 解析整体失效（npm exec 跑 `.bin` shim 即如此）→ 子进程秒退。
        // 归一化到安全长度；Command::env 在 Windows 上按大小写不敏感覆盖
        // 继承键，Path/PATH 重复变体一并收敛。用户 config.env 里的 PATH
        // 在 spawn_stdio 中后应用，仍可覆盖此结果。
        const MAX_SAFE_PATH_LEN: usize = 7_000;
        let path_values: Vec<String> = std::env::vars_os()
            .filter(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case("path"))
            .map(|(_, value)| value.to_string_lossy().into_owned())
            .collect();
        let normalized = normalize_path_value(&path_values, MAX_SAFE_PATH_LEN);
        if !normalized.is_empty() {
            builder.env("PATH", normalized);
        }
    }
}

/// 归一化 PATH 值：保序去重（归一化 `\`/`/` 与尾分隔符、大小写不敏感）、
/// 剔除不存在的目录（`%VAR%` 引用保留给子进程展开）、按 max_len 封顶。
#[cfg(windows)]
fn normalize_path_value(values: &[String], max_len: usize) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut kept: Vec<String> = Vec::new();
    let mut total = 0usize;
    'outer: for value in values {
        for entry in value.split(';') {
            if entry.is_empty() {
                continue;
            }
            let key = entry
                .replace('\\', "/")
                .trim_end_matches('/')
                .to_lowercase();
            if !seen.insert(key) {
                continue;
            }
            if !entry.contains('%') && !std::path::Path::new(entry).is_dir() {
                continue;
            }
            total += entry.len() + 1;
            if total > max_len {
                break 'outer;
            }
            kept.push(entry.to_string());
        }
    }
    kept.join(";")
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
                    transport: server.transport,
                    url: server.url.clone(),
                    // 请求头值可能含 PAT 等凭据，只回传键名给 UI。
                    header_keys: server.headers.keys().cloned().collect(),
                    env: server.env.clone(),
                    disabled: server.disabled,
                    auto_start: server.auto_start,
                    trust: server.trust,
                    timeout_ms: server.timeout_ms,
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
                    transport: config.transport,
                    url: config.url.clone(),
                    header_keys: config.headers.keys().cloned().collect(),
                    env: config.env,
                    disabled: false,
                    auto_start: config.auto_start,
                    trust: config.trust,
                    timeout_ms: config.timeout_ms,
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
                    transport: config.transport,
                    url: config.url.clone(),
                    header_keys: config.headers.keys().cloned().collect(),
                    env: config.env,
                    disabled: false,
                    auto_start: config.auto_start,
                    trust: config.trust,
                    timeout_ms: config.timeout_ms,
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

    /// initialize → notifications/initialized → tools/list，三种传输共用。
    async fn spawn(&self, name: &str, config: &McpServerConfig) -> AppResult<RunningServer> {
        match config.transport {
            McpTransport::Stdio => self.spawn_stdio(name, config).await,
            _ => self.spawn_remote(name, config).await,
        }
    }

    async fn spawn_stdio(&self, name: &str, config: &McpServerConfig) -> AppResult<RunningServer> {
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
        sanitize_mcp_child_env(&mut builder);
        // GUI 进程拉起 console 子进程时 Windows 会为其分配一个全新的
        // console——这是与终端手动复现之间仅存的系统性差异（ZCode/Electron
        // 拉起 MCP 子进程同样无窗）。CREATE_NO_WINDOW 消掉这个变量，顺带
        // 避免每个 MCP 服务启动时闪一个控制台黑窗。
        #[cfg(windows)]
        builder.creation_flags(0x08000000); // CREATE_NO_WINDOW
        let mut child = builder
            .args(&config.args)
            .envs(&config.env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
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
        // stderr 收集到 4KB 尾部缓冲：子进程退出时把死因带进错误信息
        // （此前 null() 直接丢弃，"子进程已退出"永远查不到原因）。
        let stderr_tail = Arc::new(Mutex::new(String::new()));
        let stderr_drain = child.stderr.take().map(|stderr| {
            let tail = stderr_tail.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                while let Ok(count) = reader.read_line(&mut line).await {
                    if count == 0 {
                        break;
                    }
                    let mut tail = tail.lock().unwrap();
                    tail.push_str(&line);
                    if tail.len() > 4096 {
                        let keep = tail.len() - 4096;
                        tail.drain(..keep);
                    }
                    line.clear();
                }
            })
        });
        let pending = Arc::new(Mutex::new(Pending {
            replies: HashMap::new(),
        }));
        // stdout 上的非 JSON 行（console.log 崩溃/告警输出）进独立诊断尾。
        let stdout_diag = Arc::new(Mutex::new(String::new()));
        tokio::spawn(read_loop(
            BufReader::new(stdout),
            pending.clone(),
            stdout_diag.clone(),
        ));
        let mut server = RunningServer {
            transport: Transport::Stdio {
                child: AsyncMutex::new(child),
                stdin: AsyncMutex::new(stdin),
            },
            pending,
            next_id: AtomicU64::new(0),
            tools: Vec::new(),
            trust: AtomicBool::new(config.trust),
            failed: None,
        };
        match handshake(&server, handshake_timeout(config)).await {
            Ok(listed) => {
                server.tools = parse_tools(&listed);
                Ok(server)
            }
            Err(error) => {
                server.shutdown().await;
                let code = server.exit_status().await;
                // 等 stderr drain 排干——此前立即读 tail 存在竞态，子进程
                // 临终输出可能还没被推进缓冲就被快照成"空"。
                if let Some(drain) = stderr_drain {
                    let _ = tokio::time::timeout(Duration::from_secs(2), drain).await;
                }
                let tail = stderr_tail.lock().unwrap().trim().to_string();
                let noise = stdout_diag.lock().unwrap().trim().to_string();
                let mut detail = error;
                if let Some(code) = code {
                    detail = format!("{detail}（退出码 {code}）");
                }
                if !tail.is_empty() {
                    detail = format!("{detail}\nstderr: {tail}");
                }
                if !noise.is_empty() {
                    detail = format!("{detail}\nstdout: {noise}");
                }
                Err(detail)
            }
        }
    }

    async fn spawn_remote(&self, _name: &str, config: &McpServerConfig) -> AppResult<RunningServer> {
        let url = config.url.clone().unwrap_or_default();
        let mut headers = reqwest::header::HeaderMap::new();
        for (key, value) in &config.headers {
            let name = reqwest::header::HeaderName::from_bytes(key.as_bytes())
                .map_err(|_| format!("非法请求头名：{key}"))?;
            let value = reqwest::header::HeaderValue::from_str(value)
                .map_err(|_| format!("非法请求头值：{key}"))?;
            headers.insert(name, value);
        }
        let client = reqwest::Client::builder()
            .build()
            .map_err(|error| format!("无法创建 HTTP 客户端：{error}"))?;
        let pending = Arc::new(Mutex::new(Pending {
            replies: HashMap::new(),
        }));
        let transport = match config.transport {
            McpTransport::Http => Transport::Http {
                client,
                url,
                headers,
                session_id: AsyncMutex::new(None),
            },
            McpTransport::Sse => {
                let response = client
                    .get(&url)
                    .header(reqwest::header::ACCEPT, "text/event-stream")
                    .headers(headers.clone())
                    .send()
                    .await
                    .map_err(|error| format!("MCP SSE 连接失败：{error}"))?;
                if let Some(hint) = transport::auth_hint(response.status()) {
                    return Err(hint);
                }
                if !response.status().is_success() {
                    return Err(format!("MCP SSE 响应 {}", response.status()));
                }
                let (endpoint_tx, endpoint_rx) = oneshot::channel();
                let reader = tokio::spawn(sse_read_loop(response, url.clone(), pending.clone(), endpoint_tx));
                let post_url = tokio::time::timeout(HANDSHAKE_TIMEOUT, endpoint_rx)
                    .await
                    .map_err(|_| "等待 MCP SSE endpoint 超时".to_string())?
                    .map_err(|_| "MCP SSE endpoint 通道关闭".to_string())??;
                Transport::Sse {
                    client,
                    post_url,
                    headers,
                    reader: reader.abort_handle(),
                }
            }
            McpTransport::Stdio => unreachable!(),
        };
        let mut server = RunningServer {
            transport,
            pending,
            next_id: AtomicU64::new(0),
            tools: Vec::new(),
            trust: AtomicBool::new(config.trust),
            failed: None,
        };
        match handshake(&server, handshake_timeout(config)).await {
            Ok(listed) => {
                server.tools = parse_tools(&listed);
                Ok(server)
            }
            Err(error) => {
                server.shutdown().await;
                Err(error)
            }
        }
    }

    pub async fn stop(&self, name: &str) -> AppResult<()> {
        if let Some(server) = self.running.lock().await.remove(name) {
            for (_id, tx) in server.pending.lock().unwrap().replies.drain() {
                let _ = tx.send(Err("MCP 服务已停止".into()));
            }
            server.shutdown().await;
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

    /// Startup restore: start every configured server the user has not
    /// explicitly disabled. Failures stay per-server in `errors` and never
    /// block the app.
    pub async fn start_enabled(&self) {
        let names: Vec<String> = match self.load() {
            Ok(file) => file
                .mcp_servers
                .iter()
                .filter(|(_, server)| !server.disabled)
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
            server.shutdown().await;
        }
    }
}

/// Per-server handshake timeout: explicit `timeoutMs` wins; stdio defaults to
/// 60s because a cold `npx -y` install legitimately takes tens of seconds
/// (ZCode exposes the same per-server timeoutMs), remote keeps the 20s bound.
fn handshake_timeout(config: &McpServerConfig) -> Duration {
    config.timeout_ms.map(Duration::from_millis).unwrap_or(match config.transport {
        McpTransport::Stdio => Duration::from_secs(60),
        _ => HANDSHAKE_TIMEOUT,
    })
}

/// initialize → notifications/initialized → tools/list，三种传输共用。
async fn handshake(server: &RunningServer, timeout: Duration) -> AppResult<Value> {
    server
        .request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "maestro", "version": env!("CARGO_PKG_VERSION") },
            }),
            timeout,
            None,
        )
        .await?;
    server
        .notify("notifications/initialized", json!({}))
        .await?;
    server
        .request("tools/list", json!({}), timeout, None)
        .await
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

    #[test]
    fn handshake_timeout_defaults_by_transport_and_honours_override() {
        let mut config = McpServerConfig {
            command: "node".into(),
            args: vec![],
            env: Default::default(),
            transport: McpTransport::Stdio,
            url: None,
            headers: Default::default(),
            disabled: false,
            auto_start: false,
            trust: false,
            timeout_ms: None,
        };
        // stdio 默认 60s（npx 冷装）；remote 默认 20s；显式值覆盖两者。
        assert_eq!(handshake_timeout(&config), Duration::from_secs(60));
        config.transport = McpTransport::Http;
        assert_eq!(handshake_timeout(&config), HANDSHAKE_TIMEOUT);
        config.timeout_ms = Some(5_000);
        assert_eq!(handshake_timeout(&config), Duration::from_secs(5));
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

    #[cfg(windows)]
    #[test]
    fn normalize_path_dedupes_and_drops_missing_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().to_string_lossy().replace('\\', "/");
        let dup = format!("{real}/");
        let missing = dir.path().join("gone").to_string_lossy().replace('\\', "/");
        let values = vec![format!(
            "{real};{dup};{missing};{upper};%JAVA_HOME%\\bin",
            upper = real.to_uppercase()
        )];
        let normalized = normalize_path_value(&values, 10_000);
        let entries: Vec<&str> = normalized.split(';').collect();
        // 同目录的重复/斜杠/大小写变体折叠为一个，失效目录剔除，%VAR% 保留
        assert_eq!(entries, vec![real.as_str(), "%JAVA_HOME%\\bin"]);
    }

    #[cfg(windows)]
    #[test]
    fn normalize_path_merges_case_variants_and_caps_length() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a").to_string_lossy().replace('\\', "/");
        let b = dir.path().join("b").to_string_lossy().replace('\\', "/");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        // Path/PATH 两个变体键的值按序拼接去重
        let normalized = normalize_path_value(&[a.clone(), b.clone()], 10_000);
        assert_eq!(normalized, format!("{a};{b}"));
        // 超长截断：第二条目越界即丢
        let capped = normalize_path_value(&[format!("{a};{b}")], a.len() + 2);
        assert_eq!(capped, a);
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
            transport: McpTransport::Stdio,
            url: None,
            headers: Default::default(),
            disabled: false,
            auto_start: false,
            trust,
            timeout_ms: None,
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

    #[tokio::test]
    async fn start_enabled_starts_all_non_disabled_and_surfaces_failures() {
        let dir = tempfile::tempdir().unwrap();
        let manager = McpManager::default();
        manager.set_config_path(dir.path().join("mcp.json"));
        let config = |disabled: bool| McpServerConfig {
            command: "definitely-not-a-real-mcp-launcher".into(),
            args: vec![],
            env: Default::default(),
            transport: McpTransport::Stdio,
            url: None,
            headers: Default::default(),
            disabled,
            auto_start: false,
            trust: false,
            timeout_ms: None,
        };
        // auto_start=false 的存量配置也必须自启（disabled 是唯一关停位）。
        // upsert 忽略传入的 disabled（编辑不得静默改开关位），disabled 只能经
        // disable() 路径设置——这里走真实 UI 路径。
        manager.upsert_server("on", config(false)).unwrap();
        manager.upsert_server("off", config(false)).unwrap();
        manager.disable("off").await.unwrap();

        manager.start_enabled().await;
        let views = manager.list_servers().await.unwrap();
        let on = views.iter().find(|view| view.name == "on").unwrap();
        let off = views.iter().find(|view| view.name == "off").unwrap();
        assert!(!on.running && on.error.is_some(), "未禁用服务应被尝试拉起且失败可见");
        assert!(!off.running && off.error.is_none(), "disabled 服务不被触碰");
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
        tokio::spawn(read_loop(
            BufReader::new(stdout),
            pending.clone(),
            Arc::new(Mutex::new(String::new())),
        ));
        Arc::new(RunningServer {
            transport: Transport::Stdio {
                child: AsyncMutex::new(child),
                stdin: AsyncMutex::new(stdin),
            },
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
