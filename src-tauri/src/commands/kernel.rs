use std::sync::Arc;

use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::{
    adapters::{fs::workspace::WorkspaceService, llm::{ChatConfig, Provider}},
    commands::require_main,
    domain::{AppResult, ApprovalDecision, ApprovalRule, ChatMessage, CommandScope, Mode, QuestionAnswer, RunView, Snapshot, SubagentView},
    services::kernel::Kernel,
};

#[tauri::command]
pub async fn get_snapshot(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
) -> AppResult<Snapshot> {
    require_main(&window)?;
    Ok(kernel.snapshot().await)
}

#[tauri::command]
pub async fn set_mode(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    mode: Mode,
) -> AppResult<()> {
    require_main(&window)?;
    kernel.set_mode(mode).await
}

#[tauri::command(rename_all = "snake_case")]
// Tauri injects handles/state in addition to the explicit IPC arguments.
#[allow(clippy::too_many_arguments)]
pub async fn stream_chat(
    app: AppHandle,
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    workspace: State<'_, WorkspaceService>,
    messages: Vec<ChatMessage>,
    base_url: String,
    model: String,
    session_id: Option<i64>,
    provider: Option<Provider>,
    // Profile id the key was stored under; custom endpoints share the OpenAI
    // wire format but keep their own credential.
    credential_provider: Option<String>,
    reasoning_effort: Option<String>,
    max_output: Option<u64>,
    context_window: Option<u64>,
    vision_capable: Option<bool>,
) -> AppResult<i64> {
    if window.label() != "main" || app.get_webview_window("main").is_none() {
        return Err("Only the main window may start a chat".into());
    }
    let provider = provider.unwrap_or_default();
    // The key is resolved from the OS-protected store, never from the request.
    let api_key = kernel
        .credentials()
        .get(credential_provider.as_deref().unwrap_or_else(|| provider.name()))?
        .unwrap_or_default();
    let config = ChatConfig {
        api_key,
        base_url,
        model,
        provider,
        reasoning: reasoning_effort,
        max_output,
        context_window,
        // The kernel stamps a stable per-session id before the run starts.
        session: None,
        // 前端模型能力表解析（后端无模型目录）；缺省保守按无视觉处理。
        vision_capable: vision_capable.unwrap_or(false),
    };
    kernel
        .set_workspace_root(workspace.current_root())
        .await;
    kernel.inner().start_chat(session_id, messages, config).await
}

/// Whether the live workspace carries untrusted `.maestro/` assets — the
/// trust prompt reloads this on mount since the event does not replay.
#[tauri::command]
pub async fn workspace_trust_status(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    workspace: State<'_, WorkspaceService>,
) -> AppResult<Option<crate::domain::WorkspaceTrustStatus>> {
    require_main(&window)?;
    Ok(kernel
        .workspace_trust_status(workspace.current_root().as_deref())
        .await)
}

/// Grant trust for the workspace path: project `.maestro/` assets start
/// loading on the next scan. Recorded per canonical path.
#[tauri::command]
pub async fn trust_workspace(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    path: String,
) -> AppResult<()> {
    require_main(&window)?;
    kernel.trust_workspace(path).await
}

#[tauri::command]
pub async fn cancel_agent(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    run_id: i64,
) -> AppResult<()> {
    require_main(&window)?;
    kernel.cancel(run_id).await
}

/// Hard-stop an in-flight run: cancels the watch channel so the provider's
/// SSE loop and the event relay both break immediately.
#[tauri::command]
pub async fn abort_chat(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    run_id: i64,
) -> AppResult<()> {
    require_main(&window)?;
    kernel.cancel(run_id).await
}

/// Approve or reject the active run's suspended tool call. The kernel only
/// accepts this while a call is actually waiting — replayed or stale
/// decisions error instead of being silently applied later.
#[tauri::command]
pub async fn resolve_tool_approval(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    run_id: i64,
    decision: ApprovalDecision,
    command_scope: Option<CommandScope>,
    feedback: Option<String>,
) -> AppResult<()> {
    require_main(&window)?;
    kernel
        .resolve_approval(run_id, decision, command_scope.unwrap_or_default(), feedback)
        .await
}

/// Session-scoped "always allow" rules currently in force. Rules are derived
/// from exact command/argument identities by default; the UI shows them so a
/// wide (prefix) rule can be reviewed and revoked.
#[tauri::command]
pub async fn list_approval_rules(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    run_id: i64,
) -> AppResult<Vec<ApprovalRule>> {
    require_main(&window)?;
    kernel.list_approval_rules(run_id).await
}

#[tauri::command]
pub async fn revoke_approval_rule(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    run_id: i64,
    tool: String,
    pattern: String,
    prefix: bool,
) -> AppResult<bool> {
    require_main(&window)?;
    kernel
        .revoke_approval_rule(run_id, &ApprovalRule { tool, pattern, prefix })
        .await
}

#[tauri::command]
pub async fn list_runs(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
) -> AppResult<Vec<RunView>> {
    require_main(&window)?;
    kernel.list_runs().await
}

#[tauri::command]
pub async fn list_subagent_traces(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    session_id: i64,
) -> AppResult<Vec<SubagentView>> {
    require_main(&window)?;
    kernel.list_subagent_traces(session_id).await
}

#[tauri::command]
pub async fn cancel_subagent(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    run_id: i64,
    subagent_id: String,
) -> AppResult<()> {
    require_main(&window)?;
    kernel.cancel_subagent(run_id, &subagent_id).await
}

#[tauri::command]
pub async fn answer_user_question(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    run_id: i64,
    request_id: String,
    answers: Vec<QuestionAnswer>,
) -> AppResult<()> {
    require_main(&window)?;
    kernel.answer_user_question(run_id, &request_id, answers).await
}

#[tauri::command]
pub async fn compact_session(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    session_id: i64,
) -> AppResult<()> {
    require_main(&window)?;
    kernel.compact_session(session_id).await
}
