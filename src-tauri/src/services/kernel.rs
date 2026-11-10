use std::{path::PathBuf, sync::Arc};

use tokio::sync::{watch, Mutex};

use crate::{
    adapters::db::{Database, DEFAULT_PROJECT_ID},
    agent,
    domain::{
        policy::{self, ToolEffect},
        AppResult, EventData, KernelEvent, Mode, PlanCard, PlanDecision, PlanDraft, ProjectView,
        RunMetrics, RunStatus, RunView, SessionDetail, SessionView, Snapshot,
    },
    llm::ChatConfig,
    services::extensions::ExtensionManager,
};

pub const EVENT_NAME: &str = "kernel-event";
pub type EventSink = Arc<dyn Fn(KernelEvent) + Send + Sync>;

struct Control {
    cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
    finishing: bool,
}

struct RuntimeState {
    mode: Mode,
    active_run: Option<RunView>,
    last_run: Option<RunView>,
    control: Option<Control>,
    sequence: u64,
    shutting_down: bool,
    storage_failed: bool,
}

pub struct Kernel {
    database: Database,
    state: Mutex<RuntimeState>,
    // Serializes mutating commands, without blocking an async worker on SQLite.
    commands: Mutex<()>,
    sink: EventSink,
    /// On-disk extension subsystem (`~/.maestro`): skills + MCP servers.
    extensions: ExtensionManager,
    /// Currently selected workspace, set by stream_chat from WorkspaceService.
    workspace: Mutex<Option<PathBuf>>,
}

impl Kernel {
    pub async fn open(path: PathBuf, maestro_dir: PathBuf, sink: EventSink) -> AppResult<Arc<Self>> {
        let database = Database::open(path).await?;
        let last_run = database.list_runs().await?.into_iter().next();
        let kernel = Arc::new(Self {
            database,
            state: Mutex::new(RuntimeState {
                mode: Mode::Plan,
                active_run: None,
                last_run,
                control: None,
                sequence: 0,
                shutting_down: false,
                storage_failed: false,
            }),
            commands: Mutex::new(()),
            sink,
            extensions: ExtensionManager::new(maestro_dir)?,
            workspace: Mutex::new(None),
        });
        kernel.emit_locked(&mut *kernel.state.lock().await, EventData::Ready);
        Ok(kernel)
    }

    pub fn extensions(&self) -> &ExtensionManager {
        &self.extensions
    }

    /// stream_chat supplies the workspace so skills/tools resolve project files.
    pub async fn set_workspace_root(&self, root: Option<PathBuf>) {
        *self.workspace.lock().await = root;
    }

    async fn workspace_root(&self) -> Option<PathBuf> {
        self.workspace.lock().await.clone()
    }

    fn emit_locked(&self, state: &mut RuntimeState, data: EventData) {
        state.sequence += 1;
        (self.sink)(KernelEvent {
            sequence: state.sequence,
            data,
        });
    }

    pub async fn snapshot(&self) -> Snapshot {
        let state = self.state.lock().await;
        Snapshot {
            ready: !state.shutting_down && !state.storage_failed,
            mode: state.mode,
            active_run: state.active_run.clone(),
            last_run: state.last_run.clone(),
            sequence: state.sequence,
        }
    }

    pub async fn set_mode(&self, mode: Mode) -> AppResult<()> {
        let _command = self.commands.lock().await;
        {
            let state = self.state.lock().await;
            if state.shutting_down || state.storage_failed {
                return Err("Application is shutting down".into());
            }
            if state.active_run.is_some() {
                return Err("Cannot change mode during an active run".into());
            }
            if state.mode == mode {
                return Ok(());
            }
        }
        self.database.record_mode(mode).await?;
        let mut state = self.state.lock().await;
        state.mode = mode;
        self.emit_locked(&mut state, EventData::ModeChanged { mode });
        Ok(())
    }

    pub async fn start(self: &Arc<Self>, prompt: String) -> AppResult<i64> {
        if prompt.trim().is_empty() || prompt.trim().len() > 8192 {
            return Err("Prompt must contain 1–8192 UTF-8 bytes".into());
        }
        let session = self.create_session(None).await?;
        self.start_in_session(session.id, prompt).await
    }

