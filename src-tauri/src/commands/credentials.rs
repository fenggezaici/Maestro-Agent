//! Provider credential IPC. The key travels once from the settings form and is
//! stored protected for the current OS user; nothing returns the plaintext key
//! to the webview, so the frontend only ever learns whether one is configured.
use std::sync::Arc;

use tauri::{State, WebviewWindow};

use crate::{commands::require_main, domain::AppResult, services::kernel::Kernel};

#[tauri::command(rename_all = "snake_case")]
pub async fn save_provider_credential(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    provider: String,
    api_key: String,
) -> AppResult<()> {
    require_main(&window)?;
    kernel.credentials().save(&provider, &api_key)
}

#[tauri::command]
pub async fn provider_credential_status(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
) -> AppResult<Vec<String>> {
    require_main(&window)?;
    Ok(kernel.credentials().status())
}

#[tauri::command]
pub async fn clear_provider_credential(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    provider: String,
) -> AppResult<()> {
    require_main(&window)?;
    kernel.credentials().clear(&provider)
}
