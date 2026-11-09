use serde::Serialize;

pub type AppResult<T> = Result<T, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Plan,
    Code,
    Confirm,
    Auto,
    // Historical records only; new UI selections use Code.
    Execute,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Code => "code",
            Self::Confirm => "confirm",
            Self::Auto => "auto",
            Self::Execute => "execute",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Completed,
    Cancelled,
    Failed,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "completed" => Self::Completed,
            "cancelled" => Self::Cancelled,
            "running" => Self::Running,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunView {
    pub id: i64,
    pub session_id: i64,
    pub prompt: String,
    pub mode: Mode,
    pub status: RunStatus,
    pub output: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub ready: bool,
    pub mode: Mode,
    pub active_run: Option<RunView>,
    pub last_run: Option<RunView>,
    pub sequence: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub id: i64,
    pub title: String,
    pub updated_at: i64,
    pub run_count: i64,
    pub project_id: i64,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub id: i64,
    pub name: String,
    pub path: Option<String>,
    pub archived: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub session: SessionView,
    pub runs: Vec<RunView>,
    pub plans: Vec<PlanCard>,
    pub metrics: Vec<RunMetrics>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunMetrics {
    pub run_id: i64,
    pub session_id: i64,
    pub provider: String,
    pub model: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub elapsed_ms: u64,
    pub first_token_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Pending,
    Approved,
    Rejected,
    Invalidated,
}

impl PlanStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Invalidated => "invalidated",
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanDecision {
    Approve,
    Reject,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedDiff {
    pub path: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanDraft {
    pub title: String,
    pub summary: String,
    pub steps: Vec<String>,
    pub diffs: Vec<ProposedDiff>,
}

impl PlanDraft {
    pub fn validate(&self) -> AppResult<()> {
        if self.title.trim().is_empty()
            || self.title.len() > 256
            || self.summary.len() > 16384
            || self.steps.is_empty()
            || self.steps.len() > 20
            || self.steps.iter().any(|step| step.len() > 4096)
            || self.diffs.len() > 20
            || self
                .diffs
                .iter()
                .any(|diff| diff.path.len() > 512 || diff.before.len() + diff.after.len() > 65536)
        {
            return Err("Invalid or oversized plan proposal".into());
        }
        for diff in &self.diffs {
            if diff.path.is_empty()
                || diff.path.starts_with('/')
                || diff.path.contains(['\\', ':'])
                || diff
                    .path
                    .split('/')
                    .any(|part| part == ".." || part.is_empty())
            {
                return Err("Proposed diff paths must be relative and scoped".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanCard {
    pub id: i64,
    pub session_id: i64,
    pub run_id: i64,
    pub status: PlanStatus,
    pub draft: PlanDraft,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KernelEvent {
    pub sequence: u64,
    #[serde(flatten)]
    pub data: EventData,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventData {
    Ready,
    Metrics {
        metrics: RunMetrics,
    },
    BackendError {
        message: String,
    },
    ModeChanged {
        mode: Mode,
    },
    Status {
        run: RunView,
    },
    PlanUpdated {
        plan: PlanCard,
    },
    SessionsChanged,
    ToolBlocked {
        #[serde(rename = "runId")]
        run_id: i64,
        tool: String,
        reason: String,
    },
    StreamToken {
        #[serde(rename = "runId")]
        run_id: i64,
        #[serde(rename = "sessionId")]
        session_id: i64,
        token: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stream_event_matches_the_typescript_contract() {
        let event = KernelEvent {
            sequence: 4,
            data: EventData::StreamToken {
                run_id: 7,
                session_id: 2,
                token: "你好".into(),
            },
        };
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "sequence": 4, "type": "stream_token", "runId": 7, "sessionId": 2, "token": "你好",
            })
        );
    }

    #[test]
    fn snapshot_uses_camel_case_and_explicit_null_fields() {
        let snapshot = Snapshot {
            ready: true,
            mode: Mode::Plan,
            active_run: None,
            last_run: None,
            sequence: 1,
        };
        assert_eq!(
            serde_json::to_value(snapshot).unwrap(),
            json!({
                "ready": true, "mode": "plan", "activeRun": null, "lastRun": null, "sequence": 1,
            })
        );
        let run = RunView {
            id: 7,
            session_id: 2,
            prompt: "hello".into(),
            mode: Mode::Plan,
            status: RunStatus::Completed,
            output: "done".into(),
            error: None,
        };
        let event = KernelEvent {
            sequence: 8,
            data: EventData::Status { run },
        };
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "sequence": 8, "type": "status", "run": {
                    "id": 7, "sessionId": 2, "prompt": "hello", "mode": "plan", "status": "completed",
                    "output": "done", "error": null,
                },
            })
        );
    }

    #[test]
    fn unknown_mode_cannot_be_deserialized_as_execute() {
        assert_eq!(
            serde_json::from_str::<Mode>("\"execute\"").unwrap(),
            Mode::Execute
        );
        assert!(serde_json::from_str::<Mode>("\"unrestricted\"").is_err());
        assert!(serde_json::from_str::<Mode>("\"Execute\"").is_err());
    }

    #[test]
    fn plan_contract_and_paths_are_scoped() {
        let mut draft = PlanDraft {
            title: "proposal".into(),
            summary: "not applied".into(),
            steps: vec!["review".into()],
            diffs: vec![ProposedDiff {
                path: "docs/plan.md".into(),
                before: String::new(),
                after: "proposal".into(),
            }],
        };
        draft.validate().unwrap();
        let card = PlanCard {
            id: 1,
            session_id: 2,
            run_id: 3,
            status: PlanStatus::Pending,
            draft: draft.clone(),
        };
        let value = serde_json::to_value(KernelEvent {
            sequence: 5,
            data: EventData::PlanUpdated { plan: card },
        })
        .unwrap();
        assert_eq!(value["type"], "plan_updated");
        assert_eq!(value["plan"]["sessionId"], 2);
        assert_eq!(value["plan"]["status"], "pending");
        for path in ["../outside", "/absolute", "C:\\file", "docs/../file"] {
            draft.diffs[0].path = path.into();
            assert!(draft.validate().is_err());
        }
    }
}
