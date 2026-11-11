use serde::Serialize;

use crate::domain::{
    metrics::RunMetrics,
    plan::PlanCard,
    session::{Mode, RunView},
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KernelEvent {
    pub sequence: u64,
    #[serde(flatten)]
    pub data: EventData,
}

/// Tool execution lifecycle phases reported to the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPhase {
    Started,
    /// Suspended awaiting a user approval decision; `detail` carries the
    /// bounded preview payload (diff or arguments JSON) for the approval card.
    AwaitingApproval,
    Finished,
    Failed,
    Blocked,
}

/// The single `kernel-event` channel contract. Streamed text arrives already
/// classified as `content` (answer) or `reasoning` (thinking); tool execution
/// is reported as structured `tool_call` events, never as text in the answer.
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
    /// Streamed answer text.
    Content {
        #[serde(rename = "runId")]
        run_id: i64,
        #[serde(rename = "sessionId")]
        session_id: i64,
        token: String,
    },
    /// Streamed reasoning/thinking text.
    Reasoning {
        #[serde(rename = "runId")]
        run_id: i64,
        token: String,
    },
    /// Structured tool lifecycle event.
    ToolCall {
        #[serde(rename = "runId")]
        run_id: i64,
        tool: String,
        phase: ToolPhase,
        detail: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::domain::session::{RunStatus, Snapshot};

    #[test]
    fn stream_event_matches_the_typescript_contract() {
        let event = KernelEvent {
            sequence: 4,
            data: EventData::Content {
                run_id: 7,
                session_id: 2,
                token: "你好".into(),
            },
        };
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "sequence": 4, "type": "content", "runId": 7, "sessionId": 2, "token": "你好",
            })
        );
        let reasoning = KernelEvent {
            sequence: 5,
            data: EventData::Reasoning {
                run_id: 7,
                token: "thinking".into(),
            },
        };
        assert_eq!(serde_json::to_value(reasoning).unwrap()["type"], "reasoning");
        let call = KernelEvent {
            sequence: 6,
            data: EventData::ToolCall {
                run_id: 7,
                tool: "read_file".into(),
                phase: ToolPhase::Finished,
                detail: String::new(),
            },
        };
        assert_eq!(
            serde_json::to_value(call).unwrap(),
            json!({
                "sequence": 6, "type": "tool_call", "runId": 7, "tool": "read_file",
                "phase": "finished", "detail": "",
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
}
