use std::sync::Arc;

use tauri::{State, WebviewWindow};

use crate::{
    adapters::llm::{CatalogModel, ChatConfig, FetchedModel, Provider},
    commands::require_main,
    domain::AppResult,
    services::kernel::Kernel,
};

/// List a provider's models. `api_key` is only used for a key the user has just
/// typed and not saved yet; otherwise the stored credential is resolved here so
/// the webview never needs to hold the plaintext key.
#[tauri::command]
pub async fn fetch_models(
    window: WebviewWindow,
    kernel: State<'_, Arc<Kernel>>,
    provider: String,
    base_url: String,
    api_key: Option<String>,
    credential_provider: Option<String>,
) -> AppResult<Vec<FetchedModel>> {
    require_main(&window)?;
    let provider = serde_json::from_value::<Provider>(serde_json::Value::String(provider))
        .map_err(|_| "未知 Provider".to_string())?;
    let api_key = match api_key.map(|value| value.trim().to_string()).filter(|value| !value.is_empty()) {
        Some(draft) => draft,
        None => kernel
            .credentials()
            .get(credential_provider.as_deref().unwrap_or_else(|| provider.name()))?
            .unwrap_or_default(),
    };
    crate::adapters::llm::fetch_models(&ChatConfig {
        api_key,
        base_url,
        model: String::new(),
        provider,
        reasoning: None,
        max_output: None,
        context_window: None,
        session: None,
        vision_capable: false,
    })
    .await
}

#[tauri::command]
pub async fn model_catalog(window: WebviewWindow) -> AppResult<Vec<CatalogModel>> {
    require_main(&window)?;
    crate::adapters::llm::model_catalog().await
}
