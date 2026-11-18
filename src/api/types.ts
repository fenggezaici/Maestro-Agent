/// Shared IPC DTOs — the wire contract with the Rust backend.
/// camelCase on the wire; these types are the single source of truth.

export interface ChatMessage { role: 'user' | 'assistant'; content: string; images?: string[] }

export type Mode = 'plan' | 'code' | 'confirm' | 'auto';
export interface RunMetrics {
  runId: number; sessionId: number; provider: string; model: string;
  inputTokens: number | null; outputTokens: number | null;
  cacheHitTokens?: number | null; elapsedMs: number; firstTokenMs: number | null;
  cacheWriteTokens?: number | null; reasoningTokens?: number | null;
  attempts?: number; lastInputTokens?: number | null; lastOutputTokens?: number | null; lastElapsedMs?: number;
  lastFirstTokenMs?: number | null;
  /** DSH sessionStats 口径：记录了首 delta 的完成请求数（TTFT 平均的分母）。 */
  ttftSteps?: number;
  /** Σ decode 窗口（首 delta → 流结束），只计「首 token 与 output usage 同有」的请求。 */
  decodeMs?: number;
  /** 与 decodeMs 成对的 output tokens；tok/s = ΣdecodeTokens / ΣdecodeMs。 */
  decodeTokens?: number;
  /** Σ 工具墙钟（Started → 终态配对），不含 ask_user 等待与 spawn_subagent。 */
  toolMs?: number;
  /** 在途请求的 surface 文本估算 tokens（DSH surfaceTokens 口径，仅 live 快照携带，永不持久化）。 */
  liveSurfaceTokens?: number | null;
}
export type RunStatus = 'running' | 'completed' | 'cancelled' | 'failed';

export interface RunView {
  id: number;
  sessionId: number;
  prompt: string;
  mode: Mode | 'execute';
  status: RunStatus;
  output: string;
  error: string | null;
  /** Unix seconds; absent in older fixtures/mocks. */
  createdAt?: number;
}

export interface Snapshot {
  ready: boolean;
  mode: Mode;
  activeRun: RunView | null;
  lastRun: RunView | null;
  sequence: number;
}

export interface SessionView {
  id: number;
  title: string;
  updatedAt: number;
  runCount: number;
  projectId: number;
  archived: boolean;
}

export interface ProjectView {
  id: number;
  name: string;
  path: string | null;
  archived: boolean;
  createdAt: number;
}

export type PlanStatus = 'pending' | 'approved' | 'rejected' | 'invalidated';
export type PlanDecision = 'approve' | 'reject';
export interface ProposedDiff { path: string; before: string; after: string }
export interface PlanDraft { title: string; summary: string; steps: string[]; diffs: ProposedDiff[] }
export interface PlanCard {
  id: number;
  sessionId: number;
  runId: number;
  status: PlanStatus;
  draft: PlanDraft;
}
export interface ToolCallView { runId: number; tool: string; detail: string; allowed: boolean; pending?: boolean; awaitingApproval?: boolean; target?: string; subagentId?: string }

export interface QuestionOption { label: string; description?: string | null }
export interface UserQuestion {
  id: string;
  header?: string | null;
  question: string;
  multiSelect?: boolean;
  options: QuestionOption[];
}
export interface QuestionAnswer { questionId: string; selected: string[]; custom?: string | null }

export type ApprovalDecision = 'allow_once' | 'allow_always' | 'deny';
export type CommandScope = 'prefix' | 'exact';

export type SubagentStatus = 'running' | 'completed' | 'failed' | 'cancelled';
export interface SubagentView {
  id: string;
  runId: number;
  sessionId: number;
  agent: string;
  description: string;
  prompt: string;
  model?: string | null;
  status: SubagentStatus;
  summary: string;
  error?: string | null;
  content: string;
  reasoning: string;
  toolCount: number;
  inputTokens?: number | null;
  outputTokens?: number | null;
  reasoningTokens?: number | null;
  firstTokenMs?: number | null;
  elapsedMs?: number | null;
  /** Σ 每次 LLM 调用的 decode 窗口（首 delta → 末 delta），仅计入上报 output usage 的调用。 */
  decodeMs?: number;
  /** 与 decodeMs 成对的 output tokens；tok/s = decodeTokens / decodeMs。 */
  decodeTokens?: number;
  lastInputTokens?: number | null;
  startedAt: number;
  finishedAt?: number | null;
  tools: ToolCallView[];
  /** Live-only interleaved reasoning/tool/content timeline. */
  timeline?: import('../lib/conversation').TimelineItem[];
}