    pub async fn start_in_session(
        self: &Arc<Self>,
        session_id: i64,
        prompt: String,
    ) -> AppResult<i64> {
        self.start_run(session_id, prompt, None).await
    }

    pub async fn start_model(
        self: &Arc<Self>,
        session_id: i64,
        prompt: String,
        config: ChatConfig,
    ) -> AppResult<i64> {
        config.validate()?;
        self.start_run(session_id, prompt, Some(config)).await
    }

    async fn start_run(
        self: &Arc<Self>,
        session_id: i64,
        prompt: String,
        config: Option<ChatConfig>,
    ) -> AppResult<i64> {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() || prompt.len() > 8192 {
            return Err("Prompt must contain 1–8192 UTF-8 bytes".into());
        }
        let _command = self.commands.lock().await;
        let mode = {
            let state = self.state.lock().await;
            if state.shutting_down || state.storage_failed {
                return Err("Application is shutting down".into());
            }
            if state.active_run.is_some() {
                return Err("An agent run is already active".into());
            }
            state.mode
        };
        let id = self
            .database
            .create_run_in_session(session_id, prompt.clone(), mode)
            .await?;
        // Workspace fallback: sessions live inside projects — use the project's
        // path so read tools work even before the user opens the Explorer.
        {
            let mut workspace = self.workspace.lock().await;
            if workspace.is_none() {
                if let Ok(Some(path)) = self.database.project_path_for_session(session_id).await {
                    if !path.trim().is_empty() {
                        *workspace = Some(std::path::PathBuf::from(path));
                    }
                }
            }
        }
        let run = RunView {
            id,
            session_id,
            prompt,
            mode,
            status: RunStatus::Running,
            output: String::new(),
            error: None,
        };
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let (done_tx, done_rx) = watch::channel(false);
        {
            let mut state = self.state.lock().await;
            state.active_run = Some(run.clone());
            state.control = Some(Control {
                cancel: cancel_tx,
                done: done_rx,
                finishing: false,
            });
            self.emit_locked(&mut state, EventData::Status { run: run.clone() });
            self.emit_locked(&mut state, EventData::SessionsChanged);
        }
        let kernel = self.clone();
        tauri::async_runtime::spawn(async move {
            match config {
                Some(config) => agent::run_model(kernel, run, config, cancel_rx).await,
                None => agent::run(kernel, run, cancel_rx).await,
            }
            let _ = done_tx.send(true);
        });
        Ok(id)
    }

    pub async fn cancel(&self, run_id: i64) -> AppResult<()> {
        let _command = self.commands.lock().await;
        let state = self.state.lock().await;
        if state.active_run.as_ref().map(|run| run.id) != Some(run_id) {
            return Err("Requested run is not active".into());
        }
        let control = state.control.as_ref().ok_or("Run control is missing")?;
        if control.finishing {
            return Err("Run is already finishing".into());
        }
        control
            .cancel
            .send(true)
            .map_err(|_| "Run has already stopped".to_string())
    }

    /// Authorization is tied to the active run, never to a frontend-supplied mode.
    pub async fn authorize_tool(&self, run_id: i64, tool: &str) -> AppResult<ToolEffect> {
        let state = self.state.lock().await;
        let run = Self::active_run_locked(&state, run_id)?;
        policy::authorize(run.mode, tool)
    }

    fn active_run_locked(state: &RuntimeState, run_id: i64) -> AppResult<RunView> {
        let run = state
            .active_run
            .as_ref()
            .filter(|run| run.id == run_id)
            .ok_or("No matching active run")?;
        if state.shutting_down
            || state.storage_failed
            || state
                .control
                .as_ref()
                .is_none_or(|control| control.finishing || *control.cancel.borrow())
        {
            return Err("Run was cancelled or application is shutting down".into());
        }
        Ok(run.clone())
    }

