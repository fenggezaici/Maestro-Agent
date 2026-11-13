use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDay {
    pub date: String,
    pub short_date: String,
    pub lines: i64,
    pub tokens: i64,
    pub plans: i64,
    pub level: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySnapshot {
    pub source: &'static str,
    pub start: String,
    pub end: String,
    pub total_lines: i64,
    pub approved_plans: i64,
    pub total_tokens: i64,
    pub cost: Option<f64>,
    pub today_lines: i64,
    pub days: Vec<ActivityDay>,
    pub trend: Vec<ActivityDay>,
}
