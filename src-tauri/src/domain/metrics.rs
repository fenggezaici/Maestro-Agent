use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunMetrics {
    pub run_id: i64,
    pub session_id: i64,
    pub provider: String,
    pub model: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_hit_tokens: Option<u64>,
    pub elapsed_ms: u64,
    pub first_token_ms: Option<u64>,
    #[serde(default)]
    pub cache_write_tokens: Option<u64>,
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub last_input_tokens: Option<u64>,
    #[serde(default)]
    pub last_output_tokens: Option<u64>,
    #[serde(default)]
    pub last_elapsed_ms: u64,
    #[serde(default)]
    pub last_first_token_ms: Option<u64>,
}
