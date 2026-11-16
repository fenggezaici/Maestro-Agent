use std::sync::Arc;
use tauri::{State, WebviewWindow};
use crate::{commands::require_main, domain::{AgentDefinition, AppResult}, services::kernel::Kernel};

#[tauri::command]
pub async fn list_agents(window: WebviewWindow, kernel: State<'_, Arc<Kernel>>, scope: String) -> AppResult<Vec<AgentDefinition>> {
    require_main(&window)?;
    kernel.list_agents(scope).await
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
