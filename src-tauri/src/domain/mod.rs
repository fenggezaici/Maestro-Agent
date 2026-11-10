//! Domain layer: pure data structures and side-effect-free rules.
pub mod error;
pub mod event;
pub mod extension;
pub mod message;
pub mod metrics;
pub mod plan;
pub mod policy;
pub mod session;

pub use error::AppResult;
pub use event::{EventData, KernelEvent, ToolPhase};
pub use extension::{
    McpConfigFile, McpServerConfig, McpServerView, McpToolView, RecommendedItem,
    SkillImportOutcome, SkillSource, SkillView,
};
pub use message::{validate_messages, ChatMessage, ToolCallSpec, ToolSpec};
pub use metrics::RunMetrics;
pub use plan::{PlanCard, PlanDecision, PlanDraft, PlanStatus, ProposedDiff};
pub use policy::{authorize, check_mode, ToolEffect, PLAN_TOOL_ALLOWLIST};
pub use session::{
    Mode, ProjectView, RunStatus, RunView, SessionDetail, SessionView, Snapshot, ToolCallView,
};
