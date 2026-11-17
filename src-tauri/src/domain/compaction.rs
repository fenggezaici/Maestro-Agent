use serde::{Deserialize, Serialize};

/// ZCode-style synthetic context-compaction record. `through_run_id` is the
/// boundary: runs at or before it are represented by `summary` instead of
/// being replayed verbatim.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextCompaction {
    pub operation_id: String,
    pub session_id: i64,
    pub through_run_id: i64,
    pub summary: String,
    /// `auto` or `manual`.
    pub trigger: String,
    /// `running`, `completed`, `failed` or `interrupted`.
    pub status: String,
    pub pre_tokens: u64,
    pub post_tokens: Option<u64>,
    pub created_at: i64,
}
