use std::sync::Arc;

use tauri::{State, WebviewWindow};

use crate::{
    adapters::fs::workspace::WorkspaceService,
    commands::require_main,
    domain::{AppResult, ArchitectureGraph},
    services::{
        architecture::{ArchitectureGraphView, GraphSaveResult, GraphScope},
        kernel::Kernel,
    },
};

/// 读指定作用域的规则图。`effective`（默认）解析为当前生效作用域
/// （项目文件存在 → project，否则 global，全无 → project 空视图）。
/// 项目级读取走受信任根：与运行时实际加载的规则集保持一致。
#[tauri::command]
pub async fn get_architecture_graph(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    workspace: State<'_, WorkspaceService>,
    scope: Option<String>,
) -> AppResult<ArchitectureGraphView> {
    require_main(&window)?;
    let trusted = kernel
        .trusted_project_root(workspace.current_root().as_deref())
        .await;
    let maestro = kernel.extensions().maestro_dir().to_path_buf();
    let resolved = match scope.as_deref().unwrap_or("effective") {
        "effective" => crate::services::architecture::effective_path(&maestro, trusted.as_deref())
            .map(|(scope, _)| scope)
            .unwrap_or(GraphScope::Project),
        raw => GraphScope::parse(raw)?,
    };
    crate::services::architecture::view_for(resolved, &maestro, trusted.as_deref())
}

/// 保存规则图：先过结构+DAG 校验再落盘；空画布删除该作用域规则并回退。
/// 写盘成功后刷新 ToolRouter 的规则缓存，立即可执行。
/// 与加载一致使用受信任根：未信任工作区的 `.maestro/` 规则本来就不会被
/// 运行时求值，在这里直接拒绝可避免「保存了却永不生效」的假配置。
#[tauri::command]
pub async fn save_architecture_graph(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    workspace: State<'_, WorkspaceService>,
    scope: String,
    graph: ArchitectureGraph,
) -> AppResult<GraphSaveResult> {
    require_main(&window)?;
    let scope = GraphScope::parse(&scope)?;
    let trusted = kernel
        .trusted_project_root(workspace.current_root().as_deref())
        .await;
    let result = crate::services::architecture::save(
        scope,
        kernel.extensions().maestro_dir(),
        trusted.as_deref(),
        &graph,
    )?;
    kernel.reload_architecture().await;
    Ok(result)
}
