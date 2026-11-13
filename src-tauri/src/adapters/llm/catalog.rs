//! Model catalog HTTP fetches: provider `/models` (and Ollama `/api/tags` +
//! `/api/show`) plus the public models.dev metadata catalog.
use std::time::Duration;

use reqwest::Url;

use crate::domain::AppResult;

use super::{is_opencode_host, ChatConfig, Provider};

#[derive(Debug, Clone, serde::Serialize)]
pub struct FetchedModel {
    pub id: String,
    pub context: Option<u64>,
}

/// Query the provider for its real model list. OpenAI-compatible providers expose
/// GET {base}/models (ids only — context stays None, never invented). Ollama uses
/// GET {base}/api/tags plus POST /api/show per model for a real context length.
pub async fn fetch_models(config: &ChatConfig) -> AppResult<Vec<FetchedModel>> {
    if config.api_key.trim().is_empty() && !matches!(config.provider, Provider::Ollama) {
        return Err("API Key 不能为空".into());
    }
    let base = Url::parse(config.base_url.trim()).map_err(|_| "Base URL 格式无效".to_string())?;
    if !matches!(base.scheme(), "http" | "https") || base.host_str().is_none() {
        return Err("Base URL 必须为 HTTP(S)".into());
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Maestro-Agent/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "无法初始化模型 HTTP 客户端".to_string())?;
    let root = format!("{}{}", base.origin().ascii_serialization(), base.path().trim_end_matches('/'));

    if matches!(config.provider, Provider::Ollama) {
        let response = client
            .get(format!("{root}/api/tags"))
            .send()
            .await
            .map_err(|_| "无法连接 Ollama，请确认服务已启动".to_string())?;
        if !response.status().is_success() {
            return Err(format!("Ollama 返回 HTTP {}", response.status().as_u16()));
        }
        let body: serde_json::Value = response.json().await.map_err(|_| "Ollama 响应无法解析".to_string())?;
        let ids: Vec<String> = body["models"]
            .as_array()
            .map(|items| items.iter().filter_map(|item| item["name"].as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let mut models = Vec::with_capacity(ids.len());
        for id in ids {
            // model_info holds e.g. "llama.context_length" — a real context window.
            let context = match client
                .post(format!("{root}/api/show"))
                .json(&serde_json::json!({ "name": id }))
                .send()
                .await
            {
                Ok(response) => response
                    .json::<serde_json::Value>()
                    .await
                    .ok()
                    .and_then(|body| {
                        body["model_info"].as_object().and_then(|info| {
                            info.iter()
                                .find(|(key, _)| key.ends_with("context_length"))
                                .and_then(|(_, value)| value.as_u64())
                        })
                    }),
                Err(_) => None,
            };
            models.push(FetchedModel { id, context });
        }
        return Ok(models);
    }

    let mut request = client.get(format!("{root}/models"));
    if is_opencode_host(&config.base_url) {
        request = request.header("x-opencode-session", "maestro-model-catalog");
    }
    if let Ok(value) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", config.api_key.trim())) {
        request = request.header(reqwest::header::AUTHORIZATION, value);
    }
    let response = request.send().await.map_err(|_| "无法连接 Provider，检查 Base URL 与网络".to_string())?;
    if !response.status().is_success() {
        return Err(format!("Provider 返回 HTTP {}", response.status().as_u16()));
    }
    let body: serde_json::Value = response.json().await.map_err(|_| "模型列表响应无法解析".to_string())?;
    Ok(body["data"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["id"].as_str().map(str::to_string))
                .map(|id| FetchedModel { id, context: None })
                .collect()
        })
        .unwrap_or_default())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CatalogModel {
    pub id: String,
    pub context: Option<u64>,
    pub reasoning: bool,
    pub reasoning_levels: Vec<String>,
}

/// Fetch the public models.dev catalog (same source opencode and similar tools
/// use for model metadata) and flatten every provider's models into a slim list.
/// The caller caches the result client-side for ~24h.
pub async fn model_catalog() -> AppResult<Vec<CatalogModel>> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "无法初始化模型目录 HTTP 客户端".to_string())?;
    let body = client
        .get("https://models.dev/api.json")
        .send()
        .await
        .map_err(|_| "无法连接 models.dev 模型目录".to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|_| "models.dev 响应无法解析".to_string())?;
    let mut models = Vec::new();
    if let Some(providers) = body.as_object() {
        for provider in providers.values() {
            if let Some(entries) = provider["models"].as_object() {
                for (id, meta) in entries {
                    let reasoning_levels = meta["reasoning_options"]
                        .as_array()
                        .and_then(|options| options.iter().find(|option| option["type"].as_str() == Some("effort")))
                        .and_then(|option| option["values"].as_array())
                        .map(|values| values.iter().filter_map(|value| value.as_str().map(str::to_string)).collect())
                        .unwrap_or_default();
                    models.push(CatalogModel {
                        id: id.clone(),
                        context: meta["limit"]["context"].as_u64(),
                        reasoning: meta["reasoning"].as_bool().unwrap_or(false),
                        reasoning_levels,
                    });
                }
            }
        }
    }
    Ok(models)
}
