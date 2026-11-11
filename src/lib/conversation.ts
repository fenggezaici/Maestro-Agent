/// Pure kernel-state helpers: snapshot reducer, event folding, conversation
/// assembly. No React, no IPC — unit-testable in isolation.
import type {
  ApprovalPreview, ChatMessage, KernelEvent, RunView, SessionDetail, Snapshot, ToolCallView,
} from '../api/types';

export const initialSnapshot: Snapshot = {
  ready: false, mode: 'code', activeRun: null, lastRun: null, sequence: 0,
};

export type KernelAction =
  | { type: 'snapshot'; snapshot: Snapshot }
  | { type: 'event'; event: KernelEvent };

export function kernelReducer(state: Snapshot, action: KernelAction): Snapshot {
  if (action.type === 'snapshot') {
    // Listener-first initialization can receive events while the query is in flight.
    return action.snapshot.sequence < state.sequence
      ? { ...state, ready: action.snapshot.ready }
      : action.snapshot;
  }
  const event = action.event;
  if (event.type === 'client_error') return state;
  if (event.sequence <= state.sequence) return state;
  const next = { ...state, sequence: event.sequence };
  switch (event.type) {
    case 'ready': return { ...next, ready: true };
    case 'backend_error': return { ...next, ready: false };
    case 'mode_changed': return { ...next, mode: event.mode };
    case 'status':
      return event.run.status === 'running'
        ? { ...next, activeRun: event.run }
        : { ...next, activeRun: null, lastRun: event.run };
    case 'content':
      return state.activeRun?.id === event.runId && event.sessionId === state.activeRun.sessionId
        ? { ...next, activeRun: { ...state.activeRun, output: state.activeRun.output + event.token } }
        : next;
    case 'plan_updated':
    case 'sessions_changed':
    case 'workspace_changed':
    case 'tool_call': return next;
    case 'reasoning': return next;
    case 'metrics': return next;
  }
}

