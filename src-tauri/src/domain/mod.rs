//! Domain layer: pure data structures and side-effect-free rules.
pub mod architecture;
pub mod error;
pub mod event;
pub mod extension;
pub mod message;
pub mod activity;
pub mod agent;
pub mod approval;
pub mod compaction;
pub mod change;
pub mod command_policy;
pub mod metrics;
pub mod plan;
pub mod policy;
pub mod session;
pub mod subagent;
pub mod user_question;

pub use architecture::{
    armed_obligations, describe_obligation, effective_after_set, evaluate as evaluate_architecture,
    optional_extensions, parse_graph, path_matches, required_extensions, subagent_bindings,
    trigger_ancestors, validate_graph, ActionResult, ArmedObligation, ArchitectureGraph,
    BoundExtension, ExtensionData, ExtensionKind, ExtensionRequirement, ExtensionSnapshot, GateMode,
    GateTiming, GraphDecision, GraphEdge, GraphNode, GraphNodeData, GraphPosition, HistoryEntry,
    NodeKind, PendingCall, ResolvedPlugin, SatisfyScope, TraceStatus, TraceStep, TriggerAction,
    GRAPH_VERSION, RULES_FILE_NAME,
};
pub use error::AppResult;
pub use event::{EventData, KernelEvent, ToolPhase};
pub use extension::{
    CommandView, McpConfigFile, McpServerConfig, McpServerView, McpToolView, PluginCondition,
    PluginKind, PluginManifest, PluginView, RecommendedItem, SkillImportOutcome, SkillSource,
    SkillView, WorkspaceTrustStatus,
};
pub use message::{validate_messages, ChatMessage, ToolCallSpec, ToolSpec};
pub use activity::{ActivityDay, ActivitySnapshot, ModelTrendDay, ModelUsage};
pub use agent::{AgentDefinition, AgentSource, DEFAULT_SUBAGENT_TOOL_BUDGET, MAX_SUBAGENT_TOOL_BUDGET};
pub use change::{
    line_change_counts, merge_file_changes, RevertIssue, RevertPreview, RevertResult,
    RunChangeSummary, RunFileChange,
};
pub use metrics::RunMetrics;
pub use plan::{
    validate_plan_steps, PlanCard, PlanDecision, PlanDraft, PlanStatus, PlanStep, PlanStepStatus,
    ProposedDiff,
};
pub use policy::{authorize, classify, gate, gate_call, ToolEffect, ToolGate};
pub use session::{
    unix_now, Mode, ProjectView, ReasoningSegment, RunMessage, RunPart, RunStatus, RunView, SessionDetail,
    SessionView, Snapshot, ToolCallView,
};
pub use approval::{
    canonical_json, ApprovalCandidate, ApprovalDecision, ApprovalRule, CommandScope,
    MAX_RULE_PATTERN_CHARS,
};
pub use compaction::ContextCompaction;
pub use subagent::{SubagentStatus, SubagentTokenKind, SubagentView};
pub use user_question::{format_answers, parse_questions, validate_answers, Question, QuestionAnswer, QuestionOption};
