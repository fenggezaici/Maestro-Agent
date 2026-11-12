use std::{path::PathBuf, sync::Arc};

use tokio::sync::{oneshot, watch, Mutex};

use std::sync::Mutex as StdMutex;

use crate::{
    adapters::{
        db::{Database, DEFAULT_PROJECT_ID},
        llm::{provider_for, ChatConfig, LlmProvider, ProviderResolver},
    },
    domain::{
        policy::{self, ToolEffect, ToolGate},
        AppResult, ChatMessage, EventData, KernelEvent, Mode, PlanCard, PlanDecision, PlanDraft,
        ProjectView, RunMetrics, RunStatus, RunView, SessionDetail, SessionView, Snapshot,
        ToolPhase, ToolSpec,
    },
    services::{agent_loop, extensions::ExtensionManager, tool_router::ToolRouter},
};

pub const EVENT_NAME: &str = "kernel-event";
pub type EventSink = Arc<dyn Fn(KernelEvent) + Send + Sync>;

struct Control {
    cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
    finishing: bool,
    /// A tool call suspended awaiting the user's approval decision. At most
    /// one can exist (the agent loop is serial); dropping the responder
    /// without answering counts as rejection.
    pending: Option<PendingApproval>,
}

struct PendingApproval {
    responder: oneshot::Sender<bool>,
}

struct RuntimeState {
    mode: Mode,
    active_run: Option<RunView>,
    last_run: Option<RunView>,
    control: Option<Control>,
    /// Independent cancellation flag: the watch notification is a hint, this is
    /// the source of truth the agent loop polls.
    cancelled_run: Option<i64>,
    sequence: u64,
    shutting_down: bool,
    storage_failed: bool,
}

pub struct Kernel {
    database: Arc<Database>,
    /// Tool registry/dispatcher: extension stores + workspace root.
    router: ToolRouter,
    state: Mutex<RuntimeState>,
    // Serializes mutating commands, without blocking an async worker on SQLite.
    commands: Mutex<()>,
    sink: EventSink,
    /// Resolves a `Provider` to its adapter; tests substitute a stub.
    llm_resolver: StdMutex<ProviderResolver>,
}

impl Kernel {
    pub async fn open(path: PathBuf, maestro_dir: PathBuf, sink: EventSink) -> AppResult<Arc<Self>> {
        let database = Arc::new(Database::open(path).await?);
        let last_run = database.list_runs().await?.into_iter().next();
        let kernel = Arc::new(Self {
            router: ToolRouter::new(maestro_dir, database.clone())?,
            database,
            state: Mutex::new(RuntimeState {
                mode: Mode::Plan,
                active_run: None,
                last_run,
                control: None,
                cancelled_run: None,
                sequence: 0,
                shutting_down: false,
                storage_failed: false,
            }),
            commands: Mutex::new(()),
            sink,
            llm_resolver: StdMutex::new(Arc::new(|provider| provider_for(provider))),
        });
        kernel.emit_locked(&mut *kernel.state.lock().await, EventData::Ready);
        Ok(kernel)
    }

    pub fn extensions(&self) -> &ExtensionManager {
        self.router.extensions()
    }

    /// Resolve the LLM adapter for this run's provider.
    pub(crate) fn llm_provider(&self, config: &ChatConfig) -> Arc<dyn LlmProvider> {
        (self.llm_resolver.lock().unwrap())(config.provider)
    }

