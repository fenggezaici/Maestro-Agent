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
/// appends a pending row, later phases update it in place.
export function applyToolEvent(
  current: Record<number, ToolCallView[]>,
  event: Extract<KernelEvent, { type: 'tool_call' }>,
): Record<number, ToolCallView[]> {
  const list = [...(current[event.runId] ?? [])];
  const pendingIndex = list.findIndex((call) => call.tool === event.tool && call.pending);
  const entry: ToolCallView = {
    runId: event.runId,
    tool: event.tool,
    detail: event.detail,
    allowed: event.phase !== 'blocked' && event.phase !== 'failed',
    pending: event.phase === 'started' || event.phase === 'awaiting_approval' || undefined,
    awaitingApproval: event.phase === 'awaiting_approval' || undefined,
  };
  if (event.phase === 'started' || pendingIndex < 0) list.push(entry);
  else list[pendingIndex] = entry;
  return { ...current, [event.runId]: list };
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
}

/// The one call currently suspended on a user decision, if any — the agent
/// loop is serial so at most one `awaitingApproval` row exists across runs.
export function pendingApproval(liveTools: Record<number, ToolCallView[]> | undefined): PendingApprovalView | null {
  for (const calls of Object.values(liveTools ?? {})) {
    const call = calls.find((item) => item.awaitingApproval);
    if (call) return { runId: call.runId, tool: call.tool, preview: parseApprovalPreview(call.detail) };
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
