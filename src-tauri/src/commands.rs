use std::sync::Arc;

use tauri::{State, WebviewWindow};

use crate::{
    extensions::{
        mcp::{McpServerConfig, McpServerView},
        skills::{RecommendedItem, SkillImportOutcome, SkillView},
    },
    kernel::Kernel,
    model::{
        AppResult, Mode, PlanCard, PlanDecision, ProjectView, RunView, SessionDetail, SessionView,
        Snapshot,
    },
    workspace::WorkspaceService,
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

// ---- Extensions: skills + MCP servers -----------------------------------

#[tauri::command]
pub async fn fetch_models(
    window: WebviewWindow,
    provider: String,
    base_url: String,
    api_key: String,
) -> AppResult<Vec<crate::llm::FetchedModel>> {
    require_main(&window)?;
    let provider = serde_json::from_value::<crate::llm::Provider>(serde_json::Value::String(provider))
        .map_err(|_| "未知 Provider".to_string())?;
    crate::llm::fetch_models(&crate::llm::ChatConfig {
        api_key,
        base_url,
        model: String::new(),
        provider,
        reasoning: None,
    })
    .await
}

#[tauri::command]
pub async fn model_catalog(window: WebviewWindow) -> AppResult<Vec<crate::llm::CatalogModel>> {
    require_main(&window)?;
    crate::llm::model_catalog().await
}

#[tauri::command]
pub async fn scan_skills(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    workspace: State<'_, WorkspaceService>,
) -> AppResult<Vec<SkillView>> {
    require_main(&window)?;
    let root = workspace.current_root();
    let dir = kernel.extensions().maestro_dir().to_path_buf();
    Ok(kernel.extensions().skills.scan(&dir, root.as_deref()))
}

#[tauri::command]
pub async fn create_skill(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    name: String,
    description: String,
) -> AppResult<SkillView> {
    require_main(&window)?;
    kernel.extensions().skills.create_new(
        kernel.extensions().maestro_dir(),
        &name,
        &description,
    )
}

#[tauri::command]
pub async fn pick_directory(window: WebviewWindow) -> AppResult<Option<String>> {
    require_main(&window)?;
    Ok(rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn pick_file(window: WebviewWindow, extensions: Vec<String>) -> AppResult<Option<String>> {
    require_main(&window)?;
    let mut dialog = rfd::FileDialog::new();
    if !extensions.is_empty() {
        dialog = dialog.add_filter("files", &extensions);
    }
    Ok(dialog
        .pick_file()
        .map(|path| path.to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn import_skill(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    path: String,
) -> AppResult<Vec<SkillImportOutcome>> {
    require_main(&window)?;
    kernel
        .extensions()
        .skills
        .import_from_path(kernel.extensions().maestro_dir(), &path)
}

#[tauri::command]
pub async fn import_skill_github(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    repo_url: String,
) -> AppResult<Vec<SkillView>> {
    require_main(&window)?;
    kernel
        .extensions()
        .skills
        .import_from_github(kernel.extensions().maestro_dir(), &repo_url)
        .await
}

#[tauri::command]
pub async fn list_mcp_servers(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
) -> AppResult<Vec<McpServerView>> {
    require_main(&window)?;
    kernel.extensions().mcp.list_servers().await
}

#[tauri::command]
pub async fn add_mcp_server(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    name: String,
    command: String,
    args: Option<Vec<String>>,
) -> AppResult<()> {
    require_main(&window)?;
    kernel.extensions().mcp.upsert_server(
        &name,
        McpServerConfig {
            command,
            args: args.unwrap_or_default(),
            env: Default::default(),
            disabled: false,
        },
    )
}

#[tauri::command]
pub async fn import_mcp_json(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    path: String,
) -> AppResult<usize> {
    require_main(&window)?;
    kernel.extensions().mcp.import_json(&path)
}

#[tauri::command]
pub async fn start_mcp_server(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    name: String,
) -> AppResult<McpServerView> {
    require_main(&window)?;
    kernel.extensions().mcp.start(&name).await
}

#[tauri::command]
pub async fn stop_mcp_server(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    name: String,
) -> AppResult<()> {
    require_main(&window)?;
    kernel.extensions().mcp.stop(&name).await
}

#[tauri::command]
pub async fn remove_mcp_server(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    name: String,
) -> AppResult<()> {
    require_main(&window)?;
    kernel.extensions().mcp.remove_server(&name).await
}

#[tauri::command]
pub async fn recommended_skills(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
) -> AppResult<Vec<RecommendedItem>> {
    require_main(&window)?;
    let _ = kernel;
    Ok(crate::extensions::skills::recommended_skills())
}

#[tauri::command]
pub async fn recommended_mcps(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
) -> AppResult<Vec<RecommendedItem>> {
    require_main(&window)?;
    let _ = kernel;
    Ok(crate::extensions::mcp::recommended_mcps())
}

#[tauri::command]
pub async fn read_mcp_config(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
) -> AppResult<String> {
    require_main(&window)?;
    let path = kernel.extensions().maestro_dir().join("mcp.json");
    if !path.is_file() {
        return Ok(String::new());
    }
    std::fs::read_to_string(path).map_err(|_| "无法读取 mcp.json".to_string())
}