    /// stream_chat supplies the workspace so skills/tools resolve project files.
    pub async fn set_workspace_root(&self, root: Option<PathBuf>) {
        self.router.set_workspace_root(root).await;
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
        // `execute` exists only so historical run rows deserialize; it must
        // never be selectable again — it bypasses every approval gate.
        if mode == Mode::Execute {
            return Err("execute 模式已废弃，仅用于历史记录，不可选择".into());
        }
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

    /// `stream_chat` entry: validate the message list, resolve the session,
    /// then start the run. Validation lives here because it precedes the run.
    pub async fn start_chat(
        self: &Arc<Self>,
        session_id: Option<i64>,
        messages: Vec<ChatMessage>,
        config: ChatConfig,
    ) -> AppResult<i64> {
        config.validate()?;
        let prompt = crate::domain::validate_messages(&messages)?;
        let id = match session_id {
            Some(id) => id,
            None => self.create_session(None).await?.id,
        };
        self.start_model(id, prompt, config).await
    }

    pub async fn start_model(
        self: &Arc<Self>,
        session_id: i64,
        prompt: String,
        config: ChatConfig,
    ) -> AppResult<i64> {
        config.validate()?;
        self.start_run(session_id, prompt, config).await
    }

    async fn start_run(
        self: &Arc<Self>,
        session_id: i64,
        prompt: String,
        mut config: ChatConfig,
    ) -> AppResult<i64> {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() || prompt.len() > 8192 {
            return Err("Prompt must contain 1–8192 UTF-8 bytes".into());
        }
        // OpenCode gateways route/cache per conversation: keep one stable id
        // for every turn of this session.
        config.session = Some(format!("maestro-{session_id}"));
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
            if self.router.workspace_root().await.is_none() {
                if let Ok(Some(path)) = self.database.project_path_for_session(session_id).await {
                    if !path.trim().is_empty() {
                        self.router
                            .set_workspace_root(Some(std::path::PathBuf::from(path)))
                            .await;
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
            created_at: crate::domain::unix_now(),
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
                pending: None,
            });
            state.cancelled_run = None;
            self.emit_locked(&mut state, EventData::Status { run: run.clone() });
            self.emit_locked(&mut state, EventData::SessionsChanged);
        }
        let kernel = self.clone();
        tauri::async_runtime::spawn(async move {
            agent_loop::run(kernel, run, config, cancel_rx).await;
            let _ = done_tx.send(true);
        });
        Ok(id)
    }

    pub async fn cancel(&self, run_id: i64) -> AppResult<()> {
        // Cancellation must never queue behind the command gate: a long
        // mutating command would otherwise make "stop" feel dead.
        let mut state = self.state.lock().await;
        let active = state.active_run.as_ref().map(|run| run.id);
        // Idempotent best-effort: the control channel is a singleton for the
        // single in-flight run, so an id mismatch or an already-finished run
        // is a harmless no-op — never an error the UI should surface.
        let Some(control) = state.control.as_mut() else { return Ok(()) };
        if control.finishing {
            return Ok(());
        }
        // send_replace also works when no receiver is currently polling.
        control.cancel.send_replace(true);
        // Unblock any call suspended on an approval so the loop can observe
        // the cancellation and finish instead of hanging on the user's answer.
        if let Some(pending) = control.pending.take() {
            let _ = pending.responder.send(false);
        }
        // Flag after the control borrow ends: the loop polls this even if the
        // watch notification is lost.
        state.cancelled_run = active;
        let _ = run_id;
        Ok(())
    }

    /// True once cancellation was requested for this run. The agent loop polls
    /// this between turns/tools so a missed watch notification cannot hang it.
    pub(crate) async fn is_cancelled(&self, run_id: i64) -> bool {
        let state = self.state.lock().await;
        state.cancelled_run == Some(run_id)
            || state
                .control
                .as_ref()
                .is_none_or(|control| control.finishing || *control.cancel.borrow())
    }

    /// Authorization is tied to the active run, never to a frontend-supplied mode.
    /// Goes through the router so `mcp__*` tools resolve their readOnlyHint.
    pub async fn authorize_tool(&self, run_id: i64, tool: &str) -> AppResult<ToolEffect> {
        let run = {
            let state = self.state.lock().await;
            Self::active_run_locked(&state, run_id)?
        };
        let effect = self.router.classify(tool).await?;
        match policy::gate(run.mode, effect) {
            ToolGate::Deny => Err(format!(
                "Plan Mode blocked tool: {tool}; only read tools are allowed"
            )),
            _ => Ok(effect),
        }
    }

    fn active_run_locked(state: &RuntimeState, run_id: i64) -> AppResult<RunView> {
        let run = state
            .active_run
            .as_ref()
            .filter(|run| run.id == run_id)
            .ok_or("No matching active run")?;
        if state.shutting_down
            || state.storage_failed
            || state.cancelled_run == Some(run_id)
            || state
                .control
                .as_ref()
                .is_none_or(|control| control.finishing || *control.cancel.borrow())
        {
            return Err("Run was cancelled or application is shutting down".into());
        }
        Ok(run.clone())
    }

    /// Emit one `tool_call` event while the run is still active.
    async fn emit_tool(&self, run_id: i64, tool: &str, phase: ToolPhase, detail: &str) {
        let mut state = self.state.lock().await;
        if Self::active_run_locked(&state, run_id).is_ok() {
            self.emit_locked(
                &mut state,
                EventData::ToolCall {
                    run_id,
                    tool: tool.into(),
                    phase,
                    detail: detail.into(),
                },
            );
        }
    }

    /// The only registered tool dispatcher. No frontend-selected effect or arbitrary SQL/path.
    /// Exercised by the kernel test suite; the agent loop uses execute_model_tool.
    #[allow(dead_code)]
    pub(crate) async fn dispatch_tool(
        &self,
        run_id: i64,
        tool: &str,
    ) -> AppResult<serde_json::Value> {
        let _command = self.commands.lock().await;
        let run = Self::active_run_locked(&*self.state.lock().await, run_id)?;
        let authorization = self.router.classify(tool).await.and_then(|effect| {
            match policy::gate(run.mode, effect) {
                ToolGate::Allow => Ok(effect),
                ToolGate::RequireApproval => Err(
                    "Tool requires interactive approval; only the agent loop can request it"
                        .into(),
                ),
                ToolGate::Deny => Err(format!(
                    "Plan Mode blocked tool: {tool}; only read tools are allowed"
                )),
            }
        });
        if let Err(reason) = authorization {
            self.database
                .record_tool(run_id, tool.into(), reason.clone(), false)
                .await?;
            self.emit_tool(run_id, tool, ToolPhase::Blocked, &reason).await;
            return Err(reason);
        }
        let text = self.router.execute(run.session_id, tool, "{}").await?.text;
        self.database
            .record_tool(run_id, tool.into(), "read-only".into(), true)
            .await?;
        serde_json::from_str(&text).or_else(|_| {
            if text.is_empty() {
                Err("No registered tool executor".into())
            } else {
                Ok(serde_json::Value::String(text))
            }
        })
    }

    /// Tool definitions offered to the model this run, already filtered by the
    /// run's mode: denied tools never reach the request sent to the provider.
    pub async fn agent_tools(&self, mode: Mode) -> Vec<ToolSpec> {
        self.router.tool_specs(mode).await
    }

    /// Record a blocked call in the audit log and emit the Blocked phase.
    async fn block_tool(&self, run_id: i64, tool: &str, reason: String) -> AppResult<String> {
        let _ = self
            .database
            .record_tool(run_id, tool.into(), reason.clone(), false)
            .await;
        self.emit_tool(run_id, tool, ToolPhase::Blocked, &reason).await;
        Err(reason)
    }

    /// Resolve a suspended tool call: `true` executes it, `false` rejects it.
    /// A run holds at most one pending approval; unknown/stale calls error.
    pub async fn resolve_approval(&self, run_id: i64, approved: bool) -> AppResult<()> {
        let _command = self.commands.lock().await;
        let mut state = self.state.lock().await;
        Self::active_run_locked(&state, run_id)?;
        let pending = state
            .control
            .as_mut()
            .and_then(|control| control.pending.take())
            .ok_or("没有等待批准的工具调用")?;
        let _ = pending.responder.send(approved);
        Ok(())
    }

    /// Execute one model-requested tool call: classify, apply the mode gate
    /// (deny / suspend-for-approval / allow), route, audit, and return bounded
    /// text for the next conversation turn. The approval wait never holds the
    /// commands lock, so cancel/resolve/shutdown can always interpose.
    pub(crate) async fn execute_model_tool(
        &self,
        run_id: i64,
        tool: &str,
        arguments_json: &str,
    ) -> AppResult<String> {
        self.execute_tool_calls(run_id, &[(tool.to_string(), arguments_json.to_string())])
            .await
            .into_iter()
            .next()
            .unwrap_or_else(|| Err("missing tool result".into()))
    }

    /// Execute a batch of model-requested calls that share ONE approval
    /// decision: every suspended call emits its own Started + AwaitingApproval
    /// event (each with its own preview), but the batch registers a single
    /// responder — one Approve executes all, one Reject blocks all.
    pub(crate) async fn execute_tool_calls(
        &self,
        run_id: i64,
        calls: &[(String, String)],
    ) -> Vec<AppResult<String>> {
        enum Gate {
            Denied(AppResult<String>),
            Allow(ToolEffect),
            Suspended(ToolEffect),
        }
        let run = {
            let _command = self.commands.lock().await;
            match Self::active_run_locked(&*self.state.lock().await, run_id) {
                Ok(run) => run,
                Err(reason) => return calls.iter().map(|_| Err(reason.clone())).collect(),
            }
        };
        // Phase 1: classify + gate every call before any execution so the
        // whole batch can share one approval wait.
        let mut gates = Vec::with_capacity(calls.len());
        let mut suspended = Vec::new();
        for (index, (tool, arguments_json)) in calls.iter().enumerate() {
            let target = ToolRouter::call_target(tool, arguments_json);
            self.emit_tool(run_id, tool, ToolPhase::Started, &target).await;
            let gate = match self.router.classify(tool).await {
                Err(reason) => Gate::Denied(self.block_tool(run_id, tool, reason).await),
                Ok(effect) => match policy::gate(run.mode, effect) {
                    ToolGate::Deny => Gate::Denied(
                        self.block_tool(
                            run_id,
                            tool,
                            format!("Plan Mode blocked tool: {tool}; only read tools are allowed"),
                        )
                        .await,
                    ),
                    ToolGate::RequireApproval => {
                        suspended.push(index);
                        Gate::Suspended(effect)
                    }
                    ToolGate::Allow => Gate::Allow(effect),
                },
            };
            gates.push(gate);
        }
        // Phase 2: one shared suspension — every gated call reports
        // AwaitingApproval (with its own preview) before the single wait.
        let approved = if suspended.is_empty() {
            true
        } else {
            let (tx, rx) = oneshot::channel();
            let mut waiting = false;
            {
                let mut state = self.state.lock().await;
                if Self::active_run_locked(&state, run_id).is_ok() {
                    if let Some(control) = state.control.as_mut() {
                        control.pending = Some(PendingApproval { responder: tx });
                        waiting = true;
                    }
                }
            }
            for &index in &suspended {
                let preview = self
                    .router
                    .approval_preview(&calls[index].0, &calls[index].1)
                    .await;
                self.emit_tool(run_id, &calls[index].0, ToolPhase::AwaitingApproval, &preview)
                    .await;
            }
            waiting && rx.await.unwrap_or(false)
        };
        // Phase 3: execute (or reject) each call in order.
        let mut results = Vec::with_capacity(calls.len());
        for (index, (tool, arguments_json)) in calls.iter().enumerate() {
            // A batch can be large; stop before the next side effect instead of
            // finishing every call after the user pressed stop.
            let cancelled = {
                let state = self.state.lock().await;
                state
                    .control
                    .as_ref()
                    .is_none_or(|control| *control.cancel.borrow())
            };
            if cancelled {
                results.push(Err("Run cancelled".into()));
                continue;
            }
            match &mut gates[index] {
                Gate::Denied(result) => results.push(std::mem::replace(result, Ok(String::new()))),
                Gate::Suspended(_) if !approved => {
                    results.push(self.block_tool(run_id, tool, "用户拒绝了该操作".into()).await);
                }
                Gate::Allow(effect) | Gate::Suspended(effect) => {
                    let audit = if suspended.contains(&index) {
                        "user approved"
                    } else {
                        "extension"
                    };
                    let target = ToolRouter::call_target(tool, arguments_json);
                    results.push(
                        self.finish_tool(run_id, run.session_id, tool, arguments_json, &target, audit, *effect)
                            .await,
                    );
                }
            }
        }
        results
    }

    /// Execute the routed call, audit the terminal result, emit
    /// Finished/Failed and refresh the workspace on any side effect.
    async fn finish_tool(
        &self,
        run_id: i64,
        session_id: i64,
        tool: &str,
        arguments_json: &str,
        target: &str,
        audit: &str,
        effect: ToolEffect,
    ) -> AppResult<String> {
        match self.router.execute(session_id, tool, arguments_json).await {
            Ok(outcome) => {
                let detail = if target.is_empty() {
                    audit.to_string()
                } else {
                    format!("{audit} · {target}")
                };
                self.database
                    .record_tool(run_id, tool.into(), detail, true)
                    .await?;
                // Task-tracker steps are UI state, not workspace content: emit
                // the dedicated plan event before the generic tool row settles.
                if let Some(steps) = &outcome.plan {
                    let mut state = self.state.lock().await;
                    if Self::active_run_locked(&state, run_id).is_ok() {
                        self.emit_locked(
                            &mut state,
                            EventData::TaskPlanUpdated {
                                run_id,
                                session_id,
                                steps: steps.clone(),
                            },
                        );
                    }
                }
                // Write tools send a structured payload the UI diffs; every
                // other tool sends the bounded result text.
                let event_detail = match &outcome.write {
                    Some(report) => serde_json::json!({
                        "kind": "write_result",
                        "path": report.path,
                        "before": report.before.chars().take(8192).collect::<String>(),
                        "after": report.after.chars().take(8192).collect::<String>(),
                    })
                    .to_string(),
                    None => outcome.text.chars().take(2048).collect::<String>(),
                };
                self.emit_tool(run_id, tool, ToolPhase::Finished, &event_detail).await;
                // Any side-effecting success may have changed the tree.
                if effect != ToolEffect::ReadOnly {
                    let mut state = self.state.lock().await;
                    self.emit_locked(&mut state, EventData::WorkspaceChanged);
                }
                Ok(outcome.text)
            }
            Err(reason) => {
                let _ = self
                    .database
                    .record_tool(run_id, tool.into(), reason.clone(), false)
                    .await;
                self.emit_tool(run_id, tool, ToolPhase::Failed, &reason).await;
                Err(reason)
            }
        }
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

    #[allow(dead_code)]
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
            || state.cancelled_run == Some(run_id)
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
            EventData::Reasoning { run_id, token },
        );
    }