/// Live task-tracker steps published by `update_plan`; per-session UI state.
export type PlanStepStatus = 'pending' | 'in_progress' | 'completed';
export interface PlanStep { id: string; description: string; status: PlanStepStatus }

/// Payload carried in an `awaiting_approval` tool_call detail: the kernel's
/// bounded preview of what would run. `write` diffs render in the code
/// panel; `command` renders as a terminal card; `args` is the generic fallback.
export type ApprovalPreview =
  | { kind: 'write'; path: string; exists: boolean; old: string; new: string }
  | { kind: 'command'; command: string }
  | { kind: 'args'; arguments: string };
/// Session-scoped "always allow" rule derived from an approval decision.
/// `prefix` rules exist only for `run_command`; both are shown so they can be
/// reviewed and revoked instead of silently widening.
export interface ApprovalRuleView { tool: string; pattern: string; prefix: boolean }
export interface ContextCompaction {  operationId: string; sessionId: number; throughRunId: number; summary: string;
  trigger: 'auto' | 'manual'; status: 'running' | 'completed' | 'failed' | 'interrupted';
  preTokens: number; postTokens?: number | null; createdAt: number;
}
export interface RunMessage {
  runId: number; sessionId: number; seq: number; role: string; content: string;
  toolCalls?: unknown; toolCallId?: string | null; name?: string | null;
}
/// Persisted reasoning bursts per run id (replay after a restart).
export interface PersistedReasoning { text: string; startedAt: number; endedAt?: number | null }
/// Ordered timeline part persisted per run (ZCode-style message parts).
export interface PersistedPart { kind: 'reasoning' | 'tool' | 'content'; text?: string; index?: number; startedAt: number; endedAt?: number | null }
export interface SessionDetail { session: SessionView; runs: RunView[]; plans: PlanCard[]; metrics?: RunMetrics[]; tools?: ToolCallView[]; compactions?: ContextCompaction[]; runMessages?: RunMessage[]; reasoning?: Record<number, PersistedReasoning[]>; parts?: Record<number, PersistedPart[]> }

/// Single `kernel-event` channel contract. Streamed text arrives already
/// classified as `content` (answer) or `reasoning` (thinking); tool execution
/// is reported as structured `tool_call` lifecycle events.
export type KernelEvent = { sequence: number } & (
  | { type: 'ready' }
  | { type: 'metrics'; metrics: RunMetrics }
  | { type: 'backend_error'; message: string }
  | { type: 'client_error'; message: string }
  | { type: 'mode_changed'; mode: Mode }
  | { type: 'status'; run: RunView }
  | { type: 'content'; runId: number; sessionId: number; token: string }
  | { type: 'plan_updated'; plan: PlanCard }
  | { type: 'task_plan_updated'; runId: number; sessionId: number; steps: PlanStep[] }
  | { type: 'sessions_changed' }
  | { type: 'workspace_changed' }
  | { type: 'tool_call'; runId: number; tool: string; phase: 'started' | 'awaiting_approval' | 'finished' | 'failed' | 'blocked'; detail: string; subagentId?: string }
  | { type: 'subagent_started'; runId: number; subagent: SubagentView }
  | { type: 'subagent_tool'; runId: number; subagentId: string; tool: string; phase: 'started' | 'awaiting_approval' | 'finished' | 'failed' | 'blocked'; detail: string }
  | { type: 'subagent_token'; runId: number; subagentId: string; kind: 'content' | 'reasoning'; token: string }
  | { type: 'subagent_finished'; runId: number; subagentId: string; status: SubagentStatus; summary: string; error: string | null; toolCount: number; inputTokens: number | null; outputTokens: number | null; reasoningTokens?: number | null; firstTokenMs?: number | null; elapsedMs?: number | null; decodeMs?: number; decodeTokens?: number; lastInputTokens?: number | null; finishedAt: number }
  | { type: 'user_question'; runId: number; requestId: string; questions: UserQuestion[] }
  | { type: 'context_compaction'; compaction: ContextCompaction }
  | { type: 'reasoning'; runId: number; token: string }
);