    /// The only registered tool dispatcher. No frontend-selected effect or arbitrary SQL/path.
    pub(crate) async fn dispatch_tool(
        &self,
        run_id: i64,
        tool: &str,
    ) -> AppResult<serde_json::Value> {
        let _command = self.commands.lock().await;
        let run = Self::active_run_locked(&*self.state.lock().await, run_id)?;
        let authorization = policy::authorize(run.mode, tool).and_then(|effect| {
            if effect != ToolEffect::ReadOnly {
                return Err(
                    "Side-effect execution is not enabled in Phase 1; approval records intent only"
                        .into(),
                );
            }
            Ok(effect)
        });
        if let Err(reason) = authorization {
            self.database
                .record_tool(run_id, tool.into(), reason.clone(), false)
                .await?;
            let mut state = self.state.lock().await;
            if Self::active_run_locked(&state, run_id).is_ok() {
                self.emit_locked(
                    &mut state,
                    EventData::ToolBlocked {
                        run_id,
                        tool: tool.into(),
                        reason: reason.clone(),
                    },
                );
            }
            return Err(reason);
        }
        let result = match tool {
            "read_session" => {
                serde_json::to_value(self.database.session_detail(run.session_id).await?)
            }
            "list_sessions" => serde_json::to_value(self.database.list_sessions().await?),
            _ => return Err("No registered tool executor".into()),
        }
        .map_err(|error| error.to_string())?;
        self.database
            .record_tool(run_id, tool.into(), "read-only".into(), true)
            .await?;
        Ok(result)
    }

