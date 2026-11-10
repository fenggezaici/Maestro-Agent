/// Pure kernel-state helpers: snapshot reducer, event folding, conversation
/// assembly. No React, no IPC — unit-testable in isolation.
import type {
  ChatMessage, KernelEvent, RunView, SessionDetail, Snapshot, ToolCallView,
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
    pending: event.phase === 'started' || undefined,
  };
  if (event.phase === 'started' || pendingIndex < 0) list.push(entry);
  else list[pendingIndex] = entry;
  return { ...current, [event.runId]: list };
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
