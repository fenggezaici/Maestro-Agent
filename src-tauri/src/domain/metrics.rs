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
    /// DSH `sessionStats` 对齐字段（schema v15）：ttft_steps 是记录了首 token
    /// 的完成请求数（TTFT 平均的分母），decode_ms/decode_tokens 只在
    /// 「首 token 与 output usage 同有」的请求上成对累计，tool_ms 是
    /// Started→终态的工具墙钟之和。
    #[serde(default)]
    pub ttft_steps: u32,
    #[serde(default)]
    pub decode_ms: u64,
    #[serde(default)]
    pub decode_tokens: u64,
    #[serde(default)]
    pub tool_ms: u64,
    /// DSH surfaceTokens 对齐：当前在途请求的 surface 文本估算 tokens
    /// （content+reasoning 的 CJK=1/其他÷4）。只在 live 快照携带，provider
    /// usage 到达或请求完成后置 None —— 估算永远不进持久化。
    #[serde(default)]
    pub live_surface_tokens: Option<u64>,
}
