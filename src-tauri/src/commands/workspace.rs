use tauri::{State, WebviewWindow};

use crate::{
    adapters::fs::workspace::{FileNode, WorkspaceInfo, WorkspaceService},
    commands::require_main,
    domain::AppResult,
};

#[tauri::command]
pub async fn get_workspace_info(
    window: WebviewWindow,
    service: State<'_, WorkspaceService>,
) -> Result<WorkspaceInfo, String> {
    require_main(&window)?;
    service.info().await
}

#[tauri::command]
pub async fn select_workspace(
    window: WebviewWindow,
    service: State<'_, WorkspaceService>,
    workspace_path: String,
) -> Result<WorkspaceInfo, String> {
    require_main(&window)?;
    service.select(workspace_path).await
}

#[tauri::command]
pub async fn read_directory_tree(
    window: WebviewWindow,
    service: State<'_, WorkspaceService>,
    workspace_path: String,
) -> Result<Vec<FileNode>, String> {
    require_main(&window)?;
    service.tree(workspace_path).await
}

#[tauri::command]
pub async fn read_workspace_text(
    window: WebviewWindow,
    service: State<'_, WorkspaceService>,
    workspace_path: String,
    path: String,
) -> Result<String, String> {
    require_main(&window)?;
    service.text(workspace_path, path).await
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
