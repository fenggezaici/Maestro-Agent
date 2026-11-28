use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{atomic::{AtomicU64, Ordering}, Arc},
};

use tokio::sync::{oneshot, watch, Mutex};

use std::sync::Mutex as StdMutex;

use crate::{
    adapters::{
        credentials::CredentialStore,
        db::Database,
        llm::{provider_for, ChatConfig, LlmProvider, ProviderResolver},
    },
    domain::{
        command_policy,
        policy::{self, ToolEffect, ToolGate},
        AppResult, ChatMessage, EventData, KernelEvent, Mode, PlanCard, PlanDecision, PlanDraft,
        ProjectView, RunMetrics, RunStatus, RunView, SessionDetail, SessionView, Snapshot,
        format_answers, parse_questions, validate_answers, ApprovalCandidate, ApprovalDecision, ApprovalRule, ReasoningSegment, RunPart,
        canonical_json, line_change_counts, merge_file_changes, ArmedObligation, ContextCompaction, GraphDecision, HistoryEntry,
        MAX_RULE_PATTERN_CHARS, TraceStatus, TraceStep,
        CommandScope, Question, QuestionAnswer, RunMessage, SubagentStatus,
        SubagentTokenKind, SubagentView, ToolCallView, ToolPhase, ToolSpec, WorkspaceTrustStatus,
    },
    services::{agent_loop, extensions::ExtensionManager, tool_router::{ToolOutcome, ToolRouter}},
};

/// 工具执行产出的模型可见载荷：文本 + 图像 data URL 列表（目前仅
/// read_file 读图产生）。Err 路径与此前一致——错误本就纯文本。
type ToolPayload = (String, Vec<String>);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

pub const EVENT_NAME: &str = "kernel-event";
pub type EventSink = Arc<dyn Fn(KernelEvent) + Send + Sync>;

struct Control {
    cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
    finishing: bool,
    /// Workspace root captured when this run started. Tool execution must use
    /// this snapshot — a mid-run Explorer/project switch can change the global
    /// root, and writes/commands must stay in the directory the run began in.
    workspace_root: Option<PathBuf>,
    /// A tool call suspended awaiting the user's approval decision. At most
    /// one can exist (the agent loop is serial); dropping the responder
    /// without answering counts as rejection.
    pending: Option<PendingApproval>,
    /// 本批剩余挂起调用的批量决定（allow_batch/deny_batch 写入）。在
    /// `execute_tool_calls_scoped` 入口清空——决定不外泄到下一批。
    bulk_decision: Option<BulkDecision>,
    /// `ask_user` suspension; separate from approvals so neither overwrites
    /// the other. Dropping the responder counts as cancellation.
    pending_question: Option<PendingQuestion>,
}

#[derive(Clone)]
struct BulkDecision {
    approved: bool,
    feedback: Option<String>,
}

struct PendingApproval {
    responder: oneshot::Sender<ApprovalResponse>,
    session_id: i64,
    candidates: Vec<ApprovalCandidate>,
}

struct ApprovalResponse {
    decision: ApprovalDecision,
    feedback: Option<String>,
}

struct PendingQuestion {
    request_id: String,
    questions: Vec<Question>,
    responder: oneshot::Sender<Option<Vec<QuestionAnswer>>>,
}

static QUESTION_SEQ: AtomicU64 = AtomicU64::new(1);

/// Per-run metrics accumulate across model turns; the relay reports one
/// snapshot per turn, so a single `RunMetrics` row otherwise only keeps the
/// last turn.
#[derive(Default)]
struct MetricsAccumulator {
    /// Identity of the reporting stream, kept so a tool-close snapshot can
    /// re-emit the same cumulative row without a model turn in flight.
    run_id: i64,
    session_id: i64,
    provider: String,
    model: String,
    /// Last prompt size the provider ever reported. Metrics arrive before usage
    /// exists, so the projection must never regress to `None` once known.
    last_known_input: Option<u64>,
    input: Option<u64>,
    output: Option<u64>,
    cache: Option<u64>,
    cache_write: Option<u64>,
    reasoning: Option<u64>,
    attempts: u32,
    elapsed: u64,
    /// Σ 首 token 延迟，只覆盖记录了首 delta 的完成请求（DSH ttftSteps 口径）。
    first: u64,
    ttft_steps: u32,
    /// Σ decode 窗口（首 delta → 流结束），只在「首 token 与 output usage
    /// 同有」的请求上成对累计 —— DSH sessionStats decodeMs/decodeTokens。
    decode_ms: u64,
    decode_tokens: u64,
    /// Σ 工具墙钟：emit_tool_scoped 的 Started→终态配对（DSH tool/call→result）。
    tool_ms: u64,
    /// 未完成工具调用的 Started 时刻队列；终态按 FIFO 配对（事件本就按序发出）。
    tool_starts: VecDeque<i64>,
    /// 最近一次完成请求的快照字段，供工具配对快照与持久化使用。
    last_output: Option<u64>,
    last_elapsed: u64,
    last_first: Option<u64>,
    current: Option<RunMetrics>,
    /// 自上次持久化后 tool_ms 有新增：finish 时补一次最终落库。
    dirty: bool,
}

/// `list_approval_rules` 中会话全部放行合成行的 tool 字段。
const SESSION_GRANT_TOOL: &str = "*";

/// Why a call ran without a prompt — ordered strongest evidence first.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AuditKind {
    /// 历史 "always allow" 规则命中。
    RuleApproved,
    /// 本会话「全部放行」生效中。
    SessionGranted,
    /// 只读白名单直接执行（无 shell）。
    ReadOnlyAllowlist,
    /// 自主模式：未命中危险规则。
    Autonomous,
    /// 其余工具（MCP 只读、扩展工具等）。
    Extension,
}

fn sum_tokens(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (None, None) => None,
        (left, right) => Some(left.unwrap_or(0) + right.unwrap_or(0)),
    }
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
    /// Reasoning bursts of the active run; persisted on finish so replay can
    /// still show "思考 X 秒" after a restart.
    reasoning: Vec<ReasoningSegment>,
    /// Ordered timeline parts of the active run (ZCode-style parts).
    parts: Vec<RunPart>,
    reasoning_run: Option<i64>,
    shutting_down: bool,
    storage_failed: bool,
}

/// Per-run 按路径累计的 (首版 before, 最新 after) 净差表。
type RunWriteNet = StdMutex<HashMap<i64, HashMap<String, (String, String)>>>;

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
    /// Live child traces keyed by subagent id; persisted copies live in SQLite.
    subagents: StdMutex<HashMap<String, SubagentView>>,
    /// Cancellation channels for running children, so one child can stop alone.
    subagent_cancels: StdMutex<HashMap<String, watch::Sender<bool>>>,
    metrics_acc: StdMutex<HashMap<i64, MetricsAccumulator>>,
    /// Per-run 按路径累计的 首版 before → 最新 after：write_result 载荷里的
    /// before/after 有截断上限，面板合并多次写入时无法重算净差，由此下发权威计数。
    write_net: RunWriteNet,
    /// Session-scoped "always allow" rules; cleared when the app exits.
    approval_rules: StdMutex<HashMap<i64, Vec<ApprovalRule>>>,
    /// 会话级「全部放行」（ZCode full access 对应物）：纯内存不落库，
    /// 应用退出即失效；只能在非 Plan 模式下豁免 RequireApproval。
    full_access_sessions: StdMutex<HashSet<i64>>,
    /// Provider API keys at rest (DPAPI on Windows); the webview never persists them.
    credentials: CredentialStore,
    /// 弱自引用：`set_workspace_root` 触发后台 MCP 补恢复需要 'static Kernel。
    self_ref: std::sync::OnceLock<std::sync::Weak<Kernel>>,
    /// 上次 MCP 恢复所见到的工作区根——root 变更时幂等补跑一次恢复
    ///（启动恢复时 root 尚未就位，工作区相对命令的解析不同）。
    mcp_restored_for: Mutex<Option<PathBuf>>,
}