export function reconcileSnapshot(snapshot: Snapshot, buffered: readonly KernelEvent[]): Snapshot {
  return [...buffered].sort((left, right) => left.sequence - right.sequence)
    .reduce((state, event) => kernelReducer(state, { type: 'event', event }), snapshot);
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/// Fold one `tool_call` lifecycle event into the per-run tool list: `started`
/// appends a pending row, later phases update it in place. Returns the touched
/// row index so the reasoning timeline can keep the same ordering.
export function applyToolEventIndexed(
  current: Record<number, ToolCallView[]>,
  event: Extract<KernelEvent, { type: 'tool_call' }>,
): { next: Record<number, ToolCallView[]>; index: number } {
  const list = [...(current[event.runId] ?? [])];
  const pendingIndex = list.findIndex((call) => call.tool === event.tool && call.pending);
  const prev = pendingIndex >= 0 ? list[pendingIndex] : undefined;
  const entry: ToolCallView = {
    runId: event.runId,
    tool: event.tool,
    detail: event.detail,
    allowed: event.phase !== 'blocked' && event.phase !== 'failed',
    pending: event.phase === 'started' || event.phase === 'awaiting_approval' || undefined,
    awaitingApproval: event.phase === 'awaiting_approval' || undefined,
    // `started` carries the extracted target; later phases keep it while
    // `detail` rotates to preview JSON, result text, or the failure reason.
    target: event.phase === 'started' ? event.detail : prev?.target,
  };
  const index = event.phase === 'started' || pendingIndex < 0 ? list.length : pendingIndex;
  if (index === list.length) list.push(entry);
  else list[index] = entry;
  return { next: { ...current, [event.runId]: list }, index };
}

export function applyToolEvent(
  current: Record<number, ToolCallView[]>,
  event: Extract<KernelEvent, { type: 'tool_call' }>,
): Record<number, ToolCallView[]> {
  return applyToolEventIndexed(current, event).next;
}

/// One reasoning burst between tool calls / the final answer.
export interface ReasoningSegment { text: string; startedAt: number; endedAt?: number }

/// Ordered live timeline: reasoning bursts and tool rows exactly as they
/// happened. Tool items point at the row index in `liveTools`.
export type TimelineItem =
  | { kind: 'reasoning'; segment: ReasoningSegment }
  | { kind: 'tool'; index: number };

export function appendReasoning(items: TimelineItem[] | undefined, token: string, now: number): TimelineItem[] {
  const list = [...(items ?? [])];
  const last = list[list.length - 1];
  if (last?.kind === 'reasoning' && last.segment.endedAt === undefined) {
    list[list.length - 1] = { kind: 'reasoning', segment: { ...last.segment, text: last.segment.text + token } };
  } else {
    list.push({ kind: 'reasoning', segment: { text: token, startedAt: now } });
  }
  return list;
}

/// Seal the open reasoning burst; returns the same reference when nothing is open.
export function closeReasoning(items: TimelineItem[] | undefined, now: number): TimelineItem[] | undefined {
  const last = items?.[items.length - 1];
  if (!items || last?.kind !== 'reasoning' || last.segment.endedAt !== undefined) return items;
  const list = [...items];
  list[list.length - 1] = { kind: 'reasoning', segment: { ...last.segment, endedAt: now } };
  return list;
}

/// Seal the open reasoning burst and append the tool row once.
export function appendTool(items: TimelineItem[] | undefined, index: number, now: number): TimelineItem[] {
  const sealed = closeReasoning(items, now);
  const list = [...(sealed ?? [])];
  if (!list.some((item) => item.kind === 'tool' && item.index === index)) {
    list.push({ kind: 'tool', index });
  }
  return list;
}

/// Decode the kernel's approval-preview JSON carried in an `awaiting_approval`
/// detail. Anything malformed falls back to the generic args shape so the UI
/// still shows what was asked.
export function parseApprovalPreview(detail: string): ApprovalPreview | null {
  try {
    const value: unknown = JSON.parse(detail);
    if (!value || typeof value !== 'object') return null;
    const preview = value as Record<string, unknown>;
    if (preview.kind === 'write') {
      return {
        kind: 'write',
        path: typeof preview.path === 'string' ? preview.path : '',
        exists: preview.exists === true,
        old: typeof preview.old === 'string' ? preview.old : '',
        new: typeof preview.new === 'string' ? preview.new : '',
      };
    }
    if (preview.kind === 'command') {
      return { kind: 'command', command: typeof preview.command === 'string' ? preview.command : '' };
    }
    if (preview.kind === 'args') {
      return { kind: 'args', arguments: typeof preview.arguments === 'string' ? preview.arguments : detail };
    }
    return { kind: 'args', arguments: detail };
  } catch {
    return null;
  }
}

export interface PendingApprovalView {
  runId: number;
  tool: string;
  preview: ApprovalPreview | null;
  count: number;
}

/// One entry in the Review Hub file list — pending write, applied write, or
/// plan proposal, all reduced to a before/after diff of one path.
export interface ReviewFile {
  path: string;
  before: string;
  after: string;
  status: 'pending' | 'applied' | 'proposal';
  runId?: number;
  exists?: boolean;
  tool?: string;
}

function writeResult(detail: string): { path: string; before: string; after: string } | null {
  try {
    const value = JSON.parse(detail) as { kind?: string; path?: string; before?: string; after?: string };
    return value.kind === 'write_result' && typeof value.before === 'string' && typeof value.after === 'string'
      ? { path: value.path ?? '', before: value.before, after: value.after }
      : null;
  } catch {
    return null;
  }
}

/// Flatten every diff-shaped thing the run produced into one ordered list —
/// pending write beats applied beats proposal for the same path.
export function reviewFiles(input: {
  liveTools: Record<number, ToolCallView[]> | undefined;
  pending: PendingApprovalView | null;
  plans: { id: number; draft: { diffs: { path: string; before: string; after: string }[] } }[];
}): ReviewFile[] {
  const files: ReviewFile[] = [];
  const seen = new Set<string>();
  const push = (file: ReviewFile) => {
    if (file.path && !seen.has(file.path)) { seen.add(file.path); files.push(file); }
  };
  if (input.pending?.preview?.kind === 'write') {
    const preview = input.pending.preview;
    push({ path: preview.path, before: preview.old, after: preview.new, status: 'pending', runId: input.pending.runId, exists: preview.exists, tool: input.pending.tool });
  }
  const appliedByPath = new Map<string, number>();
  for (const calls of Object.values(input.liveTools ?? {})) {
    for (const call of calls) {
      if (!call.allowed || call.pending || call.awaitingApproval) continue;
      const result = writeResult(call.detail);
      if (!result) continue;
      // Same path written repeatedly → merge into a cumulative diff
      // (first before → latest after) so the panel never goes stale.
      const previous = appliedByPath.get(result.path);
      if (previous !== undefined) {
        files[previous] = { ...files[previous], after: result.after, runId: call.runId };
      } else if (!seen.has(result.path)) {
        seen.add(result.path);
        appliedByPath.set(result.path, files.length);
        files.push({ path: result.path, before: result.before, after: result.after, status: 'applied', runId: call.runId, tool: call.tool });
      }
    }
  }
  for (const plan of input.plans) {
    for (const diff of plan.draft.diffs) {
      push({ path: diff.path, before: diff.before, after: diff.after, status: 'proposal', tool: 'plan' });
    }
  }
  return files;
}

/// The one call currently suspended on a user decision, if any — the agent
/// loop is serial so at most one `awaitingApproval` row exists across runs.
export type AgentStatus = 'stopping' | 'approval' | 'tool' | 'thinking' | 'working';

/// Devin-style bottom status pill: the first matching live state wins —
/// stopping > waiting for approval > tool executing > reasoning > working.
export function agentStatus(input: { running: boolean; liveCalls?: ToolCallView[]; reasoning?: string; hasApproval?: boolean; stopping?: boolean }): AgentStatus | null {
  if (!input.running) return null;
  if (input.stopping) return 'stopping';
  if (input.hasApproval) return 'approval';
  if (input.liveCalls?.some((call) => call.pending)) return 'tool';
  if (input.reasoning) return 'thinking';
  return 'working';
}

export function pendingApproval(liveTools: Record<number, ToolCallView[]> | undefined): PendingApprovalView | null {
  for (const calls of Object.values(liveTools ?? {})) {
    const call = calls.find((item) => item.awaitingApproval);
    if (call) return { runId: call.runId, tool: call.tool, preview: parseApprovalPreview(call.detail), count: calls.filter((item) => item.awaitingApproval).length };
  }
  return null;
}

export function conversationRuns(detail: SessionDetail | null, sessionId: number | null, snapshot: Snapshot): RunView[] {
  const runs = new Map((detail?.session.id === sessionId ? detail.runs : []).map((run) => [run.id, run]));
  for (const run of [snapshot.lastRun, snapshot.activeRun]) {
    if (run?.sessionId === sessionId) runs.set(run.id, run);
  }
  return [...runs.values()].sort((left, right) => left.id - right.id);
}

export function chatMessages(runs: RunView[], sessionId: number, prompt: string, images?: string[]): ChatMessage[] {
  let budget = 131072;
  const included: RunView[] = [];
  for (const run of runs.filter((run) => run.sessionId === sessionId && run.status === 'completed' && run.output).slice(-32).reverse()) {
    const size = new TextEncoder().encode(run.prompt + run.output).length;
    if (size > budget) break;
    budget -= size;
    included.push(run);
  }
  return [...included.reverse().flatMap((run): ChatMessage[] => [{ role: 'user', content: run.prompt }, { role: 'assistant', content: run.output }]), { role: 'user', content: prompt.trim(), ...(images?.length ? { images } : {}) }];
}
