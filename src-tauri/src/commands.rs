use std::sync::Arc;

use tauri::{State, WebviewWindow};

use crate::{
    kernel::Kernel,
    model::{
        AppResult, Mode, PlanCard, PlanDecision, RunView, SessionDetail, SessionView, Snapshot,
    },
};

// Defense in depth in addition to local-window capabilities; callers supply no paths or SQL.
fn require_main(window: &WebviewWindow) -> AppResult<()> {
    if window.label() != "main" {
        return Err("Only the main window may invoke backend commands".into());
    }
    Ok(())
}

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

#[tauri::command]
pub async fn create_session(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
) -> AppResult<SessionView> {
    require_main(&window)?;
    kernel.create_session().await
}

#[tauri::command]
pub async fn list_sessions(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
) -> AppResult<Vec<SessionView>> {
    require_main(&window)?;
    kernel.list_sessions().await
}

#[tauri::command]
pub async fn get_session(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    session_id: i64,
) -> AppResult<SessionDetail> {
    require_main(&window)?;
    kernel.session_detail(session_id).await
}

#[tauri::command]
pub async fn decide_plan(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    session_id: i64,
    plan_id: i64,
    decision: PlanDecision,
) -> AppResult<PlanCard> {
    require_main(&window)?;
    kernel.decide_plan(session_id, plan_id, decision).await
}
