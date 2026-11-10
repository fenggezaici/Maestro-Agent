use std::sync::Arc;

use tauri::{State, WebviewWindow};

use crate::{
    commands::require_main,
    domain::{
        AppResult, PlanCard, PlanDecision, ProjectView, SessionDetail, SessionView,
    },
    services::kernel::Kernel,
};

#[tauri::command]
pub async fn create_session(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    project_id: Option<i64>,
) -> AppResult<SessionView> {
    require_main(&window)?;
    kernel.create_session(project_id).await
}

#[tauri::command]
pub async fn set_session_archived(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    session_id: i64,
    archived: bool,
) -> AppResult<SessionView> {
    require_main(&window)?;
    kernel.set_session_archived(session_id, archived).await
}

#[tauri::command]
pub async fn rename_session(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    session_id: i64,
    title: String,
) -> AppResult<SessionView> {
    require_main(&window)?;
    kernel.rename_session(session_id, title).await
}

#[tauri::command]
pub async fn delete_session(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    session_id: i64,
) -> AppResult<()> {
    require_main(&window)?;
    kernel.delete_session(session_id).await
}

#[tauri::command]
pub async fn list_projects(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
) -> AppResult<Vec<ProjectView>> {
    require_main(&window)?;
    kernel.list_projects().await
}

#[tauri::command]
pub async fn create_project(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    name: String,
    path: Option<String>,
) -> AppResult<ProjectView> {
    require_main(&window)?;
    kernel.create_project(name, path).await
}

#[tauri::command]
pub async fn set_project_archived(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    project_id: i64,
    archived: bool,
) -> AppResult<ProjectView> {
    require_main(&window)?;
    kernel.set_project_archived(project_id, archived).await
}

#[tauri::command]
pub async fn update_project_name(
    kernel: tauri::State<'_, Kernel>,
    project_id: i64,
    name: String,
) -> Result<(), String> {
    kernel.update_project_name(project_id, name).await
}

#[tauri::command]
pub async fn update_project_path(
    kernel: tauri::State<'_, Kernel>,
    project_id: i64,
    path: String,
) -> Result<(), String> {
    kernel.update_project_path(project_id, path).await
}

#[tauri::command]
pub async fn delete_project(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    project_id: i64,
) -> AppResult<()> {
    require_main(&window)?;
    kernel.delete_project(project_id).await
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
