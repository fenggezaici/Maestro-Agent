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
/// bounded preview of what would run. `write` diffs render in the Monaco
/// panel; `command` renders as a terminal card; `args` is the generic fallback.
export type ApprovalPreview =
  | { kind: 'write'; path: string; exists: boolean; old: string; new: string }
  | { kind: 'command'; command: string }
  | { kind: 'args'; arguments: string };
export interface SessionDetail { session: SessionView; runs: RunView[]; plans: PlanCard[]; metrics?: RunMetrics[]; tools?: ToolCallView[] }

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
  | { type: 'subagent_finished'; runId: number; subagentId: string; status: SubagentStatus; summary: string; error: string | null; toolCount: number; inputTokens: number | null; outputTokens: number | null; finishedAt: number }
  | { type: 'user_question'; runId: number; requestId: string; questions: UserQuestion[] }
  | { type: 'reasoning'; runId: number; token: string }
);
