use std::sync::Arc;
use tauri::{State, WebviewWindow};
use crate::{commands::require_main, domain::{AgentDefinition, AgentImportOutcome, AgentItemRef, AppResult, SubagentDefaults}, services::kernel::Kernel};

#[tauri::command]
pub async fn list_agents(window: WebviewWindow, kernel: State<'_, Arc<Kernel>>, scope: String) -> AppResult<Vec<AgentDefinition>> {
    require_main(&window)?;
    kernel.list_agents(scope).await
}

/// 批量导入本地目录中的 `*.md` 为全局 Agent（agency-agents-zh 类格式转换）。
#[tauri::command]
pub async fn import_agents_dir(window: WebviewWindow, kernel: State<'_, Arc<Kernel>>, path: String) -> AppResult<Vec<AgentImportOutcome>> {
    require_main(&window)?;
    kernel.import_agents_dir(path).await
}

/// 浅克隆 GitHub 仓库后批量导入其中全部 Agent 定义。
#[tauri::command]
pub async fn import_agents_github(window: WebviewWindow, kernel: State<'_, Arc<Kernel>>, repo_url: String) -> AppResult<Vec<AgentImportOutcome>> {
    require_main(&window)?;
    kernel.import_agents_github(repo_url).await
}

#[tauri::command]
pub async fn save_agent(window: WebviewWindow, kernel: State<'_, Arc<Kernel>>, scope: String, agent: AgentDefinition) -> AppResult<()> {
    require_main(&window)?;
    kernel.save_agent(scope, agent).await
}

#[tauri::command]
pub async fn delete_agent(window: WebviewWindow, kernel: State<'_, Arc<Kernel>>, scope: String, name: String) -> AppResult<()> {
    require_main(&window)?;
    kernel.delete_agent(scope, name).await
}

/// 批量启用/禁用：hidden 即禁用语义，逐项三态。
#[tauri::command]
pub async fn set_agents_hidden(window: WebviewWindow, kernel: State<'_, Arc<Kernel>>, items: Vec<AgentItemRef>, hidden: bool) -> AppResult<Vec<AgentImportOutcome>> {
    require_main(&window)?;
    kernel.set_agents_hidden(items, hidden).await
}

/// 批量删除：逐项三态；builtin 记 failed。
#[tauri::command]
pub async fn delete_agents(window: WebviewWindow, kernel: State<'_, Arc<Kernel>>, items: Vec<AgentItemRef>) -> AppResult<Vec<AgentImportOutcome>> {
    require_main(&window)?;
    kernel.delete_agents(items).await
}

/// 全局子智能体默认（模型/轮次/预算/并行/总额度）；per-agent 字段留空时继承。
#[tauri::command]
pub async fn get_agent_defaults(window: WebviewWindow, kernel: State<'_, Arc<Kernel>>) -> AppResult<SubagentDefaults> {
    require_main(&window)?;
    kernel.agent_defaults()
}

#[tauri::command]
pub async fn save_agent_defaults(window: WebviewWindow, kernel: State<'_, Arc<Kernel>>, defaults: SubagentDefaults) -> AppResult<()> {
    require_main(&window)?;
    kernel.save_agent_defaults(defaults).await
}
