use std::sync::Arc;

use tauri::{State, WebviewWindow};

use crate::{commands::require_main, domain::{ActivitySnapshot, AppResult, RevertPreview, RevertResult, RunChangeSummary, RunFileChange}, services::kernel::Kernel};

#[tauri::command(rename_all = "snake_case")]
pub async fn activity_snapshot(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    project_id: Option<i64>,
) -> AppResult<ActivitySnapshot> {
    require_main(&window)?;
    kernel.activity_snapshot(project_id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn run_changes(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    run_id: i64,
) -> AppResult<Vec<RunFileChange>> {
    require_main(&window)?;
    kernel.run_changes(run_id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn revert_run_changes(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    run_id: i64,
    session_id: i64,
) -> AppResult<RevertResult> {
    require_main(&window)?;
    kernel.revert_run_changes(run_id, session_id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn run_change_summary(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    run_id: i64,
) -> AppResult<RunChangeSummary> {
    require_main(&window)?;
    kernel.run_change_summary(run_id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn preview_revert_run_changes(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    run_id: i64,
) -> AppResult<RevertPreview> {
    require_main(&window)?;
    kernel.preview_revert_run_changes(run_id).await
}