    pub(crate) async fn token(&self, run_id: i64, token: String) {
        let mut state = self.state.lock().await;
        if state.shutting_down
            || state.cancelled_run == Some(run_id)
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
                EventData::Content {
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
        // Reject any approval still suspended before dropping the control so
        // a waiting execute_model_tool resolves as rejected, not as a hang.
        if let Some(control) = state.control.as_mut() {
            if let Some(pending) = control.pending.take() {
                let _ = pending.responder.send(false);
            }
        }
        state.control = None;
        state.cancelled_run = None;
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
            state.control.as_mut().map(|control| {
                let _ = control.cancel.send(true);
                if let Some(pending) = control.pending.take() {
                    let _ = pending.responder.send(false);
                }
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
        self.router.extensions().mcp.shutdown().await;
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
    use crate::adapters::llm::LlmProvider;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    fn test_config() -> ChatConfig {
        ChatConfig {
            api_key: "test-key".into(),
            base_url: "http://localhost/v1".into(),
            model: "test-model".into(),
            provider: crate::adapters::llm::Provider::Openai,
            reasoning: None,
            max_output: None,
            session: None,
        }
    }

    fn draft(title: &str) -> PlanDraft {
        PlanDraft {
            title: title.into(),
            summary: "stub".into(),
            steps: vec!["step".into()],
            diffs: Vec::new(),
        }
    }

    struct StubProvider;

    impl LlmProvider for StubProvider {
        fn stream<'a>(
            self: Arc<Self>,
            _config: &'a ChatConfig,
            _messages: &'a [crate::domain::ChatMessage],
            _tools: &'a [crate::domain::ToolSpec],
            _cancel: &'a mut watch::Receiver<bool>,
            events: tokio::sync::mpsc::UnboundedSender<crate::adapters::llm::StreamEvent>,
        ) -> futures_util::future::BoxFuture<'a, AppResult<crate::adapters::llm::TurnOutcome>> {
            Box::pin(async move {
                use crate::adapters::llm::{StreamEvent, TurnOutcome};
                let _ = events.send(StreamEvent::Token("Hello".into()));
                let _ = events.send(StreamEvent::Token(" 世界".into()));
                let _ = events.send(StreamEvent::Done);
                Ok(TurnOutcome { done: true, tool_calls: Vec::new() })
            })
        }
    }

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
        let (id, _rx) = seed_run(&kernel).await;
        assert!(kernel.start_model(1, "parallel".into(), test_config()).await.is_err());
        assert!(kernel.set_mode(Mode::Execute).await.is_err());
        assert!(kernel.cancel(id + 1).await.is_ok());
        assert!(kernel.authorize_tool(id, "apply_patch").await.is_err());
        assert!(kernel.authorize_tool(id + 1, "read_session").await.is_err());
        kernel.cancel(id).await.unwrap();
        kernel.finish(id, RunStatus::Cancelled, None).await;
        assert_eq!(
            kernel.list_runs().await.unwrap()[0].status,
            RunStatus::Cancelled
        );
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn demo_stream_is_ordered_and_persisted() {
        let (kernel, events) = test_kernel().await;
        *kernel.llm_resolver.lock().unwrap() = Arc::new(|_| Arc::new(StubProvider));
        let session = kernel.create_session(None).await.unwrap();
        kernel
            .start_model(session.id, "hello".into(), test_config())
            .await
            .unwrap();
        wait_for_idle(&kernel).await;
        let runs = kernel.list_runs().await.unwrap();
        assert_eq!(runs[0].status, RunStatus::Completed);
        assert_eq!(runs[0].output, "Hello 世界");
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
                .any(|event| matches!(event.data, EventData::Content { .. })));
        }
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_cancels_and_drains_and_rejects_new_work() {
        let (kernel, _) = test_kernel().await;
        let (_id, _rx) = seed_run(&kernel).await;
        kernel.shutdown().await.unwrap();
        assert!(!kernel.snapshot().await.ready);
        assert_eq!(
            kernel.snapshot().await.last_run.unwrap().status,
            RunStatus::Cancelled
        );
        assert!(kernel.start_model(1, "late".into(), test_config()).await.is_err());
        assert!(kernel.set_mode(Mode::Execute).await.is_err());
    }

    #[tokio::test]
    async fn validates_prompts_on_the_backend() {
        let (kernel, _) = test_kernel().await;
        assert!(kernel.start_model(1, "  ".into(), test_config()).await.is_err());
        assert!(kernel
            .start_model(1, "x".repeat(8193), test_config())
            .await
            .is_err());
        kernel.shutdown().await.unwrap();
    }

    // Seed a run without the demo task to exercise the terminal boundary deterministically.
    async fn seed_run(kernel: &Arc<Kernel>) -> (i64, watch::Receiver<bool>) {
        seed_run_mode(kernel, Mode::Plan).await
    }

    async fn seed_run_mode(kernel: &Arc<Kernel>, mode: Mode) -> (i64, watch::Receiver<bool>) {
        let id = kernel
            .database
            .create_run("boundary".into(), mode)
            .await
            .unwrap();
        let (cancel, receiver) = watch::channel(false);
        let (done_tx, done) = watch::channel(false);
        tokio::spawn({
            let mut receiver = receiver.clone();
            let kernel = kernel.clone();
            async move {
                let _ = receiver.changed().await;
                kernel.finish(id, RunStatus::Cancelled, None).await;
                let _ = done_tx.send(true);
            }
        });
        let session_id = kernel.database.list_runs().await.unwrap()[0].session_id;
        let mut state = kernel.state.lock().await;
        state.active_run = Some(RunView {
            id,
            session_id,
            prompt: "boundary".into(),
            mode,
            status: RunStatus::Running,
            output: String::new(),
            error: None,
            created_at: 0,
        });
        state.control = Some(Control {
            cancel,
            done,
            finishing: false,
            pending: None,
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
        assert!(kernel.cancel(id).await.is_ok());
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
        let draft = draft("test");
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
            .publish_plan(id, draft("cancelled"))
            .await
            .unwrap();
        kernel.cancel(id).await.unwrap();
        assert!(kernel
            .publish_plan(id, draft("late"))
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
        assert!(kernel.start_model(1, "late".into(), test_config()).await.is_err());
        assert!(kernel.authorize_tool(id, "read_session").await.is_err());
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event.data, EventData::BackendError { .. })));
    }

    #[tokio::test]
    async fn execute_mode_is_not_selectable_over_ipc() {
        let (kernel, _) = test_kernel().await;
        assert!(kernel.set_mode(Mode::Execute).await.is_err());
        assert_eq!(kernel.snapshot().await.mode, Mode::Plan);
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn tool_specs_are_filtered_by_mode() {
        let (kernel, _) = test_kernel().await;
        let plan: Vec<String> = kernel
            .agent_tools(Mode::Plan)
            .await
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert!(plan.iter().any(|name| name == "read_file"));
        assert!(plan.iter().any(|name| name == "load_skill"));
        assert!(plan.iter().any(|name| name == "update_plan"));
        assert!(!plan.iter().any(|name| name == "write_file"));
        assert!(!plan.iter().any(|name| name == "apply_patch"));
        assert!(!plan.iter().any(|name| name == "run_command"));
        for mode in [Mode::Code, Mode::Confirm, Mode::Auto] {
            let names: Vec<String> = kernel
                .agent_tools(mode)
                .await
                .into_iter()
                .map(|tool| tool.name)
                .collect();
            assert!(names.iter().any(|name| name == "write_file"), "{mode:?}");
            assert!(names.iter().any(|name| name == "run_command"), "{mode:?}");
        }
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn update_plan_emits_task_plan_event_in_plan_mode() {
        let (kernel, events) = test_kernel().await;
        let (id, _rx) = seed_run(&kernel).await;
        let result = kernel
            .execute_model_tool(
                id,
                "update_plan",
                r#"{"steps":[
                    {"id":"1","description":"分析项目","status":"completed"},
                    {"id":"2","description":"重构核心文件","status":"in_progress"}
                ]}"#,
            )
            .await
            .unwrap();
        assert!(result.contains("1/2"));
        let emitted = events.lock().unwrap().clone();
        let steps = emitted
            .iter()
            .find_map(|event| match &event.data {
                EventData::TaskPlanUpdated { steps, run_id, .. } if *run_id == id => Some(steps.clone()),
                _ => None,
            })
            .expect("task_plan_updated must be emitted");
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[1].status, crate::domain::PlanStepStatus::InProgress);
        assert!(kernel
            .execute_model_tool(id, "update_plan", r#"{"steps":[]}"#)
            .await
            .is_err());
        kernel.finish(id, RunStatus::Completed, None).await;
        kernel.shutdown().await.unwrap();
    }

    async fn wait_for_phase(
        events: &Arc<StdMutex<Vec<KernelEvent>>>,
        phase: ToolPhase,
    ) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|event| matches!(&event.data, EventData::ToolCall { phase: p, .. } if *p == phase))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn code_mode_write_suspends_for_approval_then_executes() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        let worker = kernel.clone();
        let call = tokio::spawn(async move {
            worker
                .execute_model_tool(id, "write_file", r#"{"path":"a.txt","content":"hello"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel.resolve_approval(id, true).await.unwrap();
        assert!(call.await.unwrap().is_ok());
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("a.txt")).unwrap(),
            "hello"
        );
        kernel.finish(id, RunStatus::Completed, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn rejected_approval_blocks_the_write() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        let worker = kernel.clone();
        let call = tokio::spawn(async move {
            worker
                .execute_model_tool(id, "write_file", r#"{"path":"b.txt","content":"nope"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel.resolve_approval(id, false).await.unwrap();
        assert!(call.await.unwrap().is_err());
        assert!(!workspace.path().join("b.txt").exists());
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event.data, EventData::ToolCall { phase: ToolPhase::Blocked, .. })));
        kernel.finish(id, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn cancel_is_not_queued_behind_other_commands() {
        let (kernel, _) = test_kernel().await;
        let (id, _receiver) = seed_run(&kernel).await;
        // Simulate a long mutating command holding the command gate: stop must
        // still reach the run instead of waiting for the queue.
        let gate = kernel.commands.lock().await;
        let cancelled = tokio::time::timeout(Duration::from_millis(200), kernel.cancel(id)).await;
        drop(gate);
        assert!(cancelled.is_ok(), "cancel must not wait for the command gate");
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_run_flag_is_visible_to_the_loop() {
        let (kernel, _) = test_kernel().await;
        let (id, _receiver) = seed_run(&kernel).await;
        assert!(!kernel.is_cancelled(id).await);
        kernel.state.lock().await.cancelled_run = Some(id);
        assert!(kernel.is_cancelled(id).await);
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn cancel_rejects_a_suspended_approval() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        let worker = kernel.clone();
        let call = tokio::spawn(async move {
            worker
                .execute_model_tool(id, "write_file", r#"{"path":"c.txt","content":"x"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel.cancel(id).await.unwrap();
        assert!(call.await.unwrap().is_err());
        assert!(!workspace.path().join("c.txt").exists());
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn code_mode_run_command_suspends_then_executes_after_approval() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        let worker = kernel.clone();
        let call = tokio::spawn(async move {
            worker
                .execute_model_tool(id, "run_command", r#"{"command":"echo approved-run"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel.resolve_approval(id, true).await.unwrap();
        let output = call.await.unwrap().unwrap();
        assert!(output.contains("exit code 0"), "{output}");
        assert!(output.contains("approved-run"), "{output}");
        kernel.finish(id, RunStatus::Completed, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn confirm_mode_suspends_even_read_tools() {
        let (kernel, events) = test_kernel().await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Confirm).await;
        let worker = kernel.clone();
        let call = tokio::spawn(async move {
            worker.execute_model_tool(id, "list_sessions", "{}").await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel.resolve_approval(id, true).await.unwrap();
        assert!(call.await.unwrap().is_ok());
        kernel.finish(id, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn load_skill_body_is_wrapped_in_a_system_warning() {
        let (kernel, _) = test_kernel().await;
        let skill_dir = kernel
            .extensions()
            .maestro_dir()
            .join("skills")
            .join("demo-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: demo-skill\n---\n\nskill body text",
        )
        .unwrap();
        let (id, _rx) = seed_run(&kernel).await;
        let text = kernel
            .execute_model_tool(id, "load_skill", r#"{"name":"demo-skill"}"#)
            .await
            .unwrap();
        assert!(text.contains("<system_warning>"));
        assert!(text.contains("skill body text"));
        kernel.finish(id, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }
}
