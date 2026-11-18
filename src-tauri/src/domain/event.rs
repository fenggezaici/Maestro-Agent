use serde::Serialize;

use crate::domain::{
    metrics::RunMetrics,
    plan::{PlanCard, PlanStep},
    session::{Mode, RunView},
    subagent::{SubagentStatus, SubagentTokenKind, SubagentView},
    compaction::ContextCompaction,
    user_question::Question,
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
    /// Live task-tracker steps published by the model through `update_plan`.
    /// Distinct from `PlanUpdated`, which carries the approve/reject plan card.
    TaskPlanUpdated {
        #[serde(rename = "runId")]
        run_id: i64,
        #[serde(rename = "sessionId")]
        session_id: i64,
        steps: Vec<PlanStep>,
    },
    SessionsChanged,
    /// A side-effecting tool finished — the workspace tree should re-scan.
    WorkspaceChanged,
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
        /// Present on `spawn_subagent` rows so the UI can open its trace.
        #[serde(rename = "subagentId", skip_serializing_if = "Option::is_none")]
        subagent_id: Option<String>,
    },
    /// A child research run was created (trace is already persisted).
    SubagentStarted {
        #[serde(rename = "runId")]
        run_id: i64,
        subagent: SubagentView,
    },
    /// Child tool lifecycle; never mixed into the parent tool timeline.
    SubagentTool {
        #[serde(rename = "runId")]
        run_id: i64,
        #[serde(rename = "subagentId")]
        subagent_id: String,
        tool: String,
        phase: ToolPhase,
        detail: String,
    },
    /// Streamed child answer or reasoning token.
    SubagentToken {
        #[serde(rename = "runId")]
        run_id: i64,
        #[serde(rename = "subagentId")]
        subagent_id: String,
        kind: SubagentTokenKind,
        token: String,
    },
    /// Synthetic context-compaction separator (ZCode `context_compaction`).
    ContextCompaction { compaction: ContextCompaction },
    /// The model is waiting for a structured answer from the user.
    UserQuestion {
        #[serde(rename = "runId")]
        run_id: i64,
        #[serde(rename = "requestId")]
        request_id: String,
        questions: Vec<Question>,
    },
    SubagentFinished {
        #[serde(rename = "runId")]
        run_id: i64,
        #[serde(rename = "subagentId")]
        subagent_id: String,
        status: SubagentStatus,
        summary: String,
        error: Option<String>,
        #[serde(rename = "toolCount")]
        tool_count: usize,
        #[serde(rename = "inputTokens")]
        input_tokens: Option<u64>,
        #[serde(rename = "outputTokens")]
        output_tokens: Option<u64>,
        #[serde(rename = "reasoningTokens")]
        reasoning_tokens: Option<u64>,
        #[serde(rename = "firstTokenMs")]
        first_token_ms: Option<u64>,
        #[serde(rename = "elapsedMs")]
        elapsed_ms: u64,
        #[serde(rename = "decodeMs")]
        decode_ms: u64,
        #[serde(rename = "decodeTokens")]
        decode_tokens: u64,
        #[serde(rename = "lastInputTokens")]
        last_input_tokens: Option<u64>,
        #[serde(rename = "finishedAt")]
        finished_at: i64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::domain::session::{RunStatus, Snapshot};
    use crate::domain::{SubagentStatus, SubagentTokenKind, SubagentView};

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
                subagent_id: None,
            },
        };
        assert_eq!(
            serde_json::to_value(call).unwrap(),
            json!({
                "sequence": 6, "type": "tool_call", "runId": 7, "tool": "read_file",
                "phase": "finished", "detail": "",
            })
        );
        let plan = KernelEvent {
            sequence: 7,
            data: EventData::TaskPlanUpdated {
                run_id: 7,
                session_id: 2,
                steps: vec![PlanStep {
                    id: "1".into(),
                    description: "读取项目".into(),
                    status: crate::domain::PlanStepStatus::InProgress,
                }],
            },
        };
        assert_eq!(
            serde_json::to_value(plan).unwrap(),
            json!({
                "sequence": 7, "type": "task_plan_updated", "runId": 7, "sessionId": 2,
                "steps": [{ "id": "1", "description": "读取项目", "status": "in_progress" }],
            })
        );
    }

    #[test]
    fn subagent_events_serialize_with_camel_case_ids() {
        let view = SubagentView {
            id: "sa-7-1".into(),
            run_id: 7,
            session_id: 2,
            agent: "explore".into(),
            description: "调研".into(),
            prompt: "读文件".into(),
            model: Some("m".into()),
            status: SubagentStatus::Running,
            summary: String::new(),
            error: None,
            content: String::new(),
            reasoning: String::new(),
            tool_count: 0,
            input_tokens: None,
            output_tokens: None,
            reasoning_tokens: None,
            first_token_ms: None,
            elapsed_ms: 0,
            decode_ms: 0,
            decode_tokens: 0,
            last_input_tokens: None,
            started_at: 1,
            finished_at: None,
            tools: Vec::new(),
        };
        let started = KernelEvent {
            sequence: 9,
            data: EventData::SubagentStarted { run_id: 7, subagent: view },
        };
        let value = serde_json::to_value(started).unwrap();
        assert_eq!(value["type"], "subagent_started");
        assert_eq!(value["runId"], 7);
        assert_eq!(value["subagent"]["id"], "sa-7-1");
        assert_eq!(value["subagent"]["toolCount"], 0);

        let spawn = KernelEvent {
            sequence: 10,
            data: EventData::ToolCall {
                run_id: 7,
                tool: "spawn_subagent".into(),
                phase: ToolPhase::Started,
                detail: "调研".into(),
                subagent_id: Some("sa-7-1".into()),
            },
        };
        assert_eq!(serde_json::to_value(spawn).unwrap()["subagentId"], "sa-7-1");

        let token = KernelEvent {
            sequence: 11,
            data: EventData::SubagentToken {
                run_id: 7,
                subagent_id: "sa-7-1".into(),
                kind: SubagentTokenKind::Content,
                token: "hi".into(),
            },
        };
        let value = serde_json::to_value(token).unwrap();
        assert_eq!(value["type"], "subagent_token");
        assert_eq!(value["kind"], "content");

        let finished = KernelEvent {
            sequence: 12,
            data: EventData::SubagentFinished {
                run_id: 7,
                subagent_id: "sa-7-1".into(),
                status: SubagentStatus::Completed,
                summary: "done".into(),
                error: None,
                tool_count: 2,
                input_tokens: Some(1),
                output_tokens: Some(2),
                reasoning_tokens: Some(0),
                first_token_ms: Some(100),
                elapsed_ms: 1_000,
                decode_ms: 900,
                decode_tokens: 2,
                last_input_tokens: Some(1),
                finished_at: 3,
            },
        };
        let value = serde_json::to_value(finished).unwrap();
        assert_eq!(value["status"], "completed");
        assert_eq!(value["toolCount"], 2);
        assert_eq!(value["finishedAt"], 3);
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
            created_at: 0,
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
                    "output": "done", "error": null, "createdAt": 0,
                },
            })
        );
    }
}
