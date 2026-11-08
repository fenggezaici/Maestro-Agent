import { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import { backend, type KernelEvent, type Mode, type PlanCard, type PlanDecision, type RunView, type SessionDetail, type SessionView, type Snapshot } from './ipc';
import { readLlmSettings, validateLlmSettings } from './settings';
import type { ChatMessage } from './ipc';

export const initialSnapshot: Snapshot = {
  ready: false, mode: 'plan', activeRun: null, lastRun: null, sequence: 0,
};

type Action = { type: 'snapshot'; snapshot: Snapshot } | { type: 'event'; event: KernelEvent };

export function kernelReducer(state: Snapshot, action: Action): Snapshot {
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
    case 'stream_token':
      return state.activeRun?.id === event.runId && (event.sessionId === undefined || event.sessionId === state.activeRun.sessionId)
        ? { ...next, activeRun: { ...state.activeRun, output: state.activeRun.output + event.token } }
        : next;
    case 'plan_updated':
    case 'sessions_changed':
    case 'tool_blocked': return next;
  }
}

export function reconcileSnapshot(snapshot: Snapshot, buffered: readonly KernelEvent[]): Snapshot {
  return [...buffered].sort((left, right) => left.sequence - right.sequence)
    .reduce((state, event) => kernelReducer(state, { type: 'event', event }), snapshot);
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function conversationRuns(detail: SessionDetail | null, sessionId: number | null, snapshot: Snapshot): RunView[] {
  const runs = new Map((detail?.session.id === sessionId ? detail.runs : []).map((run) => [run.id, run]));
  for (const run of [snapshot.lastRun, snapshot.activeRun]) {
    if (run?.sessionId === sessionId) runs.set(run.id, run);
  }
  return [...runs.values()].sort((left, right) => left.id - right.id);
}

export function chatMessages(runs: RunView[], sessionId: number, prompt: string): ChatMessage[] {
  let budget = 131072;
  const included: RunView[] = [];
  for (const run of runs.filter((run) => run.sessionId === sessionId && run.status === 'completed' && run.output).slice(-32).reverse()) {
    const size = new TextEncoder().encode(run.prompt + run.output).length;
    if (size > budget) break;
    budget -= size;
    included.push(run);
  }
  return [...included.reverse().flatMap((run): ChatMessage[] => [{ role: 'user', content: run.prompt }, { role: 'assistant', content: run.output }]), { role: 'user', content: prompt.trim() }];
}

export function useKernel() {
  const [state, dispatch] = useReducer(kernelReducer, initialSnapshot);
  const [sessions, setSessions] = useState<SessionView[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<number | null>(null);
  const [detail, setDetail] = useState<SessionDetail | null>(null);
  const [loadingSession, setLoadingSession] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pendingCount, setPendingCount] = useState(0);
  const sessionsRevision = useRef(0);
  const detailRevision = useRef(0);
  const selectedRef = useRef<number | null>(null);
  const selectionRevision = useRef(0);
  const mounted = useRef(false);

  const selectSession = useCallback((id: number | null) => {
    selectionRevision.current++;
    if (selectedRef.current === id) return;
    selectedRef.current = id;
    detailRevision.current++;
    setDetail(null);
    setSelectedSessionId(id);
    setLoadingSession(id !== null);
  }, []);

  const refreshSessions = useCallback(async () => {
    const revision = ++sessionsRevision.current;
    const items = await backend.sessions();
    if (!mounted.current || revision !== sessionsRevision.current) return;
    setSessions(items);
    if (selectedRef.current === null || !items.some((item) => item.id === selectedRef.current)) {
      selectSession(items[0]?.id ?? null);
    }
  }, [selectSession]);

  const refreshDetail = useCallback(async (id = selectedRef.current) => {
    if (id === null || id !== selectedRef.current) return;
    const revision = ++detailRevision.current;
    setLoadingSession(true);
    try {
      const result = await backend.session(id);
      if (mounted.current && revision === detailRevision.current && selectedRef.current === id) setDetail(result);
    } finally {
      if (mounted.current && revision === detailRevision.current) setLoadingSession(false);
    }
  }, []);

  useEffect(() => {
    if (selectedSessionId !== null) {
      void refreshDetail(selectedSessionId).catch((reason: unknown) => {
        if (mounted.current) setError(errorMessage(reason));
      });
    }
  }, [selectedSessionId, refreshDetail]);

  useEffect(() => {
    mounted.current = true;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let buffered: KernelEvent[] | null = [];
    let lastEventSequence = 0;
    async function connect() {
      try {
        const stop = await backend.listen((event) => {
          if (disposed) return;
          // A reload can attach mid-run. Do not apply deltas before the full baseline.
          if (buffered !== null) { buffered.push(event); return; }
          if (event.type !== 'client_error') {
            if (event.sequence <= lastEventSequence) return;
            lastEventSequence = event.sequence;
          }
          dispatch({ type: 'event', event });
          if (event.type === 'backend_error' || event.type === 'client_error') setError(event.message);
          if (event.type === 'backend_error') {
            void backend.snapshot().then((snapshot) => {
              if (!disposed) dispatch({ type: 'snapshot', snapshot });
            }).catch(() => { /* Retain the explicit storage failure instead of masking it. */ });
          }
          if (event.type === 'status' && event.run.status === 'failed') setError(event.run.error ?? '模型请求失败');
          if (event.type === 'sessions_changed') {
            void refreshSessions().catch((reason: unknown) => { if (!disposed) setError(errorMessage(reason)); });
          }
          if (event.type === 'plan_updated' || (event.type === 'status' && event.run.status !== 'running')) {
            void refreshDetail().catch((reason: unknown) => {
              if (!disposed) setError(errorMessage(reason));
            });
          }
        });
        if (disposed) { stop(); return; }
        unlisten = stop;
        const snapshot = await backend.snapshot();
        if (disposed) return;
        const reconciled = reconcileSnapshot(snapshot, buffered ?? []);
        lastEventSequence = reconciled.sequence;
        dispatch({ type: 'snapshot', snapshot: reconciled });
        for (const event of buffered ?? []) {
          if (event.type === 'backend_error' || event.type === 'client_error') setError(event.message);
          if (event.type === 'status' && event.run.status === 'failed') setError(event.run.error ?? '模型请求失败');
        }
        if (!reconciled.ready && reconciled.activeRun?.error) setError(reconciled.activeRun.error);
        if (selectedRef.current === null && reconciled.activeRun) selectSession(reconciled.activeRun.sessionId);
        buffered = null;
        await refreshSessions();
      } catch (reason) {
        if (!disposed) setError(`无法连接 Rust 后端，请通过 npm run tauri dev 启动。${errorMessage(reason)}`);
      }
    }
    void connect();
    return () => {
      disposed = true;
      mounted.current = false;
      sessionsRevision.current++;
      detailRevision.current++;
      unlisten?.();
    };
  }, [refreshSessions, refreshDetail, selectSession]);

  const command = useCallback(async <T,>(operation: () => Promise<T>): Promise<T | undefined> => {
    setPendingCount((count) => count + 1);
    setError(null);
    try {
      const result = await operation();
      const snapshot = await backend.snapshot();
      if (mounted.current) dispatch({ type: 'snapshot', snapshot });
      return result;
    } catch (reason) {
      if (mounted.current) setError(errorMessage(reason));
      return undefined;
    } finally {
      if (mounted.current) setPendingCount((count) => count - 1);
    }
  }, []);

  return {
    state, sessions, selectedSessionId, detail, loadingSession, error, pending: pendingCount > 0,
    runs: conversationRuns(detail, selectedSessionId, state),
    selectSession,
    createSession: () => command(async () => {
      const revision = selectionRevision.current;
      const session = await backend.createSession();
      if (mounted.current && revision === selectionRevision.current) selectSession(session.id);
      await refreshSessions();
      return session;
    }),
    start: (prompt: string) => command(async () => {
      const settings = validateLlmSettings(readLlmSettings());
      let id = selectedRef.current;
      if (id === null) {
        const revision = selectionRevision.current;
        const session = await backend.createSession();
        id = session.id;
        if (mounted.current && revision === selectionRevision.current) selectSession(id);
      }
      const context = await backend.session(id);
      const runId = await backend.start(id, chatMessages(context.runs, id, prompt), settings);
      await refreshSessions();
      await refreshDetail(id);
      return runId;
    }),
    cancel: (runId: number) => command(() => backend.cancel(runId)),
    setMode: (mode: Mode) => command(() => backend.setMode(mode)),
    decidePlan: (card: PlanCard, decision: PlanDecision) => command(async () => {
      const plan = await backend.decidePlan(card.sessionId, card.id, decision);
      await refreshDetail(card.sessionId);
      return plan;
    }),
  };
}
