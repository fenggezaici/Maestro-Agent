//! git 只读感知 IPC：cwd 一律由后端 WorkspaceService 解析——前端不传路径。
use tauri::{State, WebviewWindow};

use crate::{
    adapters::{
        fs::workspace::WorkspaceService,
        git::{self, GitCommit, GitDiff, GitStatus, GitSummary},
    },
    commands::require_main,
};

fn missing_workspace() -> String {
    "未选择工作区".to_string()
}

#[tauri::command]
pub async fn git_summary(
    window: WebviewWindow,
    workspace: State<'_, WorkspaceService>,
) -> Result<GitSummary, String> {
    require_main(&window)?;
    let root = workspace.current_root().ok_or_else(missing_workspace)?;
    git::summary(&root).await
}

#[tauri::command]
pub async fn git_status(
    window: WebviewWindow,
    workspace: State<'_, WorkspaceService>,
) -> Result<Option<GitStatus>, String> {
    require_main(&window)?;
    let root = workspace.current_root().ok_or_else(missing_workspace)?;
    git::status(&root).await
}

#[tauri::command]
pub async fn git_diff(
    window: WebviewWindow,
    workspace: State<'_, WorkspaceService>,
    path: String,
    source: String,
) -> Result<Option<GitDiff>, String> {
    require_main(&window)?;
    let root = workspace.current_root().ok_or_else(missing_workspace)?;
    // path 只做 `--` 后的路径参数，绝不进 shell——无注入面。
    git::diff(&root, &path, &source).await
}

#[tauri::command]
pub async fn git_log(
    window: WebviewWindow,
    workspace: State<'_, WorkspaceService>,
    limit: Option<usize>,
    skip: Option<usize>,
) -> Result<Option<Vec<GitCommit>>, String> {
    require_main(&window)?;
    let root = workspace.current_root().ok_or_else(missing_workspace)?;
    git::log(&root, limit.unwrap_or(200), skip.unwrap_or(0)).await
}
