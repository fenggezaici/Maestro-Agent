use tauri::WebviewWindow;

use crate::{
    adapters::llm::{CatalogModel, ChatConfig, FetchedModel, Provider},
    commands::require_main,
    domain::AppResult,
};

#[tauri::command]
pub async fn fetch_models(
    window: WebviewWindow,
    provider: String,
    base_url: String,
    api_key: String,
) -> AppResult<Vec<FetchedModel>> {
    require_main(&window)?;
    let provider = serde_json::from_value::<Provider>(serde_json::Value::String(provider))
        .map_err(|_| "未知 Provider".to_string())?;
    crate::adapters::llm::fetch_models(&ChatConfig {
        api_key,
        base_url,
        model: String::new(),
        provider,
        reasoning: None,
        max_output: None,
        session: None,
    })
    .await
}

#[tauri::command]
pub async fn model_catalog(window: WebviewWindow) -> AppResult<Vec<CatalogModel>> {
    require_main(&window)?;
    crate::adapters::llm::model_catalog().await
}
