use std::sync::Arc;

use tauri::{State, WebviewWindow};

use crate::{
    adapters::fs::workspace::WorkspaceService,
    commands::require_main,
    domain::{
        AppResult, CommandView, McpServerConfig, McpServerView, RecommendedItem,
        SkillImportOutcome, SkillSource, SkillView,
    },
    services::kernel::Kernel,
};

/// Resolve the skills root for a scope; project scope needs a selected workspace.
fn skill_root(
    maestro_dir: &std::path::Path,
    workspace: Option<&std::path::Path>,
    scope: &str,
) -> AppResult<(std::path::PathBuf, SkillSource)> {
    match scope {
        "project" => {
            let root = workspace.ok_or("未选择工作区")?;
            Ok((
                root.join(".maestro").join("skills"),
                SkillSource::Project,
            ))
        }
        _ => Ok((maestro_dir.join("skills"), SkillSource::Global)),
    }
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
    workspace: State<'_, WorkspaceService>,
    name: String,
    description: String,
    scope: Option<String>,
) -> AppResult<SkillView> {
    require_main(&window)?;
    let root = workspace.current_root();
    let (skills_dir, source) = skill_root(
        kernel.extensions().maestro_dir(),
        root.as_deref(),
        scope.as_deref().unwrap_or("global"),
    )?;
    kernel
        .extensions()
        .skills
        .create_new(&skills_dir, &name, &description, source)
}

#[tauri::command]
pub async fn delete_skill(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    workspace: State<'_, WorkspaceService>,
    name: String,
    scope: Option<String>,
) -> AppResult<()> {
    require_main(&window)?;
    let root = workspace.current_root();
    let (skills_dir, _source) = skill_root(
        kernel.extensions().maestro_dir(),
        root.as_deref(),
        scope.as_deref().unwrap_or("global"),
    )?;
    kernel.extensions().skills.delete(&skills_dir, &name)
}

#[tauri::command]
pub async fn scan_commands(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    workspace: State<'_, WorkspaceService>,
) -> AppResult<Vec<CommandView>> {
    require_main(&window)?;
    let root = workspace.current_root();
    Ok(crate::adapters::commands::scan(
        kernel.extensions().maestro_dir(),
        root.as_deref(),
    ))
}

#[tauri::command]
pub async fn read_command(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    workspace: State<'_, WorkspaceService>,
    name: String,
) -> AppResult<String> {
    require_main(&window)?;
    let root = workspace.current_root();
    crate::adapters::commands::read(
        kernel.extensions().maestro_dir(),
        root.as_deref(),
        &name,
    )
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
    env: Option<std::collections::BTreeMap<String, String>>,
    trust: Option<bool>,
) -> AppResult<()> {
    require_main(&window)?;
    kernel.extensions().mcp.upsert_server(
        &name,
        McpServerConfig {
            command,
            args: args.unwrap_or_default(),
            env: env.unwrap_or_default(),
            disabled: false,
            auto_start: false,
            trust: trust.unwrap_or(false),
        },
    )
}

#[tauri::command]
pub async fn set_mcp_server_trust(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    name: String,
    trust: bool,
) -> AppResult<()> {
    require_main(&window)?;
    kernel.extensions().mcp.set_trust(&name, trust).await
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
    kernel.extensions().mcp.enable(&name).await
}

#[tauri::command]
pub async fn stop_mcp_server(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    name: String,
) -> AppResult<McpServerView> {
    require_main(&window)?;
    kernel.extensions().mcp.disable(&name).await
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
    Ok(crate::adapters::skills::recommended_skills())
}

#[tauri::command]
pub async fn recommended_mcps(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
) -> AppResult<Vec<RecommendedItem>> {
    require_main(&window)?;
    let _ = kernel;
    Ok(crate::adapters::mcp::recommended_mcps())
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
