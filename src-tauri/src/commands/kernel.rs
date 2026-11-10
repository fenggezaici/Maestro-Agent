use std::sync::Arc;

use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::{
    adapters::{fs::workspace::WorkspaceService, llm::{ChatConfig, Provider}},
    commands::require_main,
    domain::{AppResult, ChatMessage, Mode, RunView, Snapshot},
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
    api_key: String,
    base_url: String,
    model: String,
    session_id: Option<i64>,
    provider: Option<Provider>,
    reasoning_effort: Option<String>,
    max_output: Option<u64>,
) -> AppResult<i64> {
    if window.label() != "main" || app.get_webview_window("main").is_none() {
        return Err("Only the main window may start a chat".into());
    }
    let config = ChatConfig {
        api_key,
        base_url,
        model,
        provider: provider.unwrap_or_default(),
        reasoning: reasoning_effort,
        max_output,
    };
    kernel
        .set_workspace_root(workspace.current_root())
        .await;
    kernel.inner().start_chat(session_id, messages, config).await
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
    cancel_agent(window, kernel, run_id).await
}

#[tauri::command]
pub async fn list_runs(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
) -> AppResult<Vec<RunView>> {
    require_main(&window)?;
    kernel.list_runs().await
}