impl Kernel {
    pub async fn open(path: PathBuf, maestro_dir: PathBuf, sink: EventSink) -> AppResult<Arc<Self>> {
        let database = Arc::new(Database::open(path).await?);
        let last_run = database.list_runs().await?.into_iter().next();
        let credentials = CredentialStore::open(&maestro_dir.join("credentials"))?;
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
                reasoning: Vec::new(),
                parts: Vec::new(),
                reasoning_run: None,
                shutting_down: false,
                storage_failed: false,
            }),
            commands: Mutex::new(()),
            sink,
            llm_resolver: StdMutex::new(Arc::new(|provider| provider_for(provider))),
            subagents: StdMutex::new(HashMap::new()),
            subagent_cancels: StdMutex::new(HashMap::new()),
            metrics_acc: StdMutex::new(HashMap::new()),
            write_net: StdMutex::new(HashMap::new()),
            approval_rules: StdMutex::new(HashMap::new()),
            full_access_sessions: StdMutex::new(HashSet::new()),
            credentials,
            self_ref: std::sync::OnceLock::new(),
            mcp_restored_for: Mutex::new(None),
        });
        let _ = kernel.self_ref.set(Arc::downgrade(&kernel));
        // 全局架构规则不依赖工作区，启动即装载（项目规则随 set_workspace_root 生效）。
        kernel.router.reload_architecture().await;
        kernel.emit_locked(&mut *kernel.state.lock().await, EventData::Ready);
        Ok(kernel)
    }

    pub fn extensions(&self) -> &ExtensionManager {
        self.router.extensions()
    }

    pub fn credentials(&self) -> &CredentialStore {
        &self.credentials
    }

    /// Startup restore for MCP servers the user left enabled. Failures stay
    /// per-server and never block the app.
    pub async fn start_enabled_mcp(&self) {
        self.extensions()
            .mcp
            .start_enabled(&|_| self.emit_extensions_changed())
            .await;
    }

    /// MCP 状态变化的内容无关刷新提示：try_lock 争用时可丢——下一次
    /// 状态变更会再发一次，事件本身不携带数据。
    fn emit_extensions_changed(&self) {
        if let Ok(mut state) = self.state.try_lock() {
            self.emit_locked(&mut state, EventData::ExtensionsChanged);
        }
    }

    /// Resolve the LLM adapter for this run's provider.
    pub(crate) fn llm_provider(&self, config: &ChatConfig) -> Arc<dyn LlmProvider> {
        (self.llm_resolver.lock().unwrap())(config.provider)
    }

    /// stream_chat supplies the workspace so skills/tools resolve project files.
    /// A directory carrying untrusted `.maestro/` assets raises a one-shot
    /// trust request — project-scope scans stay empty until `trust_workspace`.
    pub async fn set_workspace_root(&self, root: Option<PathBuf>) {
        if let Some(status) = self.workspace_trust_status(root.as_deref()).await {
            let mut state = self.state.lock().await;
            self.emit_locked(&mut state, EventData::WorkspaceTrustRequired {
                path: status.path,
                assets: status.assets,
            });
        }
        self.router.set_workspace_root(root.clone()).await;
        // 启动恢复发生时 root 尚未就位——工作区相对命令的解析不同，恢复时
        // 失败的 spawn 在手动重勾时反而能成。root 变更时后台幂等补跑一次
        //（running 中的服务早退，不阻塞本调用）。
        if let Some(path) = root {
            let mut restored = self.mcp_restored_for.lock().await;
            if restored.as_ref() != Some(&path) {
                *restored = Some(path);
                drop(restored);
                if let Some(kernel) = self.self_ref.get().and_then(|weak| weak.upgrade()) {
                    tauri::async_runtime::spawn(async move {
                        kernel.start_enabled_mcp().await;
                    });
                }
            }
        }
    }

    /// `Some` when the root carries project `.maestro/` assets the user has
    /// not yet trusted — the payload the UI needs for its trust prompt.
    pub async fn workspace_trust_status(&self, root: Option<&std::path::Path>) -> Option<WorkspaceTrustStatus> {
        let root = root?;
        let assets = crate::adapters::agents::has_maestro_assets(root);
        if assets.is_empty() {
            return None;
        }
        let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let trusted = self
            .router
            .is_trusted_canonical(&canonical.to_string_lossy())
            .await;
        (!trusted).then(|| WorkspaceTrustStatus {
            path: root.to_string_lossy().to_string(),
            assets: assets.iter().map(|dir| dir.to_string()).collect(),
        })
    }

    /// Trust grant recorded against the canonical path so junction/relative
    /// spellings of the same directory cannot bypass the gate.
    pub async fn trust_workspace(&self, path: String) -> AppResult<()> {
        let root = PathBuf::from(&path);
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        self.database
            .trust_workspace(canonical.to_string_lossy().to_string())
            .await?;
        // 同步内存副本——trusted_project_root 的读路径是零 DB 缓存。
        self.router.mark_trusted(canonical.to_string_lossy().to_string());
        let mut state = self.state.lock().await;
        self.emit_locked(&mut state, EventData::WorkspaceTrusted { path });
        Ok(())
    }

    /// Project-scope asset scans resolve through this — `None` when the
    /// workspace is untrusted so `.maestro/` content stays inert.
    pub async fn trusted_project_root(&self, root: Option<&std::path::Path>) -> Option<PathBuf> {
        self.router.trusted_project_root(root).await
    }

    /// 启动预热：载入 trusted_workspaces 内存副本，run 期间读路径零 DB。
    pub async fn prefetch_workspace_trust(&self) {
        self.router.prefetch_trusted().await;
    }

    /// e2e/debug 注入面：在 DB 里建真 session + running run，并经 emit_locked
    /// 注入 Content 洪水（真实 channel、sequence、合并窗口）——回归
    /// 「流式期间 invoke/沙盘加载不被 JS 渲染饥饿饿死」。仅 debug 构建存在。
    #[cfg(debug_assertions)]
    pub async fn debug_fake_stream(
        self: &Arc<Self>,
        session_id: Option<i64>,
        seconds: u32,
    ) -> AppResult<(i64, i64)> {
        let session_id = match session_id {
            Some(id) => id,
            None => self.create_session(None).await?.id,
        };
        let run_id = self
            .database
            .create_run_in_session(session_id, "e2e flood".into(), Mode::Auto, Vec::new())
            .await?;
        let kernel = self.clone();
        tauri::async_runtime::spawn(async move {
            // ~1KB token 每 25ms：与真实流式同速率注入，未修复的渲染路径
            // （每事件全量 concat + remark 重解析）会被迅速压满。
            let chunk = "0123456789abcdef e2e-flood token burst.\n".repeat(24);
            for _ in 0..seconds.saturating_mul(40) {
                {
                    let mut state = kernel.state.lock().await;
                    kernel.emit_locked(&mut state, EventData::Content {
                        run_id,
                        session_id,
                        token: chunk.clone(),
                    });
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            let mut state = kernel.state.lock().await;
            kernel.emit_locked(&mut state, EventData::SessionsChanged);
        });
        Ok((session_id, run_id))
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
        let (prompt, images) = crate::domain::validate_messages(&messages)?;
        let id = match session_id {
            Some(id) => id,
            None => self.create_session(None).await?.id,
        };
        self.start_model(id, prompt, images, config).await
    }

    pub async fn start_model(
        self: &Arc<Self>,
        session_id: i64,
        prompt: String,
        images: Vec<String>,
        config: ChatConfig,
    ) -> AppResult<i64> {
        config.validate()?;
        self.start_run(session_id, prompt, images, config).await
    }

    async fn start_run(
        self: &Arc<Self>,
        session_id: i64,
        prompt: String,
        images: Vec<String>,
        mut config: ChatConfig,
    ) -> AppResult<i64> {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() || prompt.len() > 8192 {
            return Err("Prompt must contain 1–8192 UTF-8 bytes".into());
        }
        // OpenCode gateways route/cache per conversation: keep one stable id
        // for every turn of this session.
        config.session = Some(format!("maestro-{session_id}"));
        // 视觉门控随 run 生效：read_file 读图据此决定注入图像还是降级元数据。
        self.router.set_vision_capable(config.vision_capable).await;
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
            .create_run_in_session(session_id, prompt.clone(), mode, images)
            .await?;
        self.metrics_acc.lock().unwrap().remove(&id);
        self.write_net.lock().unwrap().remove(&id);
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
        let run_root = self.router.workspace_root().await;
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let (done_tx, done_rx) = watch::channel(false);
        {
            let mut state = self.state.lock().await;
            state.active_run = Some(run.clone());
            state.control = Some(Control {
                cancel: cancel_tx,
                done: done_rx,
                finishing: false,
                workspace_root: run_root,
                pending: None,
                bulk_decision: None,
                pending_question: None,
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
        // Idempotent best-effort: a stale id (e.g. a retried abort for a run
        // that already finished) must never cancel a NEW active run or its
        // children — scope every effect to the run the caller actually saw.
        {
            let mut map = self.subagents.lock().unwrap();
            let senders: Vec<_> = self
                .subagent_cancels
                .lock()
                .unwrap()
                .iter()
                .filter(|(id, _)| {
                    map.get(*id)
                        .is_none_or(|view| view.run_id == run_id)
                })
                .map(|(_, sender)| sender.clone())
                .collect();
            for sender in senders {
                let _ = sender.send(true);
            }
            // Cascade: child subagents must stop even when the parent run is
            // already gone (otherwise a stopped run leaves orphaned children).
            for view in map.values_mut() {
                if view.run_id == run_id && view.status == SubagentStatus::Running {
                    view.status = SubagentStatus::Cancelled;
                    view.finished_at = Some(crate::domain::unix_now());
                }
            }
        }
        // Cancellation must never queue behind the command gate: a long
        // mutating command would otherwise make "stop" feel dead.
        let mut state = self.state.lock().await;
        let active = state.active_run.as_ref().map(|run| run.id);
        if active != Some(run_id) {
            return Ok(());
        }
        let Some(control) = state.control.as_mut() else { return Ok(()) };
        if control.finishing {
            return Ok(());
        }
        // send_replace also works when no receiver is currently polling.
        control.cancel.send_replace(true);
        // Unblock any call suspended on an approval so the loop can observe
        // the cancellation and finish instead of hanging on the user's answer.
        if let Some(pending) = control.pending.take() {
            let _ = pending.responder.send(ApprovalResponse {
                decision: ApprovalDecision::Deny,
                feedback: None,
            });
        }
        if let Some(question) = control.pending_question.take() {
            let _ = question.responder.send(None);
        }
        // Flag after the control borrow ends: the loop polls this even if the
        // watch notification is lost.
        state.cancelled_run = active;
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
    pub(crate) async fn emit_tool(&self, run_id: i64, tool: &str, phase: ToolPhase, detail: &str) {
        self.emit_tool_scoped(run_id, None, tool, phase, detail).await;
    }

    /// Spawn rows carry their trace id so the UI can open the drill-down.
    pub(crate) async fn emit_spawn(
        &self,
        run_id: i64,
        subagent_id: &str,
        phase: ToolPhase,
        detail: &str,
    ) {
        let mut state = self.state.lock().await;
        if Self::active_run_locked(&state, run_id).is_ok() {
            if phase == ToolPhase::Started {
                if let Some(part) = state.parts.last_mut() {
                    if part.ended_at.is_none() {
                        part.ended_at = Some(now_ms());
                    }
                }
                let index = state.parts.iter().filter(|part| part.kind == "tool").count();
                state.parts.push(RunPart {
                    kind: "tool".into(),
                    text: String::new(),
                    index: Some(index),
                    started_at: now_ms(),
                    ended_at: None,
                });
            }
            self.emit_locked(
                &mut state,
                EventData::ToolCall {
                    run_id,
                    tool: "spawn_subagent".into(),
                    phase,
                    detail: detail.into(),
                    subagent_id: Some(subagent_id.into()),
                },
            );
        }
    }

    async fn emit_tool_scoped(
        &self,
        run_id: i64,
        subagent_id: Option<&str>,
        tool: &str,
        phase: ToolPhase,
        detail: &str,
    ) {
        if let Some(id) = subagent_id {
            self.subagent_tool(run_id, id, tool, phase, detail).await;
            return;
        }
        // DSH tool/call→tool/result 口径：父级工具按 Started→终态（Finished/
        // Failed/Blocked）FIFO 配对累计墙钟；ask_user 是用户等待而非工具执行，
        // spawn_subagent 走 emit_spawn 独立路径，均不计入 tool_ms。
        if tool != "ask_user" {
            let snapshot = {
                let mut map = self.metrics_acc.lock().unwrap();
                // 工具可能早于首个 metrics 报告到达（首个请求 <200ms 完成即派
                // 工具），此时 acc 尚无身份：照样记录配对，只在已有身份的 acc
                // 上 emit 快照，避免发出 run_id=0 的空 Metrics。
                let acc = map.entry(run_id).or_default();
                let now = now_ms();
                let mut snapshot = None;
                match phase {
                    ToolPhase::Started => acc.tool_starts.push_back(now),
                    ToolPhase::Finished | ToolPhase::Failed | ToolPhase::Blocked => {
                        if let Some(started) = acc.tool_starts.pop_front() {
                            acc.tool_ms += now.saturating_sub(started) as u64;
                            acc.dirty = true;
                            if acc.run_id != 0 {
                                snapshot = Some(Self::metrics_view(acc));
                            }
                        }
                    }
                    _ => {}
                }
                snapshot
            };
            if let Some(metrics) = snapshot {
                let mut state = self.state.lock().await;
                if Self::active_run_locked(&state, run_id).is_ok() {
                    self.emit_locked(&mut state, EventData::Metrics { metrics });
                }
            }
        }
        let mut state = self.state.lock().await;
        if Self::active_run_locked(&state, run_id).is_ok() {
            self.emit_locked(
                &mut state,
                EventData::ToolCall {
                    run_id,
                    tool: tool.into(),
                    phase,
                    detail: detail.into(),
                    subagent_id: None,
                },
            );
        }
    }

    pub(crate) fn register_subagent_cancel(&self, id: String, sender: watch::Sender<bool>) {
        self.subagent_cancels.lock().unwrap().insert(id, sender);
    }

    pub(crate) fn unregister_subagent_cancel(&self, id: &str) {
        self.subagent_cancels.lock().unwrap().remove(id);
    }

    /// Stop one child without touching its siblings or the parent run.
    pub async fn cancel_subagent(&self, run_id: i64, subagent_id: &str) -> AppResult<()> {
        // The stop button must work even after the parent run finished: the
        // child keeps its own cancel handle and status.
        let _ = run_id;
        if let Some(sender) = self.subagent_cancels.lock().unwrap().get(subagent_id) {
            let _ = sender.send(true);
        }
        let mut map = self.subagents.lock().unwrap();
        if let Some(view) = map.get_mut(subagent_id) {
            if view.status == SubagentStatus::Running {
                view.status = SubagentStatus::Cancelled;
                view.finished_at = Some(crate::domain::unix_now());
            }
        }
        Ok(())
    }

    /// Persist and announce a new child trace. Called before the first turn so
    /// the drill-down can open immediately.
    pub async fn start_subagent(&self, view: SubagentView) -> AppResult<()> {
        self.database.insert_subagent_trace(view.clone()).await?;
        self.subagents
            .lock()
            .unwrap()
            .insert(view.id.clone(), view.clone());
        let mut state = self.state.lock().await;
        self.emit_locked(
            &mut state,
            EventData::SubagentStarted {
                run_id: view.run_id,
                subagent: view,
            },
        );
        Ok(())
    }

    pub async fn subagent_token(
        &self,
        run_id: i64,
        subagent_id: &str,
        kind: SubagentTokenKind,
        token: &str,
    ) {
        {
            let mut map = self.subagents.lock().unwrap();
            if let Some(view) = map.get_mut(subagent_id) {
                match kind {
                    SubagentTokenKind::Content => view.content.push_str(token),
                    SubagentTokenKind::Reasoning => view.reasoning.push_str(token),
                }
            }
        }
        let mut state = self.state.lock().await;
        if Self::active_run_locked(&state, run_id).is_ok() {
            self.emit_locked(
                &mut state,
                EventData::SubagentToken {
                    run_id,
                    subagent_id: subagent_id.into(),
                    kind,
                    token: token.into(),
                },
            );
        }
    }

    pub(crate) async fn subagent_tool(
        &self,
        run_id: i64,
        subagent_id: &str,
        tool: &str,
        phase: ToolPhase,
        detail: &str,
    ) {
        {
            let mut map = self.subagents.lock().unwrap();
            if let Some(view) = map.get_mut(subagent_id) {
                Self::apply_trace_tool(view, tool, phase, detail);
            }
        }
        let mut state = self.state.lock().await;
        if Self::active_run_locked(&state, run_id).is_ok() {
            self.emit_locked(
                &mut state,
                EventData::SubagentTool {
                    run_id,
                    subagent_id: subagent_id.into(),
                    tool: tool.into(),
                    phase,
                    detail: detail.into(),
                },
            );
        }
    }

    /// Persist the terminal trace state and notify the UI.
    #[allow(clippy::too_many_arguments)]
    pub async fn finish_subagent(
        &self,
        subagent_id: &str,
        status: SubagentStatus,
        summary: String,
        error: Option<String>,
        content: String,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        reasoning_tokens: Option<u64>,
        first_token_ms: Option<u64>,
        elapsed_ms: u64,
        decode_ms: u64,
        decode_tokens: u64,
        last_input_tokens: Option<u64>,
    ) -> AppResult<()> {
        let mut view = self
            .subagents
            .lock()
            .unwrap()
            .get(subagent_id)
            .cloned()
            .ok_or("Subagent trace not found")?;
        if matches!(view.status, SubagentStatus::Completed | SubagentStatus::Failed | SubagentStatus::Cancelled) {
            return Ok(());
        }
        let finished_at = crate::domain::unix_now();
        view.status = status;
        view.summary = summary.clone();
        view.error = error.clone();
        view.content = content;
        view.tool_count = view.tools.len();
        view.input_tokens = input_tokens.or(view.input_tokens);
        view.output_tokens = output_tokens.or(view.output_tokens);
        view.reasoning_tokens = reasoning_tokens.or(view.reasoning_tokens);
        view.first_token_ms = first_token_ms.or(view.first_token_ms);
        view.elapsed_ms = elapsed_ms;
        view.decode_ms = decode_ms;
        view.decode_tokens = decode_tokens;
        view.last_input_tokens = last_input_tokens.or(view.last_input_tokens);
        view.finished_at = Some(finished_at);
        self.database
            .finish_subagent_trace(
                view.id.clone(),
                status,
                summary.clone(),
                error.clone(),
                view.content.clone(),
                view.tools.clone(),
                view.input_tokens,
                view.output_tokens,
                view.reasoning_tokens,
                view.first_token_ms,
                view.elapsed_ms,
                view.decode_ms,
                view.decode_tokens,
                view.last_input_tokens,
                finished_at,
            )
            .await?;
        self.subagents
            .lock()
            .unwrap()
            .insert(view.id.clone(), view.clone());
        self.unregister_subagent_cancel(subagent_id);
        let mut state = self.state.lock().await;
        if Self::active_run_locked(&state, view.run_id).is_ok() {
            self.emit_locked(
                &mut state,
                EventData::SubagentFinished {
                    run_id: view.run_id,
                    subagent_id: view.id.clone(),
                    status,
                    summary,
                    error,
                    tool_count: view.tool_count,
                    input_tokens: view.input_tokens,
                    output_tokens: view.output_tokens,
                    reasoning_tokens: view.reasoning_tokens,
                    first_token_ms: view.first_token_ms,
                    elapsed_ms: view.elapsed_ms,
                    decode_ms: view.decode_ms,
                    decode_tokens: view.decode_tokens,
                    last_input_tokens: view.last_input_tokens,
                    finished_at,
                },
            );
        }
        Ok(())
    }

    pub async fn save_run_messages(&self, messages: Vec<RunMessage>) -> AppResult<()> {
        self.database.save_run_messages(messages).await
    }

    pub async fn save_compaction(&self, compaction: ContextCompaction) -> AppResult<()> {
        self.database.save_compaction(compaction.clone()).await?;
        let mut state = self.state.lock().await;
        self.emit_locked(&mut state, EventData::ContextCompaction { compaction });
        Ok(())
    }

    /// Manual `/compact`: fold terminal runs into one persisted summary. The
    /// next run replays the summary instead of the covered runs.
    pub async fn compact_session(&self, session_id: i64) -> AppResult<()> {
        let detail = self.session_detail(session_id).await?;
        let terminal: Vec<_> = detail
            .runs
            .iter()
            .filter(|run| run.status != RunStatus::Running)
            .collect();
        let Some(latest) = terminal.last() else {
            return Ok(());
        };
        let mut summary = String::from("[手动压缩] 会话历史摘要：\n");
        for run in &terminal {
            summary.push_str(&format!(
                "- [{}] {}: {}\n",
                run.status.as_str(),
                run.prompt.chars().take(200).collect::<String>(),
                run.output.chars().take(500).collect::<String>()
            ));
            let calls: Vec<_> = detail.tools.iter().filter(|call| call.run_id == run.id).collect();
            let start = calls.len().saturating_sub(12);
            for call in &calls[start..] {
                summary.push_str(&format!(
                    "  · {} · {}\n",
                    call.tool,
                    call.detail.replace(['\n', '\r'], " ").chars().take(120).collect::<String>()
                ));
            }
        }
        let summary: String = summary.chars().take(12000).collect();
        let pre: u64 = terminal
            .iter()
            .map(|run| (run.prompt.len() + run.output.len()) as u64 / 4)
            .sum();
        let compaction = ContextCompaction {
            operation_id: format!("compact-manual-{}-{}", session_id, crate::domain::unix_now()),
            session_id,
            through_run_id: latest.id,
            post_tokens: Some((summary.len() / 4) as u64),
            summary,
            trigger: "manual".into(),
            status: "completed".into(),
            pre_tokens: pre,
            created_at: crate::domain::unix_now(),
        };
        self.save_compaction(compaction).await
    }

    pub async fn latest_compaction(&self, session_id: i64) -> AppResult<Option<ContextCompaction>> {
        Ok(self
            .database
            .list_compactions(session_id)
            .await?
            .into_iter()
            .rfind(|item| item.status == "completed"))
    }

    pub async fn list_subagent_traces(&self, session_id: i64) -> AppResult<Vec<SubagentView>> {
        let mut traces = self.database.list_subagent_traces(session_id).await?;
        let live = self.subagents.lock().unwrap();
        for trace in &mut traces {
            if let Some(view) = live.get(&trace.id) {
                *trace = view.clone();
            }
        }
        Ok(traces)
    }

    /// Bounded report block injected into later turns of the same session.
    pub async fn recent_subagent_reports(&self, session_id: i64) -> AppResult<String> {
        let traces = self
            .database
            .recent_subagent_reports(session_id, 6)
            .await?;
        Ok(Self::format_subagent_reports(&traces))
    }

    /// `history_target` 约定为 `"type/description"`——Requirement/派发义务按类型前缀命中。
    pub(crate) async fn record_spawn(
        &self,
        run_id: i64,
        subagent_id: &str,
        description: &str,
        history_target: &str,
    ) {
        let _ = self
            .database
            .record_tool(
                run_id,
                "spawn_subagent".into(),
                format!("{subagent_id} · {description}"),
                true,
            )
            .await;
        let _ = self
            .database
            .record_tool_history(run_id, "spawn_subagent".into(), history_target.into())
            .await;
    }

    /// The only registered tool dispatcher. No frontend-selected effect or arbitrary SQL/path.
    /// Exercised by the kernel test suite; the agent loop uses execute_model_tool.
    #[allow(dead_code)]
    pub(crate) async fn dispatch_tool(        &self,
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
        let root = {
            let state = self.state.lock().await;
            state.control.as_ref().and_then(|control| control.workspace_root.clone())
        };
        let text = self.router.execute(run.session_id, tool, "{}", root.as_deref(), None).await?.text;
        self.database
            .record_tool(run_id, tool.into(), "read-only".into(), true)
            .await?;
        let _ = self
            .database
            .record_tool_history(run_id, tool.into(), ToolRouter::history_target(tool, "{}"))
            .await;
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
    /// `root` is the run-bound workspace (agents/skills scan source).
    pub async fn agent_tools(&self, mode: Mode, root: Option<&std::path::Path>) -> Vec<ToolSpec> {
        self.router.tool_specs(mode, root).await
    }

    /// The tool's resolved effect (MCP tools consult the server readOnlyHint).
    /// Subagent tool lists filter to `ReadOnly` so children never see
    /// approvable side-effect tools like Plan-mode `run_command`.
    pub(crate) async fn tool_effect(&self, tool: &str) -> AppResult<ToolEffect> {
        self.router.classify(tool).await
    }

    /// The workspace root bound to `run_id` at start (None when no run is
    /// active or the run never had one).
    pub(crate) async fn run_workspace_root(&self, run_id: i64) -> Option<PathBuf> {
        let state = self.state.lock().await;
        state
            .active_run
            .as_ref()
            .filter(|run| run.id == run_id)
            .and_then(|_| state.control.as_ref())
            .and_then(|control| control.workspace_root.clone())
    }

    async fn block_tool_scoped(
        &self,
        run_id: i64,
        subagent_id: Option<&str>,
        tool: &str,
        reason: String,
    ) -> AppResult<ToolPayload> {
        if subagent_id.is_none() {
            let _ = self
                .database
                .record_tool(run_id, tool.into(), reason.clone(), false)
                .await;
        }
        self.emit_tool_scoped(run_id, subagent_id, tool, ToolPhase::Blocked, &reason)
            .await;
        Err(reason)
    }

    /// Resolve a suspended tool call: `true` executes it, `false` rejects it.
    /// A run holds at most one pending approval; unknown/stale calls error.
    pub async fn resolve_approval(
        &self,
        run_id: i64,
        decision: ApprovalDecision,
        command_scope: CommandScope,
        feedback: Option<String>,
    ) -> AppResult<()> {
        let _command = self.commands.lock().await;
        // Release the state guard before any DB await: `cancel()` takes the
        // same lock and must never queue behind the persistence worker.
        let (pending, new_rules) = {
            let mut state = self.state.lock().await;
            Self::active_run_locked(&state, run_id)?;
            let control = state.control.as_mut().ok_or("没有等待批准的工具调用")?;
            let pending = control
                .pending
                .take()
                .ok_or("没有等待批准的工具调用")?;
            // 批/会话级决定在当前挂起调用之外记账：批决定写入 control，
            // 会话放行写入 Kernel 级内存集，二者都不需要额外等待。
            match decision {
                ApprovalDecision::AllowBatch => {
                    control.bulk_decision = Some(BulkDecision {
                        approved: true,
                        feedback: feedback.clone(),
                    });
                }
                ApprovalDecision::DenyBatch => {
                    control.bulk_decision = Some(BulkDecision {
                        approved: false,
                        feedback: feedback.clone(),
                    });
                }
                ApprovalDecision::AllowSession => {
                    self.full_access_sessions
                        .lock()
                        .unwrap()
                        .insert(pending.session_id);
                }
                _ => {}
            }
            let mut new_rules = Vec::new();
            if decision == ApprovalDecision::AllowAlways {
                let mut rules = self.approval_rules.lock().unwrap();
                let entry = rules.entry(pending.session_id).or_default();
                for candidate in &pending.candidates {
                    let Some(rule) = ApprovalRule::from_candidate(candidate, command_scope) else { continue };
                    if !entry.contains(&rule) {
                        entry.push(rule.clone());
                        new_rules.push(rule);
                    }
                }
            }
            (pending, new_rules)
        };
        // Persist before resolving so a crash can never grant without
        // recording. Rules live per project (not in `.maestro/` on disk) —
        // a cloned repo must never ship its own pre-approvals.
        if !new_rules.is_empty() {
            if let Ok(Some(project_id)) = self.database.project_id_for_session(pending.session_id).await {
                for rule in &new_rules {
                    let _ = self.database.save_approval_rule(project_id, rule).await;
                }
            }
        }
        let _ = pending.responder.send(ApprovalResponse { decision, feedback });
        Ok(())
    }

    /// Ask the user a structured question and wait for the answer. The UI
    /// opens a modal from the `UserQuestion` event; `answer_user_question`
    /// resolves it. Cancellation resolves the wait as an error.
    pub async fn ask_user(
        &self,
        run_id: i64,
        arguments_json: &str,
        mut cancel: watch::Receiver<bool>,
    ) -> AppResult<String> {
        let arguments: serde_json::Value =
            serde_json::from_str(arguments_json).unwrap_or_else(|_| serde_json::json!({}));
        let questions = parse_questions(&arguments)?;
        let request_id = format!("q-{run_id}-{}", QUESTION_SEQ.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        {
            let mut state = self.state.lock().await;
            Self::active_run_locked(&state, run_id)?;
            let control = state.control.as_mut().ok_or("Run has no control channel")?;
            control.pending_question = Some(PendingQuestion {
                request_id: request_id.clone(),
                questions: questions.clone(),
                responder: tx,
            });
            self.emit_locked(
                &mut state,
                EventData::UserQuestion {
                    run_id,
                    request_id: request_id.clone(),
                    questions: questions.clone(),
                },
            );
        }
        self.emit_tool(run_id, "ask_user", ToolPhase::Started, &format!("{} 个问题", questions.len()))
            .await;
        let answer = tokio::select! {
            biased;
            result = rx => result.ok().flatten(),
            _ = cancel.changed() => None,
        };
        {
            let mut state = self.state.lock().await;
            if let Some(control) = state.control.as_mut() {
                if control
                    .pending_question
                    .as_ref()
                    .is_some_and(|pending| pending.request_id == request_id)
                {
                    control.pending_question = None;
                }
            }
        }
        let Some(answers) = answer else {
            self.emit_tool(run_id, "ask_user", ToolPhase::Failed, "用户取消了提问")
                .await;
            return Err("用户取消了提问".into());
        };
        let result = format_answers(&questions, &answers);
        let payload = serde_json::json!({
            "requestId": request_id,
            "questions": questions,
            "answers": answers,
        })
        .to_string();
        let payload = payload.chars().take(8192).collect::<String>();
        let _ = self
            .database
            .record_tool(run_id, "ask_user".into(), payload.clone(), true)
            .await;
        let _ = self
            .database
            .record_tool_history(
                run_id,
                "ask_user".into(),
                ToolRouter::history_target("ask_user", arguments_json),
            )
            .await;
        self.emit_tool(run_id, "ask_user", ToolPhase::Finished, &payload)
            .await;
        Ok(result)
    }

    /// Resolve the active `ask_user` suspension with structured answers.
    pub async fn answer_user_question(
        &self,
        run_id: i64,
        request_id: &str,
        answers: Vec<QuestionAnswer>,
    ) -> AppResult<()> {
        let mut state = self.state.lock().await;
        Self::active_run_locked(&state, run_id)?;
        let control = state.control.as_mut().ok_or("Run has no control channel")?;
        let questions = control
            .pending_question
            .as_ref()
            .filter(|pending| pending.request_id == request_id)
            .map(|pending| pending.questions.clone())
            .ok_or("没有等待回答的提问")?;
        validate_answers(&questions, &answers)?;
        let pending = control.pending_question.take().ok_or("没有等待回答的提问")?;
        let _ = pending.responder.send(Some(answers));
        Ok(())
    }

    fn command_arg(arguments_json: &str) -> Option<String> {
        serde_json::from_str::<serde_json::Value>(arguments_json)
            .ok()
            .and_then(|value| {
                value
                    .get("command")
                    .and_then(|command| command.as_str())
                    .map(str::to_string)
            })
    }

    fn approval_candidate(tool: &str, arguments_json: &str) -> ApprovalCandidate {
        let value: serde_json::Value =
            serde_json::from_str(arguments_json).unwrap_or_else(|_| serde_json::json!({}));
        let command = (tool == "run_command")
            .then(|| value.get("command").and_then(|item| item.as_str()).map(str::to_string))
            .flatten();
        let (target, scopable) = if let Some(command) = &command {
            (command.clone(), true)
        } else if tool.starts_with("mcp__") {
            // MCP calls have no single "target" argument: the whole canonical
            // argument set is the identity, so a different argument set never
            // reuses an earlier approval.
            let canonical = canonical_json(&value);
            let bounded = canonical.chars().count() <= MAX_RULE_PATTERN_CHARS;
            (if bounded { canonical } else { String::new() }, bounded)
        } else {
            let path = value.get("path").and_then(|item| item.as_str()).map(str::to_string);
            (
                path.unwrap_or_else(|| ToolRouter::call_target(tool, arguments_json)),
                true,
            )
        };
        ApprovalCandidate { tool: tool.into(), target, command, scopable }
    }

    /// Seed the session's in-memory rules from the persisted project set —
    /// once per session, so later grants propagate to future sessions while
    /// the live view stays stable.
    async fn ensure_rules_loaded(&self, session_id: i64) {
        if self.approval_rules.lock().unwrap().contains_key(&session_id) {
            return;
        }
        let loaded = match self.database.project_id_for_session(session_id).await {
            Ok(Some(project_id)) => self
                .database
                .approval_rules_for_project(project_id)
                .await
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        self.approval_rules.lock().unwrap().entry(session_id).or_insert(loaded);
    }

    fn is_auto_approved(&self, session_id: i64, candidate: &ApprovalCandidate) -> bool {
        self.approval_rules
            .lock()
            .unwrap()
            .get(&session_id)
            .is_some_and(|rules| rules.iter().any(|rule| rule.matches(candidate)))
    }

    /// Session-scoped rules currently in force, for display and revocation.
    /// Addressed by run id so the UI never has to trust a session it inferred.
    /// 会话全部放行以 `tool == "*"` 的合成行出现（内存态，不入库）。
    pub async fn list_approval_rules(&self, run_id: i64) -> AppResult<Vec<ApprovalRule>> {
        let session_id = self.session_for_run(run_id).await?;
        self.ensure_rules_loaded(session_id).await;
        let mut rules = self
            .approval_rules
            .lock()
            .unwrap()
            .get(&session_id)
            .cloned()
            .unwrap_or_default();
        if self.full_access_sessions.lock().unwrap().contains(&session_id) {
            rules.insert(
                0,
                ApprovalRule {
                    tool: SESSION_GRANT_TOOL.into(),
                    pattern: "本会话全部放行".into(),
                    prefix: false,
                },
            );
        }
        Ok(rules)
    }

    pub async fn revoke_approval_rule(&self, run_id: i64, rule: &ApprovalRule) -> AppResult<bool> {
        let session_id = self.session_for_run(run_id).await?;
        // 会话全部放行是内存态而非持久规则，撤销走自己的存储。
        if rule.tool == SESSION_GRANT_TOOL {
            return Ok(self
                .full_access_sessions
                .lock()
                .unwrap()
                .remove(&session_id));
        }
        self.ensure_rules_loaded(session_id).await;
        let removed = {
            let mut rules = self.approval_rules.lock().unwrap();
            let Some(entry) = rules.get_mut(&session_id) else { return Ok(false) };
            let before = entry.len();
            entry.retain(|item| item != rule);
            entry.len() != before
        };
        if removed {
            if let Ok(Some(project_id)) = self.database.project_id_for_session(session_id).await {
                let _ = self.database.delete_approval_rule(project_id, rule).await;
            }
        }
        Ok(removed)
    }

    async fn session_for_run(&self, run_id: i64) -> AppResult<i64> {
        if let Some(session_id) = self
            .state
            .lock()
            .await
            .active_run
            .as_ref()
            .filter(|run| run.id == run_id)
            .map(|run| run.session_id)
        {
            return Ok(session_id);
        }
        self.database
            .list_runs()
            .await?
            .into_iter()
            .find(|run| run.id == run_id)
            .map(|run| run.session_id)
            .ok_or_else(|| "运行记录不存在".to_string())
    }

    /// Execute one model-requested tool call: classify, apply the mode gate
    /// (deny / suspend-for-approval / allow), route, audit, and return bounded
    /// text for the next conversation turn. The approval wait never holds the
    /// commands lock, so cancel/resolve/shutdown can always interpose.
    #[cfg(test)]
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
            .map(|(text, _)| text)
    }

    /// Execute a batch of model-requested calls that share ONE approval
    /// decision: every suspended call emits its own Started + AwaitingApproval
    /// event (each with its own preview), but the batch registers a single
    /// responder — one Approve executes all, one Reject blocks all.
    pub(crate) async fn execute_tool_calls(
        &self,
        run_id: i64,
        calls: &[(String, String)],
    ) -> Vec<AppResult<ToolPayload>> {
        self.execute_tool_calls_scoped(run_id, calls, None, None).await
    }

    /// Same batch semantics, but the caller may pin the authorization mode.
    /// Subagents pass `Mode::Plan` so a hallucinated write/command is denied
    /// instead of inheriting the parent run's mode.
    #[cfg(test)]
    pub(crate) async fn execute_tool_calls_as(
        &self,
        run_id: i64,
        calls: &[(String, String)],
        mode: Option<Mode>,
    ) -> Vec<AppResult<ToolPayload>> {
        self.execute_tool_calls_scoped(run_id, calls, mode, None).await
    }

    /// Child-scoped batch: tool lifecycle goes to `SubagentTool`, never to the
    /// parent timeline, and the child trace owns the persisted record.
    pub(crate) async fn execute_subagent_tool_calls(
        &self,
        run_id: i64,
        subagent_id: &str,
        calls: &[(String, String)],
        mode: Option<Mode>,
    ) -> Vec<AppResult<ToolPayload>> {
        self.execute_tool_calls_scoped(run_id, calls, mode, Some(subagent_id))
            .await
    }

    async fn execute_tool_calls_scoped(
        &self,
        run_id: i64,
        calls: &[(String, String)],
        mode: Option<Mode>,
        subagent_id: Option<&str>,
    ) -> Vec<AppResult<ToolPayload>> {
        enum Gate {
            Denied(AppResult<ToolPayload>),
            Allow(ToolEffect),
            Suspended(ToolEffect, ApprovalCandidate),
        }
        let (run, run_root, run_cancel) = {
            let _command = self.commands.lock().await;
            let mut state = self.state.lock().await;
            match Self::active_run_locked(&state, run_id) {
                Ok(run) => {
                    // 批边界：上一批的批量决定不外泄。
                    if let Some(control) = state.control.as_mut() {
                        control.bulk_decision = None;
                    }
                    (
                        run,
                        state
                            .control
                            .as_ref()
                            .and_then(|control| control.workspace_root.clone()),
                        state.control.as_ref().map(|control| control.cancel.clone()),
                    )
                }
                Err(reason) => return calls.iter().map(|_| Err(reason.clone())).collect(),
            }
        };
        let mode = mode.unwrap_or(run.mode);
        // Phase 1: classify + gate every call before any execution so the
        // batch knows which calls suspend. Each suspended call carries its own
        // approval candidate — decisions are resolved per call, never shared.
        let mut gates = Vec::with_capacity(calls.len());
        let mut audit_kinds: Vec<AuditKind> = Vec::with_capacity(calls.len());
        let mut any_suspended = false;
        // 会话全部放行只在非 Plan 模式生效，且不能越过模式的绝对禁止
        // （它豁免的是 RequireApproval，Plan 的 Deny 在此之前就已分流）。
        let full_access = mode != Mode::Plan
            && self
                .full_access_sessions
                .lock()
                .unwrap()
                .contains(&run.session_id);
        self.ensure_rules_loaded(run.session_id).await;
        // 架构沙盘：存在生效规则（或规则文件损坏）时载入本会话工具历史；
        // Requirement/Extension 满足度据此判定，跨 Run 持久化于 audit 表。
        let has_rules = self.router.has_architecture_rules().await;
        let mut history: Vec<HistoryEntry> = if has_rules {
            self.database
                .session_tool_history(run.session_id)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        for (index, (tool, arguments_json)) in calls.iter().enumerate() {
            let target = ToolRouter::call_target(tool, arguments_json);
            self.emit_tool_scoped(run_id, subagent_id, tool, ToolPhase::Started, &target).await;
            let candidate = Self::approval_candidate(tool, arguments_json);
            let command = Self::command_arg(arguments_json);
            audit_kinds.push(if tool == "run_command"
                && command.as_deref().is_some_and(command_policy::is_read_only_safe)
            {
                AuditKind::ReadOnlyAllowlist
            } else if tool == "run_command" {
                AuditKind::Autonomous
            } else {
                AuditKind::Extension
            });
            // A historical "always allow" rule or a session grant only waives
            // an approval that the current mode would otherwise demand. It can
            // never lift a mode's absolute denial (Plan), and in Plan every
            // side-effect call must ask explicitly — rules are ignored there
            // too (会话放行对 Plan 早已无效，见上方 full_access 排除)。
            let auto =
                mode != Mode::Plan && self.is_auto_approved(run.session_id, &candidate);
            // Plan 下 background:true 走 shell 路径，按非只读拒绝：command 置
            // 空后 gate_call 的只读白名单判定失败，自然落回 Deny。
            let command = if mode == Mode::Plan
                && tool == "run_command"
                && serde_json::from_str::<serde_json::Value>(arguments_json)
                    .ok()
                    .and_then(|value| value.get("background").and_then(|v| v.as_bool()))
                    .unwrap_or(false)
            {
                None
            } else {
                command
            };
            let mut gate = match self.router.classify(tool).await {
                Err(reason) => Gate::Denied(self.block_tool_scoped(run_id, subagent_id, tool, reason).await),
                Ok(effect) => {
                    // 子智能体保持只读硬门槛：可审批在其作用域内一律降级为
                    // 拒绝——它的契约是"不执行命令"，审批卡只能由主对话发起。
                    let tool_gate = match policy::gate_call(mode, tool, effect, command.as_deref()) {
                        ToolGate::RequireApproval if subagent_id.is_some() => ToolGate::Deny,
                        result => result,
                    };
                    match tool_gate {
                        ToolGate::Deny => Gate::Denied(
                            self.block_tool_scoped(
                                run_id,
                                subagent_id,
                                tool,
                                format!(
                                    "Plan Mode blocked tool: {tool}; only read tools and allowlisted read-only commands are allowed"
                                ),
                            )
                            .await,
                        ),
                        ToolGate::RequireApproval if auto || full_access => {
                            audit_kinds[index] = if auto {
                                AuditKind::RuleApproved
                            } else {
                                AuditKind::SessionGranted
                            };
                            Gate::Allow(effect)
                        }
                        ToolGate::RequireApproval => {
                            any_suspended = true;
                            Gate::Suspended(effect, candidate)
                        }
                        ToolGate::Allow => Gate::Allow(effect),
                    }
                }
            };
            // 架构预检（审批前）：本会话历史 + 本批次此前调用的乐观快照。
            // 命中阻断即在进入审批前拦下；是否可放行仍以 Phase 3 真实历史为准。
            if has_rules {
                let effect = match &gate {
                    Gate::Allow(effect) | Gate::Suspended(effect, _) => Some(*effect),
                    Gate::Denied(_) => None,
                };
                if let Some(effect) = effect {
                    let mut optimistic = history.clone();
                    for (prior_tool, prior_args) in calls.iter().take(index) {
                        optimistic.push(HistoryEntry {
                            tool: prior_tool.clone(),
                            target: ToolRouter::history_target(prior_tool, prior_args),
                            run_id,
                        });
                    }
                    let decision = self
                        .router
                        .architecture_decision(
                            tool,
                            arguments_json,
                            effect,
                            &optimistic,
                            run_id,
                            run_root.as_deref(),
                            run.prompt.as_str(),
                        )
                        .await;
                    self.emit_architecture_trace(run_id, run.session_id, subagent_id, tool, &target, &decision)
                        .await;
                    if decision.blocked {
                        gate = Gate::Denied(
                            self.block_tool_scoped(run_id, subagent_id, tool, Self::architecture_reason(&decision))
                                .await,
                        );
                    }
                }
            }
            gates.push(gate);
        }
        // Pure read-only batches run concurrently — 仅当没有架构规则时：
        // 读工具同样可能被 Trigger(read) 命中，且历史顺序必须串行记录。
        if !any_suspended && !has_rules && !calls.is_empty()
            && gates.iter().all(|gate| matches!(gate, Gate::Allow(ToolEffect::ReadOnly)))
        {
            let outcomes = futures_util::future::join_all(
                calls.iter().map(|(tool, arguments_json)| {
                    self.router.execute(
                        run.session_id,
                        tool,
                        arguments_json,
                        run_root.as_deref(),
                        run_cancel.as_ref().map(|cancel| cancel.subscribe()),
                    )
                }),
            )
            .await;
            let mut results = Vec::with_capacity(calls.len());
            for ((tool, arguments_json), outcome) in calls.iter().zip(outcomes) {
                let target = ToolRouter::call_target(tool, arguments_json);
                results.push(
                    self.finish_tool_outcome(run_id, run.session_id, subagent_id, tool, arguments_json, &target, "extension", ToolEffect::ReadOnly, outcome)
                        .await,
                );
            }
            return results;
        }

        // Phase 2: execute calls in order. Every suspended call gets its own
        // preview, its own wait, and its own decision — a single approval can
        // no longer smuggle a batch of mixed side effects through (ZCode
        // per-call approval baseline).
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
                Gate::Denied(result) => results.push(std::mem::replace(result, Ok((String::new(), Vec::new())))),
                Gate::Suspended(effect, candidate) => {
                    // 本批已有批量决定：不再弹卡，直接套用（clone 而非 take —
                    // 同一决定对批内所有后续挂起生效，批入口负责清空）。
                    let bulk = {
                        let state = self.state.lock().await;
                        state
                            .control
                            .as_ref()
                            .and_then(|control| control.bulk_decision.clone())
                    };
                    if let Some(bulk) = bulk {
                        if !bulk.approved {
                            let reason = bulk
                                .feedback
                                .as_deref()
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .map(|value| format!("用户拒绝了该操作：{value}"))
                                .unwrap_or_else(|| "用户拒绝了该操作".into());
                            results.push(self.block_tool_scoped(run_id, subagent_id, tool, reason).await);
                            continue;
                        }
                        let target = ToolRouter::call_target(tool, arguments_json);
                        let (blocked, reminder) = self
                            .architecture_gate(run_id, run.session_id, subagent_id, tool, arguments_json, &target, *effect, &history, has_rules, run_root.as_deref(), run.prompt.as_str())
                            .await;
                        if let Some(blocked) = blocked {
                            results.push(blocked);
                            continue;
                        }
                        let mut result = self
                            .finish_tool(run_id, run.session_id, subagent_id, tool, arguments_json, &target, "batch approved", *effect)
                            .await;
                        if let (Ok((text, _)), Some(reminder)) = (&mut result, &reminder) {
                            text.push_str("\n\n");
                            text.push_str(reminder);
                        }
                        if result.is_ok() {
                            history.push(HistoryEntry {
                                tool: tool.clone(),
                                target: ToolRouter::history_target(tool, arguments_json),
                                run_id,
                            });
                        }
                        results.push(result);
                        continue;
                    }
                    // Snapshot the write target right before the card is
                    // shown, so an edit made while the user decides is
                    // detected instead of overwritten.
                    let baseline = self
                        .router
                        .write_baseline(run_root.as_deref(), tool, arguments_json)
                        .await;
                    let preview = self
                        .router
                        .approval_preview(run_root.as_deref(), tool, arguments_json)
                        .await;
                    self.emit_tool_scoped(run_id, subagent_id, tool, ToolPhase::AwaitingApproval, &preview)
                        .await;
                    let (approved, feedback) = self
                        .wait_for_approval(run_id, run.session_id, candidate.clone())
                        .await;
                    if !approved {
                        let reason = feedback
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(|value| format!("用户拒绝了该操作：{value}"))
                            .unwrap_or_else(|| "用户拒绝了该操作".into());
                        results.push(self.block_tool_scoped(run_id, subagent_id, tool, reason).await);
                        continue;
                    }
                    // An approved write must still match what the user saw.
                    if let Err(conflict) = self
                        .router
                        .verify_write_baseline(run_root.as_deref(), tool, arguments_json, &baseline)
                        .await
                    {
                        results.push(self.block_tool_scoped(run_id, subagent_id, tool, conflict).await);
                        continue;
                    }
                    let target = ToolRouter::call_target(tool, arguments_json);
                    let (blocked, reminder) = self
                        .architecture_gate(run_id, run.session_id, subagent_id, tool, arguments_json, &target, *effect, &history, has_rules, run_root.as_deref(), run.prompt.as_str())
                        .await;
                    if let Some(blocked) = blocked {
                        results.push(blocked);
                        continue;
                    }
                    let mut result = self
                        .finish_tool(run_id, run.session_id, subagent_id, tool, arguments_json, &target, "user approved", *effect)
                        .await;
                    if let (Ok((text, _)), Some(reminder)) = (&mut result, &reminder) {
                        // 放行+附言：提醒跟工具结果一起回到模型上下文。
                        text.push_str("\n\n");
                        text.push_str(reminder);
                    }
                    if result.is_ok() {
                        history.push(HistoryEntry {
                            tool: tool.clone(),
                            target: ToolRouter::history_target(tool, arguments_json),
                            run_id,
                        });
                    }
                    results.push(result);
                }
                Gate::Allow(effect) => {
                    let audit = Self::audit_label(audit_kinds[index]);
                    let target = ToolRouter::call_target(tool, arguments_json);
                    let (blocked, reminder) = self
                        .architecture_gate(run_id, run.session_id, subagent_id, tool, arguments_json, &target, *effect, &history, has_rules, run_root.as_deref(), run.prompt.as_str())
                        .await;
                    if let Some(blocked) = blocked {
                        results.push(blocked);
                        continue;
                    }
                    let mut result = self
                        .finish_tool(run_id, run.session_id, subagent_id, tool, arguments_json, &target, audit, *effect)
                        .await;
                    if let (Ok((text, _)), Some(reminder)) = (&mut result, &reminder) {
                        text.push_str("\n\n");
                        text.push_str(reminder);
                    }
                    if result.is_ok() {
                        history.push(HistoryEntry {
                            tool: tool.clone(),
                            target: ToolRouter::history_target(tool, arguments_json),
                            run_id,
                        });
                    }
                    results.push(result);
                }
            }
        }
        results
    }

    /// Phase 3 架构权威检查（执行前最后一次）：真实历史上的完整求值，
    /// 命中时发出 trace 事件；返回 (Some(Err), _) 表示拦截结果，
    /// 放行时可能带回 reminder（Remind 动作 / 欠账自动提醒），由调用点
    /// 拼进工具结果随结果回到模型上下文。
    #[allow(clippy::too_many_arguments)]
    async fn architecture_gate(
        &self,
        run_id: i64,
        session_id: i64,
        subagent_id: Option<&str>,
        tool: &str,
        arguments_json: &str,
        target: &str,
        effect: ToolEffect,
        history: &[HistoryEntry],
        has_rules: bool,
        run_root: Option<&std::path::Path>,
        user_message: &str,
    ) -> (Option<AppResult<ToolPayload>>, Option<String>) {
        if !has_rules {
            return (None, None);
        }
        let decision = self
            .router
            .architecture_decision(tool, arguments_json, effect, history, run_id, run_root, user_message)
            .await;
        self.emit_architecture_trace(run_id, session_id, subagent_id, tool, target, &decision)
            .await;
        if decision.blocked {
            return (
                Some(
                    self.block_tool_scoped(run_id, subagent_id, tool, Self::architecture_reason(&decision))
                        .await,
                ),
                None,
            );
        }
        (None, decision.reminder)
    }

    /// 特殊通道调用（`ask_user`/`spawn_subagent`——不走 execute_tool_calls）
    /// 的架构检查：读最新会话历史，语义与批内 `architecture_gate` 一致。
    /// `Ok(reminder)` 放行（reminder 附进工具结果）；`Err(reason)` 拦截。
    pub(crate) async fn architecture_check_special(
        &self,
        run_id: i64,
        session_id: i64,
        tool: &str,
        arguments_json: &str,
        user_message: &str,
    ) -> Result<Option<String>, String> {
        if !self.router.has_architecture_rules().await {
            return Ok(None);
        }
        let effect = match self.router.classify(tool).await {
            Ok(effect) => effect,
            Err(_) => return Ok(None),
        };
        let run_root = self.run_workspace_root(run_id).await;
        let history = self
            .database
            .session_tool_history(session_id)
            .await
            .unwrap_or_default();
        let target = ToolRouter::history_target(tool, arguments_json);
        let decision = self
            .router
            .architecture_decision(
                tool,
                arguments_json,
                effect,
                &history,
                run_id,
                run_root.as_deref(),
                user_message,
            )
            .await;
        self.emit_architecture_trace(run_id, session_id, None, tool, &target, &decision)
            .await;
        if decision.blocked {
            return Err(Self::architecture_reason(&decision));
        }
        Ok(decision.reminder)
    }

    /// 命中 Trigger 时向前端发出节点级执行轨迹（仅父 Run；子智能体评估
    /// 照常，但不发事件以免污染父画布）。
    async fn emit_architecture_trace(
        &self,
        run_id: i64,
        session_id: i64,
        subagent_id: Option<&str>,
        tool: &str,
        target: &str,
        decision: &GraphDecision,
    ) {
        if subagent_id.is_some() || !decision.matched {
            return;
        }
        let mut state = self.state.lock().await;
        if Self::active_run_locked(&state, run_id).is_ok() {
            self.emit_locked(
                &mut state,
                EventData::ArchitectureTrace {
                    run_id,
                    session_id,
                    tool: tool.into(),
                    target: target.into(),
                    blocked: decision.blocked,
                    message: decision.message.clone(),
                    steps: decision.trace.clone(),
                },
            );
        }
    }

    /// 注入 agent_loop 请求的架构契约文本；无规则时为空串。
    pub async fn architecture_brief(&self, run_root: Option<&std::path::Path>) -> String {
        self.router.architecture_brief(run_root).await
    }

    /// 沙盘装配：该类型子智能体经 Extension → Subagent 绑定获得的扩展授权。
    pub async fn subagent_grants(
        &self,
        run_root: Option<&std::path::Path>,
        agent: &str,
    ) -> Vec<crate::services::tool_router::SubagentGrant> {
        self.router.subagent_grants(run_root, agent).await
    }

    /// 军团运行时解析：squads[name] → 指挥/成员/各级装配授权。
    /// spawn_subagent 的 type 在 agent 定义未命中时走这里；成员/孙级不查。
    pub async fn squad_runtime(
        &self,
        run_root: Option<&std::path::Path>,
        squad: &str,
    ) -> Option<crate::services::tool_router::ResolvedSquad> {
        self.router.squad_runtime(run_root, squad).await
    }

    /// Run 末欠账查询：会话历史中仍挂着未验收的事后义务（optional 义务也计入——
    /// 它不拦调用但会追问）。非空时向画布发 pending trace 让节点亮琥珀色。
    pub async fn architecture_outstanding(
        &self,
        run_id: i64,
        session_id: i64,
        run_root: Option<&std::path::Path>,
        user_message: &str,
    ) -> Vec<ArmedObligation> {
        self.ensure_rules_loaded(session_id).await;
        if !self.router.has_architecture_rules().await {
            return Vec::new();
        }
        let history = self
            .database
            .session_tool_history(session_id)
            .await
            .unwrap_or_default();
        let outstanding = self
            .router
            .architecture_outstanding(&history, run_id, run_root, user_message)
            .await;
        if !outstanding.is_empty() {
            let decision = GraphDecision {
                matched: true,
                blocked: false,
                warn: false,
                message: Some(
                    outstanding
                        .iter()
                        .map(|item| item.message.clone())
                        .collect::<Vec<_>>()
                        .join("；"),
                ),
                reminder: None,
                trace: outstanding
                    .iter()
                    .map(|item| TraceStep {
                        node_id: item.node_id.clone(),
                        status: TraceStatus::Pending,
                    })
                    .collect(),
            };
            self.emit_architecture_trace(run_id, session_id, None, "__run_end__", "", &decision)
                .await;
        }
        outstanding
    }

    /// 保存规则后刷新运行时缓存（架构沙盘 IPC 调用）。
    pub async fn reload_architecture(&self) {
        self.router.reload_architecture().await;
    }

    /// One call's approval wait: installs the responder on the run's control
    /// channel and returns the decision. `allow_always` rules are recorded by
    /// `resolve_approval` against exactly this call's candidate.
    async fn wait_for_approval(
        &self,
        run_id: i64,
        session_id: i64,
        candidate: ApprovalCandidate,
    ) -> (bool, Option<String>) {
        let (tx, rx) = oneshot::channel();
        let waiting = {
            let mut state = self.state.lock().await;
            if Self::active_run_locked(&state, run_id).is_ok() {
                if let Some(control) = state.control.as_mut() {
                    control.pending = Some(PendingApproval {
                        responder: tx,
                        session_id,
                        candidates: vec![candidate],
                    });
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        if !waiting {
            return (false, None);
        }
        match rx.await {
            Ok(ApprovalResponse { decision, feedback }) => {
                if decision.is_allow() {
                    (true, None)
                } else {
                    (false, feedback)
                }
            }
            Err(_) => (false, None),
        }
    }

/// 架构拦截的模型可读理由：Warn 动作用警告前缀，仍属拦截。
fn architecture_reason(decision: &GraphDecision) -> String {
    let message = decision
        .message
        .clone()
        .unwrap_or_else(|| "架构规则拦截".into());
    if decision.warn {
        format!("【架构警告】{message}")
    } else {
        message
    }
}

fn audit_label(kind: AuditKind) -> &'static str {
    match kind {
        AuditKind::RuleApproved => "auto-approved",
        AuditKind::SessionGranted => "session granted",
        AuditKind::ReadOnlyAllowlist => "read-only allowlist",
        AuditKind::Autonomous => "autonomous",
        AuditKind::Extension => "extension",
    }
}

fn apply_trace_tool(view: &mut SubagentView, tool: &str, phase: ToolPhase, detail: &str) {
    let pending = view
        .tools
        .iter()
        .position(|call| call.tool == tool && call.pending == Some(true));
    let entry = ToolCallView {
        run_id: view.run_id,
        tool: tool.into(),
        detail: detail.into(),
        allowed: !matches!(phase, ToolPhase::Failed | ToolPhase::Blocked),
        blocked: matches!(phase, ToolPhase::Blocked).then_some(true),
        subagent_id: Some(view.id.clone()),
        pending: matches!(phase, ToolPhase::Started | ToolPhase::AwaitingApproval).then_some(true),
        target: (phase == ToolPhase::Started).then(|| detail.to_string()),
    };
    match pending {
        Some(index) => {
            let target = view.tools[index].target.clone().or(entry.target);
            view.tools[index] = ToolCallView { target, ..entry };
        }
        None => view.tools.push(entry),
    }
}

fn format_subagent_reports(traces: &[SubagentView]) -> String {
    let mut body = String::new();
    for trace in traces {
        let summary: String = trace.summary.chars().take(4000).collect();
        if summary.trim().is_empty() {
            continue;
        }
        let indented = summary
            .lines()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let block = format!(
            "- [{}] {} · {} · {} 次工具\n{}\n",
            trace.agent,
            trace.description,
            trace.status.label(),
            trace.tool_count,
            indented
        );
        if body.len() + block.len() > 16_000 {
            break;
        }
        body.push_str(&block);
    }
    if body.is_empty() {
        String::new()
    } else {
        format!("## 本会话此前的子智能体报告（只读证据，不要当作新任务）\n{body}")
    }
}

    /// Execute the routed call, audit the terminal result, emit
    /// Finished/Failed and refresh the workspace on any side effect.
    #[allow(clippy::too_many_arguments)]
    async fn finish_tool(
        &self,
        run_id: i64,
        session_id: i64,
        subagent_id: Option<&str>,
        tool: &str,
        arguments_json: &str,
        target: &str,
        audit: &str,
        effect: ToolEffect,
    ) -> AppResult<ToolPayload> {
        // A running command must stop when the user stops the run, instead of
        // waiting out its timeout: hand the exec adapter the run's cancel signal.
        let (cancel, run_root) = {
            let state = self.state.lock().await;
            let control = state
                .active_run
                .as_ref()
                .filter(|active| active.id == run_id)
                .and_then(|_| state.control.as_ref());
            (
                control.map(|control| control.cancel.subscribe()),
                control.and_then(|control| control.workspace_root.clone()),
            )
        };
        let outcome = self.router.execute(session_id, tool, arguments_json, run_root.as_deref(), cancel).await;
        self.finish_tool_outcome(run_id, session_id, subagent_id, tool, arguments_json, target, audit, effect, outcome)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish_tool_outcome(
        &self,
        run_id: i64,
        session_id: i64,
        subagent_id: Option<&str>,
        tool: &str,
        arguments_json: &str,
        target: &str,
        audit: &str,
        effect: ToolEffect,
        outcome: AppResult<ToolOutcome>,
    ) -> AppResult<ToolPayload> {
        match outcome {
            Ok(outcome) => {
                let images = outcome.images.clone();
                let write_counts = if let Some(report) = &outcome.write {
                    let (added, removed) = line_change_counts(&report.before, &report.after);
                    let _ = self.database.record_file_change(run_id, session_id, report.path.clone(), added, removed, report.before.clone(), report.after.clone()).await;
                    // 累计净差：与 DB merge_file_changes 同口径（首 before → 末 after），
                    // 含子代理写入（同一 run_id）；before/after 截断后这是唯一权威计数。
                    let net = {
                        let mut map = self.write_net.lock().unwrap();
                        let entry = map
                            .entry(run_id)
                            .or_default()
                            .entry(report.path.clone())
                            .or_insert_with(|| (report.before.clone(), report.after.clone()));
                        entry.1 = report.after.clone();
                        line_change_counts(&entry.0, &entry.1)
                    };
                    Some((added, removed, net.0, net.1))
                } else {
                    None
                };
                let detail = if target.is_empty() {
                    audit.to_string()
                } else {
                    format!("{audit} · {target}")
                };
                if subagent_id.is_none() {
                    self.database
                        .record_tool(run_id, tool.into(), detail, true)
                        .await?;
                    // 架构沙盘 Session 历史：跨 Run 判定 Requirement/扩展满足度。
                    let _ = self
                        .database
                        .record_tool_history(
                            run_id,
                            tool.into(),
                            ToolRouter::history_target(tool, arguments_json),
                        )
                        .await;
                }
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
                let event_detail = match &outcome.write {
                    Some(report) => serde_json::json!({
                        "kind": "write_result",
                        "path": report.path,
                        "before": report.before.chars().take(8192).collect::<String>(),
                        "after": report.after.chars().take(8192).collect::<String>(),
                        // before/after are display-bounded — ship the full-content
                        // counts so panels never recompute on truncated text.
                        "added": write_counts.map(|counts| counts.0).unwrap_or_default(),
                        "removed": write_counts.map(|counts| counts.1).unwrap_or_default(),
                        // 同文件多次写入的累计净差（首 before → 末 after），与变更卡口径一致。
                        "net_added": write_counts.map(|counts| counts.2).unwrap_or_default(),
                        "net_removed": write_counts.map(|counts| counts.3).unwrap_or_default(),
                    })
                    .to_string(),
                    None => outcome.text.chars().take(2048).collect::<String>(),
                };
                self.emit_tool_scoped(run_id, subagent_id, tool, ToolPhase::Finished, &event_detail)
                    .await;
                if effect != ToolEffect::ReadOnly {
                    let mut state = self.state.lock().await;
                    self.emit_locked(&mut state, EventData::WorkspaceChanged);
                }
                Ok((outcome.text, images))
            }
            Err(reason) => {
                if subagent_id.is_none() {
                    let _ = self
                        .database
                        .record_tool(run_id, tool.into(), reason.clone(), false)
                        .await;
                }
                self.emit_tool_scoped(run_id, subagent_id, tool, ToolPhase::Failed, &reason)
                    .await;
                Err(reason)
            }
        }
    }

    pub async fn list_agents(&self, scope: String) -> AppResult<Vec<crate::domain::AgentDefinition>> {
        let workspace = self.router.workspace_root().await;
        // "all" is the runtime/spawn view — untrusted project agents stay out.
        // Explicit scopes keep the raw root so the UI can still inspect or
        // delete a hostile definition from an untrusted checkout.
        let workspace = if scope == "all" {
            self.router.trusted_project_root(workspace.as_deref()).await
        } else {
            workspace
        };
        Ok(crate::adapters::agents::list(self.router.extensions().maestro_dir(), workspace.as_deref(), &scope))
    }

    pub async fn save_agent(&self, scope: String, agent: crate::domain::AgentDefinition) -> AppResult<()> {
        let workspace = self.router.workspace_root().await;
        crate::adapters::agents::save(self.router.extensions().maestro_dir(), workspace.as_deref(), &scope, &agent)
    }

    pub async fn delete_agent(&self, scope: String, name: String) -> AppResult<()> {
        let workspace = self.router.workspace_root().await;
        crate::adapters::agents::delete(self.router.extensions().maestro_dir(), workspace.as_deref(), &scope, &name)
    }

    /// 批量启用/禁用（hidden 即禁用语义）；逐项三态，单项失败不阻断其余。
    /// set_hidden 按 effective() 的 def.source 定位实际文件（item.scope 仅作卡片身份）。
    pub async fn set_agents_hidden(&self, items: Vec<crate::domain::AgentItemRef>, hidden: bool) -> AppResult<Vec<crate::domain::AgentImportOutcome>> {
        let workspace = self.router.workspace_root().await;
        Ok(crate::adapters::agents::set_hidden(self.router.extensions().maestro_dir(), workspace.as_deref(), &items, hidden))
    }

    /// 批量删除：逐项三态；builtin 记 failed。
    pub async fn delete_agents(&self, items: Vec<crate::domain::AgentItemRef>) -> AppResult<Vec<crate::domain::AgentImportOutcome>> {
        let workspace = self.router.workspace_root().await;
        Ok(crate::adapters::agents::delete_many(self.router.extensions().maestro_dir(), workspace.as_deref(), &items))
    }

    /// 批量导入本地目录 Agent ——写入全局 ~/.maestro/agents，不经工作区 scope。
    pub async fn import_agents_dir(&self, path: String) -> AppResult<Vec<crate::domain::AgentImportOutcome>> {
        crate::adapters::agents::import_from_dir(self.router.extensions().maestro_dir(), &path)
    }

    /// 浅克隆 GitHub 仓库后批量导入 Agent（agency-agents-zh 类格式转换）。
    pub async fn import_agents_github(&self, repo_url: String) -> AppResult<Vec<crate::domain::AgentImportOutcome>> {
        crate::adapters::agents::import_from_github(self.router.extensions().maestro_dir(), &repo_url).await
    }

    /// 全局子智能体默认（~/.maestro/agents.json）；每次 spawn 读取——文件仅数百字节。
    pub fn agent_defaults(&self) -> AppResult<crate::domain::SubagentDefaults> {
        crate::adapters::agents::load_defaults(self.router.extensions().maestro_dir())
    }

    pub async fn save_agent_defaults(&self, defaults: crate::domain::SubagentDefaults) -> AppResult<()> {
        crate::adapters::agents::save_defaults(self.router.extensions().maestro_dir(), &defaults)
    }

    pub async fn run_change_summary(&self, run_id: i64) -> AppResult<crate::domain::RunChangeSummary> {
        // Keep DB-side files/state, but report net additions/deletions over the
        // merged per-path diff (first before → last after) so the summary card
        // matches the diff the review panel actually renders.
        let mut summary = self.database.run_change_summary(run_id).await?;
        let merged = merge_file_changes(self.database.run_changes(run_id).await?);
        summary.additions = merged.iter().map(|change| change.added).sum();
        summary.deletions = merged.iter().map(|change| change.removed).sum();
        summary.files = merged.len() as i64;
        Ok(summary)
    }

    pub async fn run_changes(&self, run_id: i64) -> AppResult<Vec<crate::domain::RunFileChange>> {
        Ok(merge_file_changes(self.database.run_changes(run_id).await?))
    }

    /// Revert writes must target the directory the run actually used — resolve
    /// the project path bound to the run's session, not the live Explorer
    /// root (the user may have switched directories since).
    async fn workspace_root_for_session(&self, session_id: i64) -> Option<PathBuf> {
        match self.database.project_path_for_session(session_id).await {
            Ok(Some(path)) if !path.trim().is_empty() => Some(PathBuf::from(path)),
            _ => self.router.workspace_root().await,
        }
    }

    pub async fn preview_revert_run_changes(&self, run_id: i64) -> AppResult<crate::domain::RevertPreview> {
        let changes = self.database.run_changes(run_id).await?;
        let session_id = self
            .database
            .list_runs()
            .await?
            .into_iter()
            .find(|run| run.id == run_id)
            .map(|run| run.session_id);
        let root = match session_id {
            Some(id) => self.workspace_root_for_session(id).await,
            None => self.router.workspace_root().await,
        };
        self.router.preview_revert_changes(root.as_deref(), &changes).await
    }

    pub async fn revert_run_changes(&self, run_id: i64, session_id: i64) -> AppResult<crate::domain::RevertResult> {
        let changes = self.database.run_changes(run_id).await?;
        let root = self.workspace_root_for_session(session_id).await;
        let result = self.router.revert_changes(root.as_deref(), &changes).await?;
        let _ = self.database.mark_run_changes_reverted(run_id, &result.reverted).await;
        let _ = self.database.record_tool(run_id, "revert_run_changes".into(), format!("reverted {} files", result.reverted.len()), true).await;
        let mut state = self.state.lock().await;
        self.emit_locked(&mut state, EventData::WorkspaceChanged);
        Ok(result)
    }

    pub async fn activity_snapshot(&self, project_id: Option<i64>) -> AppResult<crate::domain::ActivitySnapshot> {
        self.database.activity_snapshot(project_id).await
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
        let project_id = match project_id {
            Some(id) => id,
            None => self
                .database
                .first_active_project_id()
                .await?
                .ok_or_else(|| "请先新建项目".to_string())?,
        };
        let session = self
            .database
            .create_session_in_project(project_id)
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
        // Approving the plan is the user's sanction to execute: leave the
        // read-only Plan mode behind (ZCode plan→code handoff). Rejected or
        // revised plans keep the current mode.
        // Plans are decided after their run completed (decide_plan requires a
        // completed run), so approval escalates the session mode for the next
        // run rather than the finished one.
        if matches!(decision, PlanDecision::Approve) && self.state.lock().await.mode == Mode::Plan {
            self.database.record_mode(Mode::Code).await?;
            let mut state = self.state.lock().await;
            state.mode = Mode::Code;
            self.emit_locked(&mut state, EventData::ModeChanged { mode: Mode::Code });
        }
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
        if state.reasoning_run != Some(run_id) {
            state.reasoning.clear();
            state.reasoning_run = Some(run_id);
        }
        match state.reasoning.last_mut() {
            Some(segment) if segment.ended_at.is_none() => segment.text.push_str(&token),
            _ => state.reasoning.push(ReasoningSegment {
                text: token.clone(),
                started_at: now_ms(),
                ended_at: None,
            }),
        }
        match state.parts.last_mut() {
            Some(part) if part.kind == "reasoning" && part.ended_at.is_none() => part.text.push_str(&token),
            _ => state.parts.push(RunPart {
                kind: "reasoning".into(),
                text: token.clone(),
                index: None,
                started_at: now_ms(),
                ended_at: None,
            }),
        }
        self.emit_locked(
            &mut state,
            EventData::Reasoning { run_id, token },
        );
    }

    /// 空轮取证：模型以零正文零调用完成一轮时落 `turn_empty` 审计行，
    /// 携带 finish_reason 与丢弃碎片数——下次同类问题可直接查证。
    pub(crate) async fn audit_turn_anomaly(&self, run_id: i64, detail: String) {
        let _ = self
            .database
            .record_anomaly(run_id, "turn_empty".into(), detail)
            .await;
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
        if let Some(segment) = state.reasoning.last_mut() {
            if segment.ended_at.is_none() {
                segment.ended_at = Some(now_ms());
            }
        }
        if let Some(part) = state.parts.last_mut() {
            if part.kind == "reasoning" && part.ended_at.is_none() {
                part.ended_at = Some(now_ms());
            }
        }
        match state.parts.last_mut() {
            Some(part) if part.kind == "content" => part.text.push_str(&token),
            _ => state.parts.push(RunPart {
                kind: "content".into(),
                text: token.clone(),
                index: None,
                started_at: now_ms(),
                ended_at: None,
            }),
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

    /// 由累加器重建累计视图（DSH sessionStats 口径）：last_* 是最近一次
    /// 完成请求的快照，成对的 decode/ttft 字段只计完成请求。
    fn metrics_view(acc: &MetricsAccumulator) -> RunMetrics {
        RunMetrics {
            run_id: acc.run_id,
            session_id: acc.session_id,
            provider: acc.provider.clone(),
            model: acc.model.clone(),
            input_tokens: acc.input,
            output_tokens: acc.output,
            cache_hit_tokens: acc.cache,
            cache_write_tokens: acc.cache_write,
            reasoning_tokens: acc.reasoning,
            attempts: acc.attempts,
            last_input_tokens: acc.last_known_input,
            last_output_tokens: acc.last_output,
            last_elapsed_ms: acc.last_elapsed,
            last_first_token_ms: acc.last_first,
            elapsed_ms: acc.elapsed,
            first_token_ms: Some(acc.first),
            ttft_steps: acc.ttft_steps,
            decode_ms: acc.decode_ms,
            decode_tokens: acc.decode_tokens,
            tool_ms: acc.tool_ms,
            live_surface_tokens: None,
        }
    }

    /// Streaming metrics updates reach the UI as events; only the terminal
    /// report is durable in SQLite.
    pub(crate) async fn report_metrics(&self, metrics: RunMetrics, persist: bool) -> AppResult<()> {
        self.report_metrics_inner(metrics, persist, true).await
    }

    /// 子智能体调用的 usage 折进父 run 累计桶，但它的 prompt 属于子会话
    /// 自己的上下文窗口，不能覆盖父会话的 context pressure（ZCode
    /// `shouldUseModelUsageForContext`：只有主会话请求的 inputTokens 才
    /// 代表可见上下文）——否则子调用的小 prompt 会把上下文环打成 0%。
    pub(crate) async fn report_subagent_metrics(&self, metrics: RunMetrics, persist: bool) -> AppResult<()> {
        self.report_metrics_inner(metrics, persist, false).await
    }

    async fn report_metrics_inner(&self, metrics: RunMetrics, persist: bool, main_conversation: bool) -> AppResult<()> {
        let cumulative = {
            let mut map = self.metrics_acc.lock().unwrap();
            let acc = map.entry(metrics.run_id).or_default();
            acc.run_id = metrics.run_id;
            acc.session_id = metrics.session_id;
            acc.provider = metrics.provider.clone();
            acc.model = metrics.model.clone();
            let last_known_input = if main_conversation {
                metrics.input_tokens.or(acc.last_known_input)
            } else {
                acc.last_known_input
            };
            acc.last_known_input = last_known_input;
            if persist {
                acc.input = sum_tokens(acc.input, metrics.input_tokens);
                acc.output = sum_tokens(acc.output, metrics.output_tokens);
                acc.cache = sum_tokens(acc.cache, metrics.cache_hit_tokens);
                acc.cache_write = sum_tokens(acc.cache_write, metrics.cache_write_tokens);
                acc.reasoning = sum_tokens(acc.reasoning, metrics.reasoning_tokens);
                acc.attempts += 1;
                acc.elapsed += metrics.elapsed_ms;
                // DSH sessionStats fold：TTFT 只覆盖记录了首 delta 的请求；
                // decode 窗口只在「首 token 与 output usage 同有」时成对累计，
                // 与 outputTokens 一一对应，不做文本估算。
                if let Some(first) = metrics.first_token_ms {
                    acc.first += first;
                    acc.ttft_steps += 1;
                }
                if let (Some(first), Some(output)) = (metrics.first_token_ms, metrics.output_tokens) {
                    acc.decode_ms += metrics.elapsed_ms.saturating_sub(first);
                    acc.decode_tokens += output;
                }
                acc.last_output = metrics.output_tokens;
                acc.last_elapsed = metrics.elapsed_ms;
                acc.last_first = metrics.first_token_ms;
                acc.current = None;
                acc.dirty = false;
                let mut view = Self::metrics_view(acc);
                view.last_output_tokens = metrics.output_tokens;
                view.last_elapsed_ms = metrics.elapsed_ms;
                view.last_first_token_ms = metrics.first_token_ms;
                view
            } else {
                acc.current = Some(metrics.clone());
                // Live view: 已完成请求的累计值 + 当前请求的成对贡献；
                // 未上报 usage 的在途请求不进 decode 对，数字不回退漂移。
                let (pair_ms, pair_tokens) = match (metrics.first_token_ms, metrics.output_tokens) {
                    (Some(first), Some(output)) => (metrics.elapsed_ms.saturating_sub(first), output),
                    _ => (0, 0),
                };
                RunMetrics {
                    run_id: metrics.run_id,
                    session_id: metrics.session_id,
                    provider: metrics.provider.clone(),
                    model: metrics.model.clone(),
                    input_tokens: sum_tokens(acc.input, metrics.input_tokens),
                    output_tokens: sum_tokens(acc.output, metrics.output_tokens),
                    cache_hit_tokens: sum_tokens(acc.cache, metrics.cache_hit_tokens),
                    cache_write_tokens: sum_tokens(acc.cache_write, metrics.cache_write_tokens),
                    reasoning_tokens: sum_tokens(acc.reasoning, metrics.reasoning_tokens),
                    attempts: acc.attempts + 1,
                    last_input_tokens: last_known_input,
                    last_output_tokens: metrics.output_tokens,
                    last_elapsed_ms: metrics.elapsed_ms,
                    last_first_token_ms: metrics.first_token_ms,
                    elapsed_ms: acc.elapsed + metrics.elapsed_ms,
                    first_token_ms: Some(acc.first + metrics.first_token_ms.unwrap_or(0)),
                    ttft_steps: acc.ttft_steps + u32::from(metrics.first_token_ms.is_some()),
                    decode_ms: acc.decode_ms + pair_ms,
                    decode_tokens: acc.decode_tokens + pair_tokens,
                    tool_ms: acc.tool_ms,
                    live_surface_tokens: metrics.live_surface_tokens,
                }
            }
        };
        if persist {
            self.database.save_metrics(cumulative.clone()).await?;
        }
        self.emit_locked(
            &mut *self.state.lock().await,
            EventData::Metrics { metrics: cumulative },
        );
        Ok(())
    }

    /// DSH tokenUsage：usage 样本在请求失败时仍然保留 —— 流异常终止（无
    /// Done）但 provider 已上报 usage 时，只折叠 usage 桶进累计；失败 step
    /// 不产生 assistant/message/step/end，故 attempts/ttft/decode/llmMs
    /// 一概不动。置 dirty 由 finish() 统一落库，emit 立即刷新 UI。
    pub(crate) async fn report_usage_sample(&self, metrics: RunMetrics) -> AppResult<()> {
        self.report_usage_sample_inner(metrics, true).await
    }

    /// 同 `report_subagent_metrics`：失败子调用的 usage 样本保留但不得
    /// 覆盖父会话的 context pressure。
    pub(crate) async fn report_subagent_usage_sample(&self, metrics: RunMetrics) -> AppResult<()> {
        self.report_usage_sample_inner(metrics, false).await
    }

    async fn report_usage_sample_inner(&self, metrics: RunMetrics, main_conversation: bool) -> AppResult<()> {
        let cumulative = {
            let mut map = self.metrics_acc.lock().unwrap();
            let acc = map.entry(metrics.run_id).or_default();
            acc.run_id = metrics.run_id;
            acc.session_id = metrics.session_id;
            acc.provider = metrics.provider.clone();
            acc.model = metrics.model.clone();
            acc.input = sum_tokens(acc.input, metrics.input_tokens);
            acc.output = sum_tokens(acc.output, metrics.output_tokens);
            acc.cache = sum_tokens(acc.cache, metrics.cache_hit_tokens);
            acc.cache_write = sum_tokens(acc.cache_write, metrics.cache_write_tokens);
            acc.reasoning = sum_tokens(acc.reasoning, metrics.reasoning_tokens);
            if main_conversation {
                acc.last_known_input = metrics.input_tokens.or(acc.last_known_input);
            }
            acc.current = None;
            acc.dirty = true;
            Self::metrics_view(acc)
        };
        self.emit_locked(
            &mut *self.state.lock().await,
            EventData::Metrics { metrics: cumulative },
        );
        Ok(())
    }

    pub(crate) async fn finish(&self, run_id: i64, status: RunStatus, error: Option<String>) {
        let (mut run, segments, parts) = {
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
            if let Some(segment) = state.reasoning.last_mut() {
                if segment.ended_at.is_none() {
                    segment.ended_at = Some(now_ms());
                }
            }
            if let Some(part) = state.parts.last_mut() {
                if part.ended_at.is_none() {
                    part.ended_at = Some(now_ms());
                }
            }
            let segments = std::mem::take(&mut state.reasoning);
            let parts = std::mem::take(&mut state.parts);
            state.reasoning_run = None;
            (run, segments, parts)
        };
        let _ = self.database.save_run_reasoning(run_id, &segments).await;
        let _ = self.database.save_run_parts(run_id, &parts).await;
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
                let _ = pending.responder.send(ApprovalResponse {
                    decision: ApprovalDecision::Deny,
                    feedback: None,
                });
            }
        }
        state.control = None;
        state.cancelled_run = None;
        // 工具计时到达于最后一次模型上报之后：终局前把累计值补落库。
        let dirty_metrics = {
            let mut map = self.metrics_acc.lock().unwrap();
            let snapshot = map
                .get_mut(&run_id)
                .filter(|acc| acc.dirty && acc.run_id != 0)
                .map(|acc| {
                    acc.dirty = false;
                    Self::metrics_view(acc)
                });
            map.remove(&run_id);
            snapshot
        };
        if let Some(metrics) = dirty_metrics {
            let _ = self.database.save_metrics(metrics).await;
        }
        self.write_net.lock().unwrap().remove(&run_id);
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
                    let _ = pending.responder.send(ApprovalResponse {
                    decision: ApprovalDecision::Deny,
                    feedback: None,
                });
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
            context_window: None,
            session: None,
            vision_capable: true,
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
                Ok(TurnOutcome {
                    done: true,
                    tool_calls: Vec::new(),
                    content: "Hello 世界".into(),
                    thinking: None,
                    finish_reason: None,
                    dropped_tool_calls: 0,
                    input_tokens: None,
                })
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
        assert!(kernel.start_model(1, "parallel".into(), Vec::new(), test_config()).await.is_err());
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
            .start_model(session.id, "hello".into(), Vec::new(), test_config())
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
        assert!(kernel.start_model(1, "late".into(), Vec::new(), test_config()).await.is_err());
        assert!(kernel.set_mode(Mode::Execute).await.is_err());
    }

    #[tokio::test]
    async fn validates_prompts_on_the_backend() {
        let (kernel, _) = test_kernel().await;
        assert!(kernel.start_model(1, "  ".into(), Vec::new(), test_config()).await.is_err());
        assert!(kernel
            .start_model(1, "x".repeat(8193), Vec::new(), test_config())
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
        let session_id = kernel.database.list_runs().await.unwrap()[0].session_id;
        activate_seeded_run(kernel, id, session_id, mode).await
    }

    /// Seed a run in an existing session so cross-mode rules can be exercised.
    async fn seed_run_in_session(kernel: &Arc<Kernel>, session_id: i64, mode: Mode) -> i64 {
        let id = kernel
            .database
            .create_run_in_session(session_id, "boundary".into(), mode, Vec::new())
            .await
            .unwrap();
        activate_seeded_run(kernel, id, session_id, mode).await.0
    }

    async fn activate_seeded_run(
        kernel: &Arc<Kernel>,
        id: i64,
        session_id: i64,
        mode: Mode,
    ) -> (i64, watch::Receiver<bool>) {
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
            bulk_decision: None,
            pending_question: None,
            workspace_root: kernel.router.workspace_root().await,
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
        // Approving the plan is the execution sanction: the session hands off
        // to Code so the next run may write (ZCode plan→code transition).
        assert_eq!(kernel.snapshot().await.mode, Mode::Code);
        let (next, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        assert!(kernel.authorize_tool(next, "apply_patch").await.is_ok());
        kernel.finish(next, RunStatus::Completed, None).await;
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
    async fn new_session_falls_back_to_the_first_active_project() {
        let (kernel, _events) = test_kernel().await;
        let seeded = kernel.create_session(None).await.unwrap();
        assert_eq!(seeded.project_id, 1);

        let side = kernel.create_project("Side".into(), None).await.unwrap();
        kernel.set_project_archived(1, true).await.unwrap();
        let fallback = kernel.create_session(None).await.unwrap();
        assert_eq!(fallback.project_id, side.id);

        kernel.set_project_archived(side.id, true).await.unwrap();
        assert_eq!(
            kernel.create_session(None).await.unwrap_err(),
            "请先新建项目"
        );
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
        assert!(kernel.start_model(1, "late".into(), Vec::new(), test_config()).await.is_err());
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
            .agent_tools(Mode::Plan, None)
            .await
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert!(plan.iter().any(|name| name == "read_file"));
        assert!(plan.iter().any(|name| name == "load_skill"));
        assert!(plan.iter().any(|name| name == "update_plan"));
        assert!(!plan.iter().any(|name| name == "write_file"));
        assert!(!plan.iter().any(|name| name == "apply_patch"));
        // run_command 在 Plan 下保留：白名单只读命令可发起调用并逐项审批；
        // 且 Plan 规格不暴露 background 参数（后台执行走 shell，一律拒绝）。
        assert!(plan.iter().any(|name| name == "run_command"));
        let run_command = kernel
            .agent_tools(Mode::Plan, None)
            .await
            .into_iter()
            .find(|tool| tool.name == "run_command")
            .unwrap();
        assert!(run_command.parameters["properties"].get("background").is_none());
        for mode in [Mode::Code, Mode::Confirm, Mode::Auto] {
            let names: Vec<String> = kernel
                .agent_tools(mode, None)
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
        wait_for_phase_count(events, phase, 1).await;
    }

    /// 顺序挂起按出现次数等待：同一 run 里第二次 AwaitingApproval 不能命中
    /// 第一次留下的旧事件。
    async fn wait_for_phase_count(
        events: &Arc<StdMutex<Vec<KernelEvent>>>,
        phase: ToolPhase,
        count: usize,
    ) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if events
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|event| matches!(&event.data, EventData::ToolCall { phase: p, .. } if *p == phase))
                    .count()
                    >= count
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
        let (id, _rx) = seed_run_mode(&kernel, Mode::Confirm).await;
        let worker = kernel.clone();
        let call = tokio::spawn(async move {
            worker
                .execute_model_tool(id, "write_file", r#"{"path":"a.txt","content":"hello"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel.resolve_approval(id, ApprovalDecision::AllowOnce, CommandScope::Prefix, None).await.unwrap();
        assert!(call.await.unwrap().is_ok());
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("a.txt")).unwrap(),
            "hello"
        );
        kernel.finish(id, RunStatus::Completed, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn pinned_plan_mode_denies_writes_even_when_the_run_is_in_code_mode() {
        let (kernel, _events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        let calls = [(
            "write_file".to_string(),
            r#"{"path":"pinned.txt","content":"x"}"#.to_string(),
        )];
        let results = kernel.execute_tool_calls_as(id, &calls, Some(Mode::Plan)).await;
        assert!(results[0].is_err());
        assert!(!workspace.path().join("pinned.txt").exists());
        kernel.finish(id, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }

    fn subagent_fixture(id: &str, status: SubagentStatus) -> SubagentView {
        SubagentView {
            id: id.into(),
            run_id: 1,
            session_id: 1,
            agent: "explore".into(),
            description: "调研".into(),
            prompt: "读文件".into(),
            model: None,
            status,
            summary: String::new(),
            error: None,
            content: String::new(),
            reasoning: String::new(),
            tool_count: 0,
            input_tokens: None,
            output_tokens: None,
            reasoning_tokens: None,
            first_token_ms: None,
            elapsed_ms: 0,
            decode_ms: 0,
            decode_tokens: 0,
            last_input_tokens: None,
            started_at: 1,
            finished_at: None,
            tools: Vec::new(),
        }
    }

    #[tokio::test]
    async fn cancel_cascades_to_every_registered_subagent() {
        let (kernel, _events) = test_kernel().await;
        let (tx1, mut rx1) = tokio::sync::watch::channel(false);
        let (tx2, mut rx2) = tokio::sync::watch::channel(false);
        kernel.register_subagent_cancel("sa-1".into(), tx1);
        kernel.register_subagent_cancel("sa-2".into(), tx2);
        kernel.cancel(1).await.unwrap();
        assert!(*rx1.borrow_and_update(), "sa-1 must be cancelled with the parent");
        assert!(*rx2.borrow_and_update(), "sa-2 must be cancelled with the parent");
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn cancel_with_a_stale_run_id_is_a_noop() {
        let (kernel, _) = test_kernel().await;
        let (id, mut receiver) = seed_run(&kernel).await;
        // A retried abort for an already-finished run must not touch the
        // currently active one (the documented idempotent contract).
        kernel.cancel(id + 10_000).await.unwrap();
        assert!(!kernel.is_cancelled(id).await);
        assert!(!*receiver.borrow_and_update(), "stale id must not fire the active control channel");
        kernel.cancel(id).await.unwrap();
        assert!(kernel.is_cancelled(id).await);
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn cancel_leaves_other_runs_children_alone() {
        let (kernel, _) = test_kernel().await;
        let (id, _rx) = seed_run(&kernel).await;
        // A child that belongs to a different run id must not be hit by this
        // run's stop — the cascade is scoped, not global.
        let mut foreign = subagent_fixture("sa-other", SubagentStatus::Running);
        foreign.run_id = id + 10_000;
        kernel.subagents.lock().unwrap().insert(foreign.id.clone(), foreign);
        let (foreign_tx, mut foreign_rx) = tokio::sync::watch::channel(false);
        kernel.register_subagent_cancel("sa-other".into(), foreign_tx);
        kernel.cancel(id).await.unwrap();
        assert!(!*foreign_rx.borrow_and_update(), "foreign-run child must not be cancelled");
        assert_eq!(
            kernel.subagents.lock().unwrap()["sa-other"].status,
            SubagentStatus::Running
        );
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn cancel_marks_running_subagent_traces_cancelled() {
        let (kernel, _events) = test_kernel().await;
        kernel.subagents.lock().unwrap().insert("sa-1".into(), subagent_fixture("sa-1", SubagentStatus::Running));
        kernel.subagents.lock().unwrap().insert("sa-2".into(), subagent_fixture("sa-2", SubagentStatus::Completed));
        kernel.cancel(1).await.unwrap();
        {
            let map = kernel.subagents.lock().unwrap();
            assert_eq!(map["sa-1"].status, SubagentStatus::Cancelled);
            assert_eq!(map["sa-2"].status, SubagentStatus::Completed, "finished children must not be touched");
        }
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn cancel_subagent_works_after_the_parent_run_finished() {
        let (kernel, _events) = test_kernel().await;
        kernel.subagents.lock().unwrap().insert("sa-1".into(), subagent_fixture("sa-1", SubagentStatus::Running));
        let (tx, mut rx) = tokio::sync::watch::channel(false);
        kernel.register_subagent_cancel("sa-1".into(), tx);
        // No active run: the stop button must still work.
        kernel.cancel_subagent(1, "sa-1").await.unwrap();
        assert!(*rx.borrow_and_update());
        assert_eq!(kernel.subagents.lock().unwrap()["sa-1"].status, SubagentStatus::Cancelled);
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn cancel_subagent_is_idempotent() {
        let (kernel, _events) = test_kernel().await;
        kernel.subagents.lock().unwrap().insert("sa-1".into(), subagent_fixture("sa-1", SubagentStatus::Cancelled));
        kernel.cancel_subagent(1, "sa-1").await.unwrap();
        kernel.cancel_subagent(1, "sa-1").await.unwrap();
        {
            let map = kernel.subagents.lock().unwrap();
            assert_eq!(map["sa-1"].status, SubagentStatus::Cancelled);
        }
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn project_updates_round_trip_through_list_projects() {
        let (kernel, _events) = test_kernel().await;
        let project = kernel.create_project("旧名".into(), None).await.unwrap();
        kernel
            .update_project_name(project.id, "新名".into())
            .await
            .unwrap();
        kernel
            .update_project_path(project.id, "D:/workspace/demo".into())
            .await
            .unwrap();
        let listed = kernel.list_projects().await.unwrap();
        let updated = listed.iter().find(|item| item.id == project.id).unwrap();
        assert_eq!(updated.name, "新名");
        assert_eq!(updated.path.as_deref(), Some("D:/workspace/demo"));
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn metrics_keep_the_last_known_prompt_size() {
        let (kernel, events) = test_kernel().await;
        let base = RunMetrics {
            run_id: 5,
            session_id: 1,
            provider: "openai".into(),
            model: "test-model".into(),
            input_tokens: Some(136_047),
            output_tokens: Some(1),
            cache_hit_tokens: None,
            elapsed_ms: 1,
            first_token_ms: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
            attempts: 1,
            last_input_tokens: Some(136_047),
            last_output_tokens: None,
            last_elapsed_ms: 0,
            last_first_token_ms: None,
            ttft_steps: 0,
            decode_ms: 0,
            decode_tokens: 0,
            tool_ms: 0,
            live_surface_tokens: None,
        };
        kernel.report_metrics(base.clone(), false).await.unwrap();
        // A later tick without usage must not erase what was already known.
        let mut later = base;
        later.input_tokens = None;
        later.last_input_tokens = None;
        kernel.report_metrics(later, false).await.unwrap();
        let last = events
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|event| match &event.data {
                EventData::Metrics { metrics } => Some(metrics.last_input_tokens),
                _ => None,
            })
            .unwrap();
        assert_eq!(last, Some(136_047));
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn metrics_fold_pairs_decode_windows_with_output_usage() {
        let (kernel, events) = test_kernel().await;
        let (run_id, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        let base = RunMetrics {
            run_id,
            session_id: 1,
            provider: "openai".into(),
            model: "m".into(),
            input_tokens: Some(10),
            output_tokens: Some(100),
            cache_hit_tokens: None,
            elapsed_ms: 1_000,
            first_token_ms: Some(200),
            cache_write_tokens: None,
            reasoning_tokens: None,
            attempts: 1,
            last_input_tokens: None,
            last_output_tokens: None,
            last_elapsed_ms: 0,
            last_first_token_ms: None,
            ttft_steps: 0,
            decode_ms: 0,
            decode_tokens: 0,
            tool_ms: 0,
            live_surface_tokens: None,
        };
        // 完成请求 A：first+output 成对 → decode 800ms / 100 tokens，ttftSteps=1。
        kernel.report_metrics(base.clone(), true).await.unwrap();
        // 完成请求 B：有首 delta 但无 output usage → ttft 计入、decode 不计。
        let mut no_usage = base.clone();
        no_usage.output_tokens = None;
        no_usage.elapsed_ms = 500;
        no_usage.first_token_ms = Some(100);
        kernel.report_metrics(no_usage, true).await.unwrap();
        // 完成请求 C：有 output usage 但无首 delta → decode 仍不计。
        let mut no_first = base.clone();
        no_first.first_token_ms = None;
        kernel.report_metrics(no_first, true).await.unwrap();
        let metrics = events
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|event| match &event.data {
                EventData::Metrics { metrics } => Some(metrics.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(metrics.ttft_steps, 2);
        assert_eq!(metrics.first_token_ms, Some(300));
        assert_eq!(metrics.decode_ms, 800);
        assert_eq!(metrics.decode_tokens, 100);
        // usage 汇总与 decode 对无关：三次 output 都在 output_tokens 里。
        assert_eq!(metrics.output_tokens, Some(200));
        kernel.finish(run_id, RunStatus::Completed, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn usage_sample_folds_tokens_without_touching_step_stats() {
        let (kernel, events) = test_kernel().await;
        let (run_id, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        // 完成请求 A：建立 baseline（attempts=1, decode 800ms/100tok, ttft 200）。
        let mut base = RunMetrics {
            run_id,
            session_id: 1,
            provider: "openai".into(),
            model: "m".into(),
            input_tokens: Some(10),
            output_tokens: Some(100),
            cache_hit_tokens: Some(5),
            elapsed_ms: 1_000,
            first_token_ms: Some(200),
            cache_write_tokens: None,
            reasoning_tokens: None,
            attempts: 1,
            last_input_tokens: None,
            last_output_tokens: None,
            last_elapsed_ms: 0,
            last_first_token_ms: None,
            ttft_steps: 0,
            decode_ms: 0,
            decode_tokens: 0,
            tool_ms: 0,
            live_surface_tokens: None,
        };
        kernel.report_metrics(base.clone(), true).await.unwrap();
        // 失败请求 B：流中途断流（无 Done），只带 usage 样本 —— DSH
        // tokenUsage 口径：usage 进累计，attempts/ttft/decode/elapsed 不动。
        base.input_tokens = Some(50);
        base.output_tokens = Some(400);
        base.cache_hit_tokens = None;
        base.elapsed_ms = 9_000;
        base.first_token_ms = Some(100);
        kernel.report_usage_sample(base).await.unwrap();
        let metrics = events
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|event| match &event.data {
                EventData::Metrics { metrics } => Some(metrics.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(metrics.attempts, 1);
        assert_eq!(metrics.elapsed_ms, 1_000);
        assert_eq!(metrics.ttft_steps, 1);
        assert_eq!(metrics.first_token_ms, Some(200));
        assert_eq!(metrics.decode_ms, 800);
        assert_eq!(metrics.decode_tokens, 100);
        assert_eq!(metrics.input_tokens, Some(60));
        assert_eq!(metrics.output_tokens, Some(500));
        assert_eq!(metrics.cache_hit_tokens, Some(5));
        kernel.finish(run_id, RunStatus::Failed, Some("断流".into())).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn subagent_reports_do_not_overwrite_parent_context_pressure() {
        let (kernel, events) = test_kernel().await;
        let (run_id, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        let base = RunMetrics {
            run_id,
            session_id: 1,
            provider: "openai".into(),
            model: "m".into(),
            input_tokens: Some(136_047),
            output_tokens: Some(100),
            cache_hit_tokens: None,
            elapsed_ms: 1_000,
            first_token_ms: Some(200),
            cache_write_tokens: None,
            reasoning_tokens: None,
            attempts: 1,
            last_input_tokens: None,
            last_output_tokens: None,
            last_elapsed_ms: 0,
            last_first_token_ms: None,
            ttft_steps: 0,
            decode_ms: 0,
            decode_tokens: 0,
            tool_ms: 0,
            live_surface_tokens: None,
        };
        kernel.report_metrics(base.clone(), true).await.unwrap();
        // 子智能体调用（小 prompt）：usage 桶照累计、attempts 照计，但
        // last_input_tokens 属于父会话 context pressure，不得被覆盖。
        let mut child = base.clone();
        child.input_tokens = Some(2_000);
        child.output_tokens = Some(50);
        kernel.report_subagent_metrics(child.clone(), false).await.unwrap();
        kernel.report_subagent_metrics(child.clone(), true).await.unwrap();
        let mut child_sample = child;
        child_sample.input_tokens = Some(2_500);
        kernel.report_subagent_usage_sample(child_sample).await.unwrap();
        let last = || {
            events
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find_map(|event| match &event.data {
                    EventData::Metrics { metrics } => Some(metrics.clone()),
                    _ => None,
                })
                .unwrap()
        };
        let metrics = last();
        assert_eq!(metrics.last_input_tokens, Some(136_047));
        assert_eq!(metrics.input_tokens, Some(136_047 + 2_000 + 2_500));
        assert_eq!(metrics.attempts, 2);
        // 父会话下一次请求的 usage 到达后正常刷新。
        let mut parent = base;
        parent.input_tokens = Some(140_000);
        kernel.report_metrics(parent, false).await.unwrap();
        assert_eq!(last().last_input_tokens, Some(140_000));
        kernel.finish(run_id, RunStatus::Completed, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[test]
    fn audit_labels_prefer_the_strongest_reason() {
        assert_eq!(Kernel::audit_label(AuditKind::RuleApproved), "auto-approved");
        assert_eq!(Kernel::audit_label(AuditKind::SessionGranted), "session granted");
        assert_eq!(Kernel::audit_label(AuditKind::ReadOnlyAllowlist), "read-only allowlist");
        assert_eq!(Kernel::audit_label(AuditKind::Autonomous), "autonomous");
        assert_eq!(Kernel::audit_label(AuditKind::Extension), "extension");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn code_mode_runs_allowlisted_read_commands_without_approval() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        let output = kernel
            .execute_model_tool(id, "run_command", r#"{"command":"where cmd"}"#)
            .await
            .unwrap();
        assert!(output.contains("exit code 0"), "{output}");
        assert!(output.to_lowercase().contains("cmd.exe"), "{output}");
        {
            let events = events.lock().unwrap();
            assert!(
                !events.iter().any(|event| matches!(
                    &event.data,
                    EventData::ToolCall { phase: ToolPhase::AwaitingApproval, .. }
                )),
                "an allowlisted read command must not ask for approval"
            );
        }
        kernel.finish(id, RunStatus::Completed, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn code_mode_suspends_dangerous_commands() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        let worker = kernel.clone();
        let call = tokio::spawn(async move {
            worker
                .execute_model_tool(id, "run_command", r#"{"command":"git push"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel
            .resolve_approval(id, ApprovalDecision::Deny, CommandScope::Exact, None)
            .await
            .unwrap();
        assert!(call.await.unwrap().is_err());
        kernel.finish(id, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }

    fn awaiting_count(events: &Arc<StdMutex<Vec<KernelEvent>>>) -> usize {
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| {
                matches!(&event.data, EventData::ToolCall { phase: ToolPhase::AwaitingApproval, .. })
            })
            .count()
    }

    #[tokio::test]
    async fn allow_batch_approves_remaining_suspended_calls() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        let worker = kernel.clone();
        let call = tokio::spawn(async move {
            worker
                .execute_tool_calls(
                    id,
                    &[
                        ("run_command".into(), r#"{"command":"git push"}"#.into()),
                        ("run_command".into(), r#"{"command":"git push --force"}"#.into()),
                    ],
                )
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel
            .resolve_approval(id, ApprovalDecision::AllowBatch, CommandScope::Exact, None)
            .await
            .unwrap();
        let results = call.await.unwrap();
        assert_eq!(results.len(), 2);
        // 一次弹卡解决整批：第二个调用不再等待，也没有第二张卡。
        assert_eq!(awaiting_count(&events), 1);
        kernel.finish(id, RunStatus::Completed, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn deny_batch_rejects_remaining_suspended_calls() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        let worker = kernel.clone();
        let call = tokio::spawn(async move {
            worker
                .execute_tool_calls(
                    id,
                    &[
                        ("run_command".into(), r#"{"command":"git push"}"#.into()),
                        ("run_command".into(), r#"{"command":"rm -rf build"}"#.into()),
                    ],
                )
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel
            .resolve_approval(
                id,
                ApprovalDecision::DenyBatch,
                CommandScope::Exact,
                Some("不要推送".into()),
            )
            .await
            .unwrap();
        let results = call.await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| result.is_err()));
        assert!(results.iter().all(|result| {
            result.as_ref().unwrap_err().to_string().contains("不要推送")
        }));
        assert_eq!(awaiting_count(&events), 1);
        kernel.finish(id, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn batch_decision_does_not_leak_into_the_next_batch() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        let worker = kernel.clone();
        let call = tokio::spawn(async move {
            worker
                .execute_tool_calls(
                    id,
                    &[
                        ("run_command".into(), r#"{"command":"git push"}"#.into()),
                        ("run_command".into(), r#"{"command":"git push --force"}"#.into()),
                    ],
                )
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel
            .resolve_approval(id, ApprovalDecision::AllowBatch, CommandScope::Exact, None)
            .await
            .unwrap();
        call.await.unwrap();
        events.lock().unwrap().clear();
        // 下一批的危险调用重新挂起——批量决定不外泄。
        let worker = kernel.clone();
        let next = tokio::spawn(async move {
            worker
                .execute_model_tool(id, "run_command", r#"{"command":"git push"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel
            .resolve_approval(id, ApprovalDecision::Deny, CommandScope::Exact, None)
            .await
            .unwrap();
        assert!(next.await.unwrap().is_err());
        kernel.finish(id, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn allow_session_waives_future_approvals_until_revoked() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        let worker = kernel.clone();
        let call = tokio::spawn(async move {
            worker
                .execute_model_tool(id, "run_command", r#"{"command":"git push"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel
            .resolve_approval(id, ApprovalDecision::AllowSession, CommandScope::Exact, None)
            .await
            .unwrap();
        let _ = call.await.unwrap();
        // 合成行出现在授权规则列表，可撤销。
        let rules = kernel.list_approval_rules(id).await.unwrap();
        assert!(rules.iter().any(|rule| rule.tool == "*"));
        events.lock().unwrap().clear();
        // 后续危险调用（新批次）直接执行，无卡片。
        let result = kernel
            .execute_model_tool(id, "run_command", r#"{"command":"git push --force"}"#)
            .await;
        let _ = result;
        assert_eq!(awaiting_count(&events), 0);
        // 撤销后恢复风险询问。
        let grant = ApprovalRule { tool: "*".into(), pattern: "本会话全部放行".into(), prefix: false };
        assert!(kernel.revoke_approval_rule(id, &grant).await.unwrap());
        assert!(kernel
            .list_approval_rules(id)
            .await
            .unwrap()
            .iter()
            .all(|rule| rule.tool != "*"));
        kernel.finish(id, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn session_grant_never_lifts_a_plan_deny() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        let worker = kernel.clone();
        let call = tokio::spawn(async move {
            worker
                .execute_model_tool(id, "run_command", r#"{"command":"git push"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel
            .resolve_approval(id, ApprovalDecision::AllowSession, CommandScope::Exact, None)
            .await
            .unwrap();
        let _ = call.await.unwrap();
        // 同会话切到 Plan：非白名单命令的绝对拒绝优先于会话放行。
        let results = kernel
            .execute_tool_calls_as(
                id,
                &[("run_command".into(), r#"{"command":"git push"}"#.into())],
                Some(Mode::Plan),
            )
            .await;
        assert!(results[0].is_err());
        // 白名单命令在 Plan 下直放免审批——严格 argv+无 shell 执行，不再需要
        // 逐项审批环节。
        let results = kernel
            .execute_tool_calls_as(
                id,
                &[("run_command".into(), r#"{"command":"git status"}"#.into())],
                Some(Mode::Plan),
            )
            .await;
        assert!(results[0].is_ok(), "{:?}", results[0]);
        kernel.finish(id, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn session_rules_are_listed_and_revocable() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Confirm).await;
        let session_id = kernel.database.list_runs().await.unwrap()[0].session_id;
        let worker = kernel.clone();
        let call = tokio::spawn(async move {
            worker
                .execute_model_tool(id, "write_file", r#"{"path":"a.txt","content":"one"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel
            .resolve_approval(id, ApprovalDecision::AllowAlways, CommandScope::Exact, None)
            .await
            .unwrap();
        assert!(call.await.unwrap().is_ok());

        let rules = kernel.list_approval_rules(id).await.unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].tool, "write_file");
        assert_eq!(rules[0].pattern, "a.txt");
        assert!(!rules[0].prefix);
        let candidate = Kernel::approval_candidate("write_file", r#"{"path":"a.txt","content":"two"}"#);
        assert!(kernel.is_auto_approved(session_id, &candidate));

        assert!(kernel.revoke_approval_rule(id, &rules[0]).await.unwrap());
        assert!(kernel.list_approval_rules(id).await.unwrap().is_empty());
        assert!(!kernel.is_auto_approved(session_id, &candidate));
        kernel.finish(id, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[test]
    fn mcp_candidates_are_identified_by_their_full_argument_set() {
        let first = Kernel::approval_candidate("mcp__fs__write", r#"{"path":"a.txt","content":"one"}"#);
        let same = Kernel::approval_candidate("mcp__fs__write", r#"{"content":"one","path":"a.txt"}"#);
        let other = Kernel::approval_candidate("mcp__fs__write", r#"{"path":"a.txt","content":"two"}"#);
        assert!(!first.target.is_empty());
        assert!(first.scopable);
        assert_eq!(first.target, same.target);
        assert_ne!(first.target, other.target);
    }

    #[tokio::test]
    async fn an_external_edit_during_approval_cancels_the_write() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        std::fs::write(workspace.path().join("a.txt"), "original").unwrap();
        let (id, _rx) = seed_run_mode(&kernel, Mode::Confirm).await;
        // Read-before-write: the model must observe the file before edits.
        // In Confirm mode the read itself also needs approval.
        let reader = kernel.clone();
        let read_call = tokio::spawn(async move {
            reader
                .execute_model_tool(id, "read_file", r#"{"path":"a.txt"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel
            .resolve_approval(id, ApprovalDecision::AllowOnce, CommandScope::Exact, None)
            .await
            .unwrap();
        read_call.await.unwrap().unwrap();
        // Clear the read's stale approval event so the next wait tracks the
        // write call only.
        events.lock().unwrap().clear();
        let worker = kernel.clone();
        let call = tokio::spawn(async move {
            worker
                .execute_model_tool(id, "write_file", r#"{"path":"a.txt","content":"from-model"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        // The user edits the file while the approval card is open.
        std::fs::write(workspace.path().join("a.txt"), "edited-outside").unwrap();
        kernel
            .resolve_approval(id, ApprovalDecision::AllowOnce, CommandScope::Exact, None)
            .await
            .unwrap();
        let error = call.await.unwrap().unwrap_err();
        assert!(error.contains("外部修改"), "{error}");
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("a.txt")).unwrap(),
            "edited-outside"
        );
        kernel.finish(id, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn revert_refuses_a_file_edited_after_the_preview() {
        let (kernel, _events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        std::fs::write(workspace.path().join("a.txt"), "after-model").unwrap();
        let change = crate::domain::RunFileChange {
            path: "a.txt".into(),
            added: 1,
            removed: 1,
            before: "before-model".into(),
            after: "after-model".into(),
            created_at: 1,
        };
        // Simulate an edit between the revert preview and the write itself by
        // checking the write path directly (the preview would have said safe).
        assert_eq!(kernel.router.preview_revert_changes(Some(workspace.path()), std::slice::from_ref(&change)).await.unwrap().safe_files.len(), 1);
        std::fs::write(workspace.path().join("a.txt"), "edited-outside").unwrap();
        let result = kernel.router.revert_changes(Some(workspace.path()), &[change]).await.unwrap();
        assert!(result.reverted.is_empty());
        assert_eq!(result.conflicts, vec!["a.txt".to_string()]);
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("a.txt")).unwrap(),
            "edited-outside"
        );
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn rejected_approval_blocks_the_write() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Confirm).await;
        let worker = kernel.clone();
        let call = tokio::spawn(async move {
            worker
                .execute_model_tool(id, "write_file", r#"{"path":"b.txt","content":"nope"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel
            .resolve_approval(id, ApprovalDecision::Deny, CommandScope::Prefix, Some("不要写 b.txt".into()))
            .await
            .unwrap();
        let error = call.await.unwrap().unwrap_err();
        assert!(error.contains("不要写 b.txt"), "{error}");
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
    async fn allow_always_skips_the_next_approval_for_the_same_target() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Confirm).await;
        let worker = kernel.clone();
        let first = tokio::spawn(async move {
            worker
                .execute_model_tool(id, "write_file", r#"{"path":"a.txt","content":"one"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel
            .resolve_approval(id, ApprovalDecision::AllowAlways, CommandScope::Prefix, None)
            .await
            .unwrap();
        assert!(first.await.unwrap().is_ok());
        let second = tokio::time::timeout(
            Duration::from_secs(2),
            kernel.execute_model_tool(id, "write_file", r#"{"path":"a.txt","content":"two"}"#),
        )
        .await
        .expect("allow-always must not suspend the same target again")
        .unwrap();
        assert!(second.contains("a.txt"));
        kernel.finish(id, RunStatus::Completed, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn historical_allow_always_cannot_bypass_plan_writes() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (confirm_run, _rx) = seed_run_mode(&kernel, Mode::Confirm).await;
        let session_id = kernel.database.list_runs().await.unwrap()[0].session_id;
        let worker = kernel.clone();
        let first = tokio::spawn(async move {
            worker
                .execute_model_tool(confirm_run, "write_file", r#"{"path":"a.txt","content":"one"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel
            .resolve_approval(confirm_run, ApprovalDecision::AllowAlways, CommandScope::Exact, None)
            .await
            .unwrap();
        assert!(first.await.unwrap().is_ok());
        kernel.finish(confirm_run, RunStatus::Completed, None).await;

        let plan_run = seed_run_in_session(&kernel, session_id, Mode::Plan).await;
        let denied = kernel
            .execute_model_tool(plan_run, "write_file", r#"{"path":"a.txt","content":"two"}"#)
            .await
            .unwrap_err();
        assert!(denied.contains("Plan Mode blocked"), "{denied}");
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("a.txt")).unwrap(),
            "one"
        );
        assert!(events.lock().unwrap().iter().any(|event| matches!(
            &event.data,
            EventData::ToolCall { tool, phase: ToolPhase::Blocked, .. } if tool == "write_file"
        )));
        kernel.finish(plan_run, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn historical_allow_always_does_not_waive_plan_command_approvals() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (confirm_run, _rx) = seed_run_mode(&kernel, Mode::Confirm).await;
        let session_id = kernel.database.list_runs().await.unwrap()[0].session_id;
        let worker = kernel.clone();
        let first = tokio::spawn(async move {
            worker
                .execute_model_tool(confirm_run, "run_command", r#"{"command":"whoami"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel
            .resolve_approval(confirm_run, ApprovalDecision::AllowAlways, CommandScope::Exact, None)
            .await
            .unwrap();
        assert!(first.await.unwrap().is_ok());
        kernel.finish(confirm_run, RunStatus::Completed, None).await;

        // Plan 模式：白名单只读命令直放免审批——严格 argv 解析+无 shell 执行
        // 比 read_file 更安全，挂起是纯摩擦。
        let plan_run = seed_run_in_session(&kernel, session_id, Mode::Plan).await;
        let output = kernel
            .execute_model_tool(plan_run, "run_command", r#"{"command":"whoami"}"#)
            .await
            .unwrap();
        assert!(!output.trim().is_empty());
        kernel.finish(plan_run, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn plan_mode_denies_non_allowlisted_and_background_commands() {
        let (kernel, _events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (plan_run, _rx) = seed_run_mode(&kernel, Mode::Plan).await;
        // 非白名单命令：不挂起，直接拒绝。
        for arguments in [
            r#"{"command":"npm install"}"#,
            r#"{"command":"git push"}"#,
            r#"{"command":"whoami","background":true}"#,
        ] {
            let denied = kernel
                .execute_model_tool(plan_run, "run_command", arguments)
                .await
                .unwrap_err();
            assert!(denied.contains("Plan Mode blocked"), "{denied}");
        }
        // 白名单命令直放：无审批环节，直接执行返回输出。
        let output = kernel
            .execute_model_tool(plan_run, "run_command", r#"{"command":"whoami"}"#)
            .await
            .unwrap();
        assert!(output.contains("exit code 0"), "{output}");
        kernel.finish(plan_run, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn subagent_scoped_calls_downgrade_plan_approval_to_deny() {
        let (kernel, _events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (plan_run, _rx) = seed_run_mode(&kernel, Mode::Plan).await;
        // 白名单只读命令：子智能体作用域同样直放（用户拍板——研究离不开
        // git 只读感知）；非白名单命令依旧拒绝。
        let calls = [(
            "run_command".to_string(),
            r#"{"command":"whoami"}"#.to_string(),
        )];
        let results = kernel
            .execute_subagent_tool_calls(plan_run, "sub-plan", &calls, Some(Mode::Plan))
            .await;
        assert!(results[0].as_ref().is_ok(), "{:?}", results[0]);
        let denied = [(
            "run_command".to_string(),
            r#"{"command":"git push"}"#.to_string(),
        )];
        let results = kernel
            .execute_subagent_tool_calls(plan_run, "sub-plan", &denied, Some(Mode::Plan))
            .await;
        assert!(
            results[0].as_ref().unwrap_err().contains("Plan Mode blocked"),
            "{:?}",
            results[0]
        );
        kernel.finish(plan_run, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn historical_allow_always_still_waives_approval_in_an_allowing_mode() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (confirm_run, _rx) = seed_run_mode(&kernel, Mode::Confirm).await;
        let session_id = kernel.database.list_runs().await.unwrap()[0].session_id;
        let worker = kernel.clone();
        let first = tokio::spawn(async move {
            worker
                .execute_model_tool(confirm_run, "write_file", r#"{"path":"b.txt","content":"one"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel
            .resolve_approval(confirm_run, ApprovalDecision::AllowAlways, CommandScope::Exact, None)
            .await
            .unwrap();
        assert!(first.await.unwrap().is_ok());
        kernel.finish(confirm_run, RunStatus::Completed, None).await;

        // Confirm again in the same session: the rule still removes the prompt.
        let second_run = seed_run_in_session(&kernel, session_id, Mode::Confirm).await;
        let second = tokio::time::timeout(
            Duration::from_secs(2),
            kernel.execute_model_tool(second_run, "write_file", r#"{"path":"b.txt","content":"two"}"#),
        )
        .await
        .expect("a session rule must still waive approval outside Plan")
        .unwrap();
        assert!(second.contains("b.txt"));
        kernel.finish(second_run, RunStatus::Cancelled, None).await;
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
        let (id, _rx) = seed_run_mode(&kernel, Mode::Confirm).await;
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
    async fn write_result_carries_cumulative_net_counts_across_writes() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Code).await;
        kernel
            .execute_model_tool(id, "write_file", r#"{"path":"n.txt","content":"a\nb\nc\n"}"#)
            .await
            .unwrap();
        kernel
            .execute_model_tool(id, "write_file", r#"{"path":"n.txt","content":"a\nx\n"}"#)
            .await
            .unwrap();
        let details: Vec<serde_json::Value> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match &event.data {
                EventData::ToolCall { phase: ToolPhase::Finished, detail, .. }
                    if detail.contains("write_result") =>
                {
                    Some(serde_json::from_str(detail).unwrap())
                }
                _ => None,
            })
            .collect();
        assert_eq!(details.len(), 2);
        // 首次写入：净差与逐次计数一致（新文件 3 行）。
        assert_eq!(details[0]["added"], 3);
        assert_eq!(details[0]["net_added"], 3);
        assert_eq!(details[0]["net_removed"], 0);
        // 第二次写入逐次 diff：a\nb\nc\n → a\nx\n = +1 −2；
        // 净差按 首版 before("")→ 最新 after 计 = +2 −0，与 DB 合并口径一致。
        assert_eq!(details[1]["added"], 1);
        assert_eq!(details[1]["removed"], 2);
        assert_eq!(details[1]["net_added"], 2);
        assert_eq!(details[1]["net_removed"], 0);
        kernel.finish(id, RunStatus::Completed, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn code_mode_run_command_suspends_then_executes_after_approval() {
        let (kernel, events) = test_kernel().await;
        let workspace = tempfile::tempdir().unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run_mode(&kernel, Mode::Confirm).await;
        let worker = kernel.clone();
        // whoami 在 Windows(System32) 与 Unix 上都是 PATH 常驻二进制；
        // echo 依赖 Git usr\bin，纯 PowerShell 环境解析不到。
        let call = tokio::spawn(async move {
            worker
                .execute_model_tool(id, "run_command", r#"{"command":"whoami"}"#)
                .await
        });
        wait_for_phase(&events, ToolPhase::AwaitingApproval).await;
        kernel.resolve_approval(id, ApprovalDecision::AllowOnce, CommandScope::Prefix, None).await.unwrap();
        let output = call.await.unwrap().unwrap();
        assert!(output.contains("exit code 0"), "{output}");
        assert!(output.trim().len() > "exit code 0".len(), "{output}");
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
        kernel.resolve_approval(id, ApprovalDecision::AllowOnce, CommandScope::Prefix, None).await.unwrap();
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

    /// trigger(plan)→squad 派发义务图：全局/项目两文件各一段。
    fn rules_json(squad_edge: bool) -> String {
        let (nodes, edges, squads) = if squad_edge {
            (
                r#"[{"id":"gt","type":"trigger","position":{"x":0,"y":0},"data":{"action":"plan"}},
                   {"id":"gsq","type":"squad","position":{"x":0,"y":0},"data":{"squad":"team"}}]"#,
                r#"[{"id":"ge1","source":"gt","target":"gsq","sourceHandle":"out","targetHandle":"in"}]"#,
                r#""squads":{"team":{"nodes":[{"id":"m","type":"subagent","position":{"x":0,"y":0},"data":{"agent":"explore"}}],"edges":[]}}"#,
            )
        } else {
            (
                r#"[{"id":"pt","type":"trigger","position":{"x":0,"y":0},"data":{"action":"modify","targetDir":"src"}},
                   {"id":"pa","type":"action","position":{"x":0,"y":0},"data":{"result":"block","message":"stop"}}]"#,
                r#"[{"id":"pe1","source":"pt","target":"pa","sourceHandle":"out","targetHandle":"in"}]"#,
                r#""squads":{}"#,
            )
        };
        format!(r#"{{"version":1,"nodes":{nodes},"edges":{edges},{squads}}}"#)
    }

    #[tokio::test]
    async fn special_channel_calls_pass_through_architecture() {
        // ask_user/spawn_subagent 不走 execute_tool_calls——专属检查要与
        // 批内 gate 同语义：plan 触发链的派发义务拦 ask_user，spawn 自满足放行。
        let (kernel, _) = test_kernel().await;
        std::fs::write(
            kernel.extensions().maestro_dir().join("graph_rules.json"),
            rules_json(true),
        )
        .unwrap();
        kernel.router.reload_architecture().await;
        let (id, _rx) = seed_run(&kernel).await;
        let session_id = kernel.database.list_runs().await.unwrap()[0].session_id;
        // 未派发军团 → ask_user（plan 触发器覆盖）被拦。
        let hit = kernel
            .architecture_check_special(id, session_id, "ask_user", "{}", "调研")
            .await;
        assert!(hit.is_err(), "义务未满足应拦截 ask_user");
        // spawn 派发该军团本身不被拦（self-satisfying）。
        let spawn = kernel
            .architecture_check_special(
                id,
                session_id,
                "spawn_subagent",
                r#"{"subagent_type":"team","description":"调研"}"#,
                "调研",
            )
            .await;
        assert!(spawn.is_ok(), "派发军团自身应放行");
        // record_spawn 写入 "type/description" 历史 → 义务满足 → ask_user 放行。
        kernel.record_spawn(id, "sa-1", "调研", "team/调研").await;
        let pass = kernel
            .architecture_check_special(id, session_id, "ask_user", "{}", "调研")
            .await;
        assert!(pass.is_ok(), "派发后应放行，此前 description 原文的历史 target 永不命中");
        kernel.finish(id, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn architecture_rules_merge_global_and_project() {
        // 全局与项目规则文件合并生效（不再文件级遮蔽）：全局义务图 +
        // 项目阻断图各自命中各自的链。
        let (kernel, _) = test_kernel().await;
        std::fs::write(
            kernel.extensions().maestro_dir().join("graph_rules.json"),
            rules_json(true),
        )
        .unwrap();
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join(".maestro")).unwrap();
        std::fs::write(
            workspace.path().join(".maestro").join("graph_rules.json"),
            rules_json(false),
        )
        .unwrap();
        kernel
            .trust_workspace(workspace.path().to_string_lossy().to_string())
            .await
            .unwrap();
        kernel
            .set_workspace_root(Some(workspace.path().to_path_buf()))
            .await;
        let (id, _rx) = seed_run(&kernel).await;
        let session_id = kernel.database.list_runs().await.unwrap()[0].session_id;
        // 项目图：写 src/ → 拦。
        let project_hit = kernel
            .architecture_check_special(id, session_id, "write_file", r#"{"path":"src/a.ts","content":"x"}"#, "")
            .await;
        assert!(project_hit.is_err(), "项目规则应命中：{}", project_hit.err().unwrap_or_default());
        // 全局图：plan 触发链义务未满足 → 拦。
        let global_hit = kernel
            .architecture_check_special(id, session_id, "ask_user", "{}", "")
            .await;
        assert!(global_hit.is_err(), "全局军团义务应命中（合并后不再被项目文件遮蔽）");
        kernel.finish(id, RunStatus::Cancelled, None).await;
        kernel.shutdown().await.unwrap();
    }

    /// 收尾轮探针 provider：记录每轮 tools 载荷长度；工具被剥离时输出
    /// 最终结论文本，否则按 fanout 扇出互异 read_file 调用（避开停滞检测）。
    struct SynthProbeProvider {
        seen: StdMutex<Vec<usize>>,
        counter: std::sync::atomic::AtomicUsize,
        fanout: usize,
    }

    impl LlmProvider for SynthProbeProvider {
        fn stream<'a>(
            self: Arc<Self>,
            _config: &'a ChatConfig,
            _messages: &'a [crate::domain::ChatMessage],
            tools: &'a [crate::domain::ToolSpec],
            _cancel: &'a mut watch::Receiver<bool>,
            events: tokio::sync::mpsc::UnboundedSender<crate::adapters::llm::StreamEvent>,
        ) -> futures_util::future::BoxFuture<'a, AppResult<crate::adapters::llm::TurnOutcome>> {
            self.seen.lock().unwrap().push(tools.len());
            let stripped = tools.is_empty();
            let fanout = self.fanout;
            let base = self.counter.fetch_add(fanout, std::sync::atomic::Ordering::Relaxed);
            Box::pin(async move {
                use crate::adapters::llm::{StreamEvent, TurnOutcome};
                if stripped {
                    let _ = events.send(StreamEvent::Token("最终综合结论".into()));
                    let _ = events.send(StreamEvent::Done);
                    return Ok(TurnOutcome {
                        done: true,
                        tool_calls: Vec::new(),
                        content: "最终综合结论".into(),
                        thinking: None,
                        finish_reason: None,
                        dropped_tool_calls: 0,
                        input_tokens: None,
                    });
                }
                let _ = events.send(StreamEvent::Token("继续读取".into()));
                let _ = events.send(StreamEvent::Done);
                let tool_calls = (0..fanout)
                    .map(|i| crate::domain::ToolCallSpec {
                        id: format!("call-{}", base + i),
                        name: "read_file".into(),
                        arguments: format!("{{\"path\":\"f{}.rs\"}}", base + i),
                    })
                    .collect();
                Ok(TurnOutcome {
                    done: true,
                    tool_calls,
                    content: "继续读取".into(),
                    thinking: None,
                    finish_reason: None,
                    dropped_tool_calls: 0,
                    input_tokens: None,
                })
            })
        }
    }

    /// 内建 explore 已无硬顶（ZCode 哲学）——测试收尾轮安全阀须显式设上限的
    /// 自定义定义。克隆内建改名后写入全局，caps 由调用方按场景指定。
    async fn save_capped_explore(kernel: &Arc<Kernel>, steps: Option<u32>, tool_budget: Option<u32>) {
        let mut capped = crate::domain::AgentDefinition::builtin("explore").unwrap();
        capped.name = "capped-explore".into();
        capped.steps = steps;
        capped.tool_budget = tool_budget;
        kernel.save_agent("global".into(), capped).await.unwrap();
    }

    async fn drive_explore_subagent(kernel: &Arc<Kernel>) -> String {
        drive_typed_subagent(kernel, "capped-explore", &test_config()).await.unwrap()
    }

    async fn drive_typed_subagent(kernel: &Arc<Kernel>, subagent_type: &str, config: &ChatConfig) -> AppResult<String> {
        let (run_id, rx) = seed_run(kernel).await;
        let session_id = kernel.database.list_runs().await.unwrap()[0].session_id;
        crate::services::agent_loop::run_subagent(
            kernel.clone(),
            config,
            session_id,
            run_id,
            "sub-probe".into(),
            &format!(r#"{{"description":"测试调研","prompt":"调研仓库结构","subagent_type":"{subagent_type}"}}"#),
            rx,
            false,
            Vec::new(),
        )
        .await
    }

    #[tokio::test]
    async fn final_step_strips_tools_and_returns_a_synthesis() {
        let (kernel, _) = test_kernel().await;
        let provider = Arc::new(SynthProbeProvider {
            seen: StdMutex::new(Vec::new()),
            counter: std::sync::atomic::AtomicUsize::new(0),
            fanout: 1,
        });
        let probe = provider.clone();
        *kernel.llm_resolver.lock().unwrap() = Arc::new(move |_| probe.clone());
        save_capped_explore(&kernel, Some(8), None).await;
        let summary = drive_explore_subagent(&kernel).await;
        let seen = provider.seen.lock().unwrap().clone();
        // 显式 8 轮上限：前 7 轮带工具载荷，最后一轮工具必须为空。
        assert_eq!(seen.len(), 8, "应恰好跑满 8 轮：{seen:?}");
        assert!(seen[..7].iter().all(|count| *count > 0), "前 7 轮应带工具：{seen:?}");
        assert_eq!(seen[7], 0, "最后一轮工具必须剥离：{seen:?}");
        assert!(summary.contains("最终综合结论"), "收尾结论必须送达父级：{summary}");
        assert!(summary.contains("轮次上限（8 轮）"), "截断标注应透传父模型：{summary}");
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn budget_exhaustion_triggers_the_synthesis_round_instead_of_fragments() {
        let (kernel, _) = test_kernel().await;
        let provider = Arc::new(SynthProbeProvider {
            seen: StdMutex::new(Vec::new()),
            counter: std::sync::atomic::AtomicUsize::new(0),
            fanout: 120,
        });
        let probe = provider.clone();
        *kernel.llm_resolver.lock().unwrap() = Arc::new(move |_| probe.clone());
        save_capped_explore(&kernel, None, Some(100)).await;
        let summary = drive_explore_subagent(&kernel).await;
        let seen = provider.seen.lock().unwrap().clone();
        // 第一轮 120 次调用直接击穿 100 显式预算：第二轮即收尾轮。
        assert_eq!(seen.len(), 2, "预算耗尽后只应再跑一个收尾轮：{seen:?}");
        assert_eq!(seen[1], 0, "收尾轮工具必须剥离：{seen:?}");
        assert!(summary.contains("最终综合结论"), "预算耗尽也必须拿到综合结论：{summary}");
        assert!(summary.contains("工具预算（100 次）"), "预算截断标注应透传父模型：{summary}");
        kernel.shutdown().await.unwrap();
    }

    /// 永不输出永不返回的探针——模拟卡死的模型连接，喂给看门狗。
    struct SilentProbeProvider;

    impl LlmProvider for SilentProbeProvider {
        fn stream<'a>(
            self: Arc<Self>,
            _config: &'a ChatConfig,
            _messages: &'a [crate::domain::ChatMessage],
            _tools: &'a [crate::domain::ToolSpec],
            _cancel: &'a mut watch::Receiver<bool>,
            _events: tokio::sync::mpsc::UnboundedSender<crate::adapters::llm::StreamEvent>,
        ) -> futures_util::future::BoxFuture<'a, AppResult<crate::adapters::llm::TurnOutcome>> {
            Box::pin(async move { std::future::pending::<AppResult<crate::adapters::llm::TurnOutcome>>().await })
        }
    }

    #[tokio::test]
    async fn silent_stream_is_aborted_by_the_idle_watchdog() {
        // 5s 是 env 下限——探针零输出，看门狗必须在此处而非总超时（1800s）处决。
        // env 进程级共享但不影响并行测试：其余探针即时响应，远够不到静默阈值。
        unsafe { std::env::set_var("MAESTRO_SUBAGENT_IDLE_TIMEOUT", "5") };
        let (kernel, _) = test_kernel().await;
        let probe = Arc::new(SilentProbeProvider);
        *kernel.llm_resolver.lock().unwrap() = Arc::new(move |_| probe.clone());
        save_capped_explore(&kernel, Some(8), None).await;
        let started = std::time::Instant::now();
        let outcome = drive_typed_subagent(&kernel, "capped-explore", &test_config()).await;
        let elapsed = started.elapsed().as_secs();
        // 连续静默=重排队一次仍静默后才诚实失败（Err 上抛，父级拿诊断原文）。
        let error = outcome.unwrap_err();
        assert!(error.contains("静默"), "看门狗诊断应如实标注：{error}");
        // 重试语义下耗时约 2×5s+，仍须远早于总超时。
        assert!(elapsed < 60, "看门狗必须在总超时前处决（实际 {elapsed}s）");
        assert!(elapsed >= 9, "首次静默应重排队一轮而非立即杀死（实际 {elapsed}s）");
        kernel.shutdown().await.unwrap();
    }

    /// 首次调用永久静默、重试后正常的探针：验证 idle 重试能救活排队静默，
    /// 且 messages/轮次工作保留（子智能体最终正常完工而非失败）。
    struct FlakeySilentProbe {
        calls: std::sync::atomic::AtomicUsize,
    }

    impl LlmProvider for FlakeySilentProbe {
        fn stream<'a>(
            self: Arc<Self>,
            _config: &'a ChatConfig,
            _messages: &'a [crate::domain::ChatMessage],
            _tools: &'a [crate::domain::ToolSpec],
            _cancel: &'a mut watch::Receiver<bool>,
            events: tokio::sync::mpsc::UnboundedSender<crate::adapters::llm::StreamEvent>,
        ) -> futures_util::future::BoxFuture<'a, AppResult<crate::adapters::llm::TurnOutcome>> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Box::pin(async move {
                use crate::adapters::llm::{StreamEvent, TurnOutcome};
                if call == 0 {
                    return std::future::pending::<AppResult<TurnOutcome>>().await;
                }
                let _ = events.send(StreamEvent::Token("重试后完工".into()));
                let _ = events.send(StreamEvent::Done);
                Ok(TurnOutcome {
                    done: true,
                    tool_calls: Vec::new(),
                    content: "重试后完工".into(),
                    thinking: None,
                    finish_reason: None,
                    dropped_tool_calls: 0,
                    input_tokens: None,
                })
            })
        }
    }

    #[tokio::test]
    async fn idle_stream_is_retried_once_and_can_recover() {
        unsafe { std::env::set_var("MAESTRO_SUBAGENT_IDLE_TIMEOUT", "5") };
        let (kernel, _) = test_kernel().await;
        let probe = Arc::new(FlakeySilentProbe { calls: std::sync::atomic::AtomicUsize::new(0) });
        *kernel.llm_resolver.lock().unwrap() = Arc::new(move |_| probe.clone());
        save_capped_explore(&kernel, Some(8), None).await;
        let started = std::time::Instant::now();
        let summary = drive_typed_subagent(&kernel, "capped-explore", &test_config()).await.unwrap();
        let elapsed = started.elapsed().as_secs();
        assert!(summary.contains("重试后完工"), "重试后续跑应完工：{summary}");
        assert!(elapsed >= 5, "必须等满一个静默周期才重试（实际 {elapsed}s）");
        assert!(elapsed < 30, "恢复后不应拖延（实际 {elapsed}s）");
        kernel.shutdown().await.unwrap();
    }

    /// 持续产出的流绝不应被看门狗触碰：每 1.5s 一个 token（慢于 drain 但稳
    /// 定），总时长跨越两个静默周期。验证转发任务链路完整——事件经 导线tap
    /// →drain 全通且 activity 被持续喂饱。
    struct TrickleProbe;

    impl LlmProvider for TrickleProbe {
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
                for i in 0..8 {
                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    let piece = if i == 7 { "细流完工" } else { "滴" };
                    if events.send(StreamEvent::Token(piece.into())).is_err() {
                        break;
                    }
                }
                let _ = events.send(StreamEvent::Done);
                Ok(TurnOutcome {
                    done: true,
                    tool_calls: Vec::new(),
                    content: "细流完工".into(),
                    thinking: None,
                    finish_reason: None,
                    dropped_tool_calls: 0,
                    input_tokens: None,
                })
            })
        }
    }

    #[tokio::test]
    async fn steadily_producing_stream_is_never_touched_by_the_watchdog() {
        unsafe { std::env::set_var("MAESTRO_SUBAGENT_IDLE_TIMEOUT", "5") };
        let (kernel, _) = test_kernel().await;
        let probe = Arc::new(TrickleProbe);
        *kernel.llm_resolver.lock().unwrap() = Arc::new(move |_| probe.clone());
        save_capped_explore(&kernel, Some(8), None).await;
        let summary = drive_typed_subagent(&kernel, "capped-explore", &test_config()).await.unwrap();
        assert!(summary.contains("细流完工"), "持续产出必须存活到完工：{summary}");
        kernel.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn subagent_compacts_midflight_when_context_fills() {
        let (kernel, _) = test_kernel().await;
        let provider = Arc::new(SynthProbeProvider {
            seen: StdMutex::new(Vec::new()),
            counter: std::sync::atomic::AtomicUsize::new(0),
            fanout: 1,
        });
        let probe = provider.clone();
        *kernel.llm_resolver.lock().unwrap() = Arc::new(move |_| probe.clone());
        // 极小上下文窗口：几轮累积即越线——压缩必须中途接管而非让子智能体
        // 死于溢出。压缩记录落库是最硬的可观测断言面。
        let mut config = test_config();
        config.context_window = Some(400);
        // 探针永续吐工具调用——无硬顶下循环永不自然终止，须显式上限保证测试收口；
        // 压缩是否中途接管不受上限影响（窗口 400 几轮即越线）。
        save_capped_explore(&kernel, Some(6), None).await;
        let summary = drive_typed_subagent(&kernel, "capped-explore", &config).await.unwrap();
        let session_id = kernel.database.list_runs().await.unwrap()[0].session_id;
        let compactions = kernel.database.list_compactions(session_id).await.unwrap();
        assert!(
            compactions.iter().any(|c| c.status == "completed"),
            "子智能体循环必须接线压缩并落库：{compactions:?}"
        );
        assert!(summary.contains("最终综合结论"), "压缩后续跑仍应收尾：{summary}");
        kernel.shutdown().await.unwrap();
    }
}
