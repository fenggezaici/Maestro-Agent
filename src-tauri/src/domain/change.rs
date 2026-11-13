use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFileChange {
    pub path: String,
    pub added: i64,
    pub removed: i64,
    pub before: String,
    pub after: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertResult {
    pub reverted: Vec<String>,
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunChangeSummary {
    pub files: i64,
    pub additions: i64,
    pub deletions: i64,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertIssue {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertPreview {
    pub safe_files: Vec<String>,
    pub unsafe_files: Vec<RevertIssue>,
    pub ignored_files: Vec<RevertIssue>,
}
