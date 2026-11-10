use std::sync::Arc;

use tauri::{State, WebviewWindow};

use crate::{
    commands::require_main,
    domain::{AppResult, Mode, RunView, Snapshot},
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

#[tauri::command]
pub async fn start_agent(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    session_id: i64,
    prompt: String,
) -> AppResult<i64> {
    require_main(&window)?;
    kernel.inner().start_in_session(session_id, prompt).await
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

#[tauri::command]
pub async fn list_runs(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
) -> AppResult<Vec<RunView>> {
    require_main(&window)?;
    kernel.list_runs().await
}