    /// Resolve a workspace-relative path: must canonicalize under the selected root
    /// (rejects `..`, symlink junction escapes, and absolute paths).
    fn scoped_path(root: &std::path::Path, rel: &str) -> AppResult<std::path::PathBuf> {
        let rel_path = std::path::Path::new(rel);
        if rel_path.is_absolute() || rel_path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err("路径必须在工作区内".into());
        }
        let base = root.canonicalize().map_err(|_| "工作区不可用".to_string())?;
        let full = if rel.is_empty() { base.clone() } else { base.join(rel_path).canonicalize().map_err(|_| "路径不存在".to_string())? };
        if !full.starts_with(&base) { return Err("路径必须在工作区内".into()); }
        Ok(full)
    }

    /// Tool definitions offered to the model this run: workspace reads, load_skill + running MCP tools.
    pub async fn agent_tools(&self) -> Vec<serde_json::Value> {
        let mut tools = self.extensions.tool_definitions().await;
        let workspace_root = self.workspace.lock().await.clone();
        if let Some(desc) = tools.iter_mut().find(|t| t.get("name").and_then(|n| n.as_str()) == Some("load_skill")).and_then(|t| t.get_mut("description")) {
            let names = self.extensions.skills.scan(self.extensions.maestro_dir(), workspace_root.as_deref())
                .into_iter().map(|s| s.name).collect::<Vec<_>>();
            if !names.is_empty() {
                *desc = serde_json::json!(format!("{} 当前可用技能：{}", desc.as_str().unwrap_or_default(), names.join("、")));
            }
        }
        // Read tools are always offered — execution reports "未选择工作区" if unset.
        {
            tools.push(serde_json::json!({
                "name": "list_dir",
                "description": "列出工作区目录内容（只读）。path 为相对路径，省略表示根目录。",
                "input_schema": { "type": "object", "properties": { "path": { "type": "string" } } },
            }));
            tools.push(serde_json::json!({
                "name": "read_file",
                "description": "读取工作区文件内容（只读，限文本）。path 为相对路径。",
                "input_schema": { "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] },
            }));
        }
        tools
    }

    /// Execute one model-requested tool call: authorize, route, audit, return
    /// bounded text for the next conversation turn.
    pub(crate) async fn execute_model_tool(
        &self,
        run_id: i64,
        tool: &str,
        arguments_json: &str,
    ) -> AppResult<String> {
        {
            let _command = self.commands.lock().await;
            let run = Self::active_run_locked(&*self.state.lock().await, run_id)?;
            let effect = if tool.starts_with("mcp__") {
                if self.extensions.mcp.tool_is_read_only(tool).await {
                    ToolEffect::ReadOnly
                } else {
                    ToolEffect::ProcessExecution
                }
            } else {
                policy::authorize(run.mode, tool)?
            };
            if tool.starts_with("mcp__") {
                policy::check_mode(run.mode, effect, tool)?;
            }
        }
        let arguments: serde_json::Value = serde_json::from_str(arguments_json)
            .unwrap_or_else(|_| serde_json::json!({}));
        let result = match tool {
            "load_skill" => {
                let name = arguments
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "load_skill 需要 name 参数".to_string())?;
                self.extensions
                    .skills
                    .body(
                        name,
                        self.extensions.maestro_dir(),
                        self.workspace_root().await.as_deref(),
                    )
                    .map(|body| format!("【技能 {name}】\n\n{body}"))
            }
            "list_dir" => {
                let root = self.workspace.lock().await.clone()
                    .ok_or_else(|| "未选择工作区：当前项目未设置目录，请在输入框下方项目菜单中点击「设置目录」".to_string())?;
                let rel = arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let dir = Self::scoped_path(&root, rel)?;
                let mut entries = Vec::new();
                let mut read = std::fs::read_dir(&dir).map_err(|_| "目录不可读".to_string())?;
                while let Some(Ok(entry)) = read.next() {
                    entries.push(serde_json::json!({
                        "name": entry.file_name().to_string_lossy(),
                        "dir": entry.file_type().map(|t| t.is_dir()).unwrap_or(false),
                    }));
                    if entries.len() >= 200 { break; }
                }
                Ok(serde_json::to_string_pretty(&entries).unwrap_or_default())
            }
            "read_file" => {
                let root = self.workspace.lock().await.clone()
                    .ok_or_else(|| "未选择工作区：当前项目未设置目录，请在输入框下方项目菜单中点击「设置目录」".to_string())?;
                let rel = arguments.get("path").and_then(|v| v.as_str())
                    .ok_or_else(|| "read_file 需要 path 参数".to_string())?;
                let file = Self::scoped_path(&root, rel)?;
                let meta = std::fs::metadata(&file).map_err(|_| "文件不可读".to_string())?;
                if meta.len() > 262144 { return Err("文件超过 256KB 限制".into()); }
                let bytes = std::fs::read(&file).map_err(|_| "文件不可读".to_string())?;
                let text = String::from_utf8(bytes).map_err(|_| "非 UTF-8 文本文件".to_string())?;
                Ok(text.chars().take(65536).collect())
            }
            _ if tool.starts_with("mcp__") => {
                self.extensions.mcp.call_tool(tool, arguments).await
            }
            _ => Err("未注册的工具".into()),
        };
        match &result {
            Ok(_) => {
                self.database
                    .record_tool(run_id, tool.into(), "extension".into(), true)
                    .await?;
                let mut state = self.state.lock().await;
                if Self::active_run_locked(&state, run_id).is_ok() {
                    self.emit_locked(
                        &mut state,
                        EventData::ToolCalled { run_id, tool: tool.into(), detail: String::new(), allowed: true },
                    );
                }
            }
            Err(reason) => {
                let _ = self
                    .database
                    .record_tool(run_id, tool.into(), reason.clone(), false)
                    .await;
                let mut state = self.state.lock().await;
                if Self::active_run_locked(&state, run_id).is_ok() {
                    self.emit_locked(
                        &mut state,
                        EventData::ToolBlocked {
                            run_id,
                            tool: tool.into(),
                            reason: reason.clone(),
                        },
                    );
                }
            }
        }
        result
    }

    pub async fn create_session(&self, project_id: Option<i64>) -> AppResult<SessionView> {
        let _command = self.commands.lock().await;
        let unavailable = {
            let state = self.state.lock().await;
            state.shutting_down || state.storage_failed
        };
        if unavailable {
            return Err("Application is shutting down or storage is unavailable".into());
        }
        let session = self
            .database
            .create_session_in_project(project_id.unwrap_or(DEFAULT_PROJECT_ID))
            .await?;
        self.emit_locked(&mut *self.state.lock().await, EventData::SessionsChanged);
        Ok(session)
    }

    pub async fn set_session_archived(
        &self,
        session_id: i64,
        archived: bool,
    ) -> AppResult<SessionView> {
        let _command = self.commands.lock().await;
        {
            let state = self.state.lock().await;
            if state.shutting_down || state.storage_failed {
                return Err("Application is shutting down or storage is unavailable".into());
            }
            if archived
                && state
                    .active_run
                    .as_ref()
                    .is_some_and(|run| run.session_id == session_id)
            {
                return Err("Cannot archive a session with an active run".into());
            }
        }
        let session = self
            .database
            .set_session_archived(session_id, archived)
            .await?;
        self.emit_locked(&mut *self.state.lock().await, EventData::SessionsChanged);
        Ok(session)
    }

    pub async fn rename_session(&self, session_id: i64, title: String) -> AppResult<SessionView> {
        let _command = self.commands.lock().await;
        let session = self.database.rename_session(session_id, title).await?;
        self.emit_locked(&mut *self.state.lock().await, EventData::SessionsChanged);
        Ok(session)
    }

    pub async fn list_projects(&self) -> AppResult<Vec<ProjectView>> {
        self.database.list_projects().await
    }

    pub async fn create_project(
        &self,
        name: String,
        path: Option<String>,
    ) -> AppResult<ProjectView> {
        let _command = self.commands.lock().await;
        {
            let state = self.state.lock().await;
            if state.shutting_down || state.storage_failed {
                return Err("Application is shutting down or storage is unavailable".into());
            }
        }
        let project = self.database.create_project(name, path).await?;
        self.emit_locked(&mut *self.state.lock().await, EventData::SessionsChanged);
        Ok(project)
    }

    pub async fn set_project_archived(
        &self,
        project_id: i64,
        archived: bool,
    ) -> AppResult<ProjectView> {
        let _command = self.commands.lock().await;
        let project = self
            .database
            .set_project_archived(project_id, archived)
            .await?;
        self.emit_locked(&mut *self.state.lock().await, EventData::SessionsChanged);
        Ok(project)
    }

    pub async fn delete_session(&self, session_id: i64) -> AppResult<()> {
        let _command = self.commands.lock().await;
        {
            let state = self.state.lock().await;
            if state.shutting_down || state.storage_failed {
                return Err("Application is shutting down or storage is unavailable".into());
            }
        }
        self.database.delete_session(session_id).await?;
        self.emit_locked(&mut *self.state.lock().await, EventData::SessionsChanged);
        Ok(())
    }

    pub async fn update_project_name(&self, project_id: i64, name: String) -> AppResult<()> {
        let _command = self.commands.lock().await;
        self.database.update_project_name(project_id, name).await?;
        self.emit_locked(&mut *self.state.lock().await, EventData::SessionsChanged);
        Ok(())
    }

    pub async fn update_project_path(&self, project_id: i64, path: String) -> AppResult<()> {
        let _command = self.commands.lock().await;
        self.database.update_project_path(project_id, path).await?;
        self.emit_locked(&mut *self.state.lock().await, EventData::SessionsChanged);
        Ok(())
    }

    pub async fn delete_project(&self, project_id: i64) -> AppResult<()> {
        let _command = self.commands.lock().await;
        {
            let state = self.state.lock().await;
            if state.shutting_down || state.storage_failed {
                return Err("Application is shutting down or storage is unavailable".into());
            }
        }
        self.database.delete_project(project_id).await?;
        self.emit_locked(&mut *self.state.lock().await, EventData::SessionsChanged);
        Ok(())
    }

    pub async fn list_sessions(&self) -> AppResult<Vec<SessionView>> {
        self.database.list_sessions().await
    }

    pub async fn session_detail(&self, session_id: i64) -> AppResult<SessionDetail> {
        self.database.session_detail(session_id).await
    }

    pub(crate) async fn publish_plan(&self, run_id: i64, draft: PlanDraft) -> AppResult<PlanCard> {
        let _command = self.commands.lock().await;
        let run = Self::active_run_locked(&*self.state.lock().await, run_id)?;
        let plan = self.database.create_plan(run, draft).await?;
        self.emit_locked(
            &mut *self.state.lock().await,
            EventData::PlanUpdated { plan: plan.clone() },
        );
        Ok(plan)
    }

    pub async fn decide_plan(
        &self,
        session_id: i64,
        plan_id: i64,
        decision: PlanDecision,
    ) -> AppResult<PlanCard> {
        let _command = self.commands.lock().await;
        let unavailable = {
            let state = self.state.lock().await;
            state.shutting_down || state.storage_failed
        };
        if unavailable {
            return Err("Application is shutting down or storage is unavailable".into());
        }
        let plan = self
            .database
            .decide_plan(session_id, plan_id, decision)
            .await?;
        self.emit_locked(
            &mut *self.state.lock().await,
            EventData::PlanUpdated { plan: plan.clone() },
        );
        Ok(plan)
    }

    pub(crate) async fn reasoning_token(&self, run_id: i64, token: String) {
        let mut state = self.state.lock().await;
        if state.shutting_down
            || state
                .control
                .as_ref()
                .is_none_or(|control| control.finishing || *control.cancel.borrow())
        {
            return;
        }
        if Self::active_run_locked(&state, run_id).is_err() {
            return;
        }
        self.emit_locked(
            &mut state,
            EventData::ReasoningToken { run_id, token },
        );
    }

    pub(crate) async fn token(&self, run_id: i64, token: String) {
        let mut state = self.state.lock().await;
        if state.shutting_down
            || state
                .control
                .as_ref()
                .is_none_or(|control| control.finishing || *control.cancel.borrow())
        {
            return;
        }
        if let Some(run) = state.active_run.as_mut().filter(|run| run.id == run_id) {
            run.output.push_str(&token);
            let session_id = run.session_id;
            self.emit_locked(
                &mut state,
                EventData::StreamToken {
                    run_id,
                    session_id,
                    token,
                },
            );
        }
    }

    /// Streaming metrics updates reach the UI as events; only the terminal
    /// report is durable in SQLite.
    pub(crate) async fn report_metrics(&self, metrics: RunMetrics, persist: bool) -> AppResult<()> {
        if persist {
            self.database.save_metrics(metrics.clone()).await?;
        }
        self.emit_locked(
            &mut *self.state.lock().await,
            EventData::Metrics { metrics },
        );
        Ok(())
    }

    pub(crate) async fn finish(&self, run_id: i64, status: RunStatus, error: Option<String>) {
        let mut run = {
            let mut state = self.state.lock().await;
            let Some(run) = state.active_run.as_ref().filter(|run| run.id == run_id) else {
                return;
            };
            let mut run = run.clone();
            let Some(control) = state.control.as_mut() else {
                return;
            };
            if control.finishing {
                return;
            }
            // Choose the terminal result atomically with the cancellation gate.
            // Later cancellation is rejected, even while persistence is in flight.
            control.finishing = true;
            run.status = if status == RunStatus::Completed && *control.cancel.borrow() {
                RunStatus::Cancelled
            } else {
                status
            };
            run.error = error;
            run
        };
        let mut persistence_error = None;
        for attempt in 0..3 {
            match self.database.finish_run(run.clone()).await {
                Ok(()) => {
                    persistence_error = None;
                    break;
                }
                Err(error) => {
                    persistence_error = Some(error);
                    if attempt < 2 {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
            }
        }
        if let Some(error) = persistence_error {
            let message =
                format!("Cannot persist run result; new work is disabled until restart: {error}");
            run.status = RunStatus::Failed;
            run.error = Some(message.clone());
            let mut state = self.state.lock().await;
            state.storage_failed = true;
            state.active_run = Some(run);
            // Keep the finishing control/gate: durable run and plan state are not reconciled.
            self.emit_locked(&mut state, EventData::BackendError { message });
            return;
        }
        let mut state = self.state.lock().await;
        state.active_run = None;
        state.last_run = Some(run.clone());
        state.control = None;
        self.emit_locked(&mut state, EventData::Status { run });
        self.emit_locked(&mut state, EventData::SessionsChanged);
    }

    pub async fn list_runs(&self) -> AppResult<Vec<RunView>> {
        self.database.list_runs().await
    }

    pub async fn shutdown(&self) -> AppResult<()> {
        let mut done = {
            let _command = self.commands.lock().await;
            let mut state = self.state.lock().await;
            state.shutting_down = true;
            state.control.as_ref().map(|control| {
                let _ = control.cancel.send(true);
                control.done.clone()
            })
        };
        if let Some(done) = done.as_mut() {
            while !*done.borrow_and_update() {
                done.changed()
                    .await
                    .map_err(|_| "Agent stopped without completing shutdown".to_string())?;
            }
        }
        self.extensions.mcp.shutdown().await;
        self.database.shutdown().await?;
        if self.state.lock().await.storage_failed {
            return Err("Run persistence failed; recovery is required on next startup".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    async fn test_kernel() -> (Arc<Kernel>, Arc<StdMutex<Vec<KernelEvent>>>) {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let captured = events.clone();
        let maestro_dir = tempfile::tempdir().unwrap().keep();
        let kernel = Kernel::open(
            ":memory:".into(),
            maestro_dir,
            Arc::new(move |event| captured.lock().unwrap().push(event)),
        )
        .await
        .unwrap();
        (kernel, events)
    }

    async fn wait_for_idle(kernel: &Kernel) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while kernel.snapshot().await.active_run.is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn snapshot_recovers_ready_even_if_initial_event_was_missed() {
        let (kernel, events) = test_kernel().await;
        let snapshot = kernel.snapshot().await;
        assert!(snapshot.ready);
        assert_eq!(snapshot.mode, Mode::Plan);
        assert_eq!(snapshot.sequence, 1);
        assert!(matches!(events.lock().unwrap()[0].data, EventData::Ready));
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn run_guard_rejects_parallel_runs_mode_changes_and_wrong_ids() {
        let (kernel, _) = test_kernel().await;
        let id = kernel.start("test plan".into()).await.unwrap();
        assert!(kernel.start("parallel".into()).await.is_err());
        assert!(kernel.set_mode(Mode::Execute).await.is_err());
        assert!(kernel.cancel(id + 1).await.is_err());
        assert!(kernel.authorize_tool(id, "apply_patch").await.is_err());
        assert!(kernel.authorize_tool(id + 1, "read_session").await.is_err());
        kernel.cancel(id).await.unwrap();
        wait_for_idle(&kernel).await;
        assert_eq!(
            kernel.list_runs().await.unwrap()[0].status,
            RunStatus::Cancelled
        );
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn demo_stream_is_ordered_and_persisted() {
        let (kernel, events) = test_kernel().await;
        kernel.start("hello".into()).await.unwrap();
        wait_for_idle(&kernel).await;
        let runs = kernel.list_runs().await.unwrap();
        assert_eq!(runs[0].status, RunStatus::Completed);
        assert!(runs[0].output.contains("占位模型"));
        assert_eq!(
            kernel
                .session_detail(runs[0].session_id)
                .await
                .unwrap()
                .plans
                .len(),
            1
        );
        assert_eq!(
            kernel.snapshot().await.last_run.unwrap().output,
            runs[0].output
        );
        {
            let events = events.lock().unwrap();
            assert!(events
                .windows(2)
                .all(|pair| pair[1].sequence == pair[0].sequence + 1));
            assert!(events
                .iter()
                .any(|event| matches!(event.data, EventData::StreamToken { .. })));
        }
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_cancels_and_drains_and_rejects_new_work() {
        let (kernel, _) = test_kernel().await;
        kernel.start("shutdown".into()).await.unwrap();
        kernel.shutdown().await.unwrap();
        assert!(!kernel.snapshot().await.ready);
        assert_eq!(
            kernel.snapshot().await.last_run.unwrap().status,
            RunStatus::Cancelled
        );
        assert!(kernel.start("late".into()).await.is_err());
        assert!(kernel.set_mode(Mode::Execute).await.is_err());
    }

    #[tokio::test]
    async fn validates_prompts_on_the_backend() {
        let (kernel, _) = test_kernel().await;
        assert!(kernel.start("  ".into()).await.is_err());
        assert!(kernel.start("x".repeat(8193)).await.is_err());
        kernel.shutdown().await.unwrap();
    }

    // Seed a run without the demo task to exercise the terminal boundary deterministically.
    async fn seed_run(kernel: &Kernel) -> (i64, watch::Receiver<bool>) {
        let id = kernel
            .database
            .create_run("boundary".into(), Mode::Plan)
            .await
            .unwrap();
        let (cancel, receiver) = watch::channel(false);
        let (_, done) = watch::channel(false);
        let session_id = kernel.database.list_runs().await.unwrap()[0].session_id;
        let mut state = kernel.state.lock().await;
        state.active_run = Some(RunView {
            id,
            session_id,
            prompt: "boundary".into(),
            mode: Mode::Plan,
            status: RunStatus::Running,
            output: String::new(),
            error: None,
        });
        state.control = Some(Control {
            cancel,
            done,
            finishing: false,
        });
        (id, receiver)
    }

    #[tokio::test]
    async fn accepted_cancel_suppresses_tokens_and_overrides_completion() {
        let (kernel, _) = test_kernel().await;
        let (id, _receiver) = seed_run(&kernel).await;
        kernel.cancel(id).await.unwrap();
        kernel.token(id, "too late".into()).await;
        assert!(kernel
            .snapshot()
            .await
            .active_run
            .unwrap()
            .output
            .is_empty());
        kernel.finish(id, RunStatus::Completed, None).await;
        assert_eq!(
            kernel.list_runs().await.unwrap()[0].status,
            RunStatus::Cancelled
        );
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn finishing_gate_rejects_cancellation_tools_and_more_tokens() {
        let (kernel, _) = test_kernel().await;
        let (id, _receiver) = seed_run(&kernel).await;
        kernel
            .state
            .lock()
            .await
            .control
            .as_mut()
            .unwrap()
            .finishing = true;
        assert!(kernel.cancel(id).await.is_err());
        assert!(kernel.authorize_tool(id, "read_session").await.is_err());
        kernel.token(id, "too late".into()).await;
        assert!(kernel
            .snapshot()
            .await
            .active_run
            .unwrap()
            .output
            .is_empty());
        kernel
            .state
            .lock()
            .await
            .control
            .as_mut()
            .unwrap()
            .finishing = false;
        kernel.finish(id, RunStatus::Completed, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn real_dispatch_blocks_side_effects_and_approval_does_not_escalate_mode() {
        let (kernel, _) = test_kernel().await;
        let (id, _receiver) = seed_run(&kernel).await;
        assert!(kernel.dispatch_tool(id, "read_session").await.is_ok());
        for tool in [
            "write_file",
            "apply_patch",
            "run_command",
            "read_only_write_file",
        ] {
            assert!(kernel.dispatch_tool(id, tool).await.is_err());
        }
        let draft = crate::agent::placeholder::proposal("test");
        let card = kernel.publish_plan(id, draft).await.unwrap();
        kernel.finish(id, RunStatus::Completed, None).await;
        kernel
            .decide_plan(card.session_id, card.id, PlanDecision::Approve)
            .await
            .unwrap();
        assert_eq!(kernel.snapshot().await.mode, Mode::Plan);
        assert!(kernel.authorize_tool(id, "apply_patch").await.is_err());
        assert!(kernel
            .decide_plan(card.session_id, card.id, PlanDecision::Reject)
            .await
            .is_err());
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_plans_cannot_be_approved() {
        let (kernel, _) = test_kernel().await;
        let (id, _receiver) = seed_run(&kernel).await;
        let card = kernel
            .publish_plan(id, crate::agent::placeholder::proposal("cancelled"))
            .await
            .unwrap();
        kernel.cancel(id).await.unwrap();
        assert!(kernel
            .publish_plan(id, crate::agent::placeholder::proposal("late"))
            .await
            .is_err());
        kernel.finish(id, RunStatus::Cancelled, None).await;
        assert_eq!(
            kernel.session_detail(card.session_id).await.unwrap().plans[0].status,
            crate::domain::PlanStatus::Invalidated
        );
        assert!(kernel
            .decide_plan(card.session_id, card.id, PlanDecision::Approve)
            .await
            .is_err());
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn persistent_storage_failure_keeps_the_run_gate_and_disables_new_work() {
        let (kernel, events) = test_kernel().await;
        let (id, _receiver) = seed_run(&kernel).await;
        kernel.database.shutdown().await.unwrap();
        kernel.finish(id, RunStatus::Completed, None).await;
        let snapshot = kernel.snapshot().await;
        assert!(!snapshot.ready);
        assert_eq!(snapshot.active_run.unwrap().status, RunStatus::Failed);
        assert!(kernel.create_session(None).await.is_err());
        assert!(kernel.start_in_session(1, "late".into()).await.is_err());
        assert!(kernel.authorize_tool(id, "read_session").await.is_err());
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event.data, EventData::BackendError { .. })));
    }
}
