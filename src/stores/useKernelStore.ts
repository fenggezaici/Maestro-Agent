/// Global kernel store (zustand): snapshot, sessions/projects, stream
/// side-channels and every backend command. The `useAgentStream` hook writes
/// events in; components read state and call actions — they never invoke IPC
/// themselves.
import { create } from 'zustand';

import { kernelApi } from '../api/kernel';
import { sessionsApi } from '../api/sessions';
import type {
  KernelEvent, Mode, PlanCard, PlanDecision, ProjectView, RunMetrics,
  SessionDetail, SessionView, Snapshot, ToolCallView,
} from '../api/types';
import {
  applyToolEvent, chatMessages, conversationRuns, errorMessage, initialSnapshot,
  kernelReducer, type KernelAction,
} from '../lib/conversation';
import { readLlmSettings, validateLlmSettings } from '../settings';

export interface KernelStore {
  state: Snapshot;
  sessions: SessionView[];
  projects: ProjectView[];
  selectedSessionId: number | null;
  detail: SessionDetail | null;
  loadingSession: boolean;
  error: string | null;
  pending: boolean;
  metrics: Record<number, RunMetrics>;
  liveTools: Record<number, ToolCallView[]>;
  liveReasoning: Record<number, string>;
  /** Reasoning phase duration (ms) once the first content token arrives. */
  liveReasoningMs: Record<number, number>;
  /** true while the agent stream connection is mounted. */
  connected: boolean;

  dispatch: (action: KernelAction) => void;
  /** Apply one live/buffered event's side-channel effects (metrics, tools,
   * reasoning, error). The reducer part runs through `dispatch` separately. */
  applySideEffects: (event: KernelEvent) => void;
  selectSession: (id: number | null) => void;
  selectProject: (projectId: number | null) => void;
  refreshSessions: () => Promise<void>;
  refreshProjects: () => Promise<void>;
  refreshDetail: (id?: number) => Promise<void>;
  setConnected: (connected: boolean) => void;
  /** Invalidate in-flight refreshes on unmount — mirrors the old mounted flag. */
  disconnect: () => void;

  createSession: (projectId?: number) => Promise<SessionView | undefined>;
  createProject: (name: string, path?: string) => Promise<{ project?: ProjectView; error?: string }>;
  setProjectArchived: (projectId: number, archived: boolean) => Promise<ProjectView | undefined>;
  updateProjectPath: (projectId: number, path: string) => Promise<void>;
  renameProject: (projectId: number, name: string) => Promise<void>;
  deleteProject: (projectId: number) => Promise<{ error?: string }>;
  setSessionArchived: (sessionId: number, archived: boolean) => Promise<SessionView | undefined>;
  renameSession: (sessionId: number, title: string) => Promise<SessionView | undefined>;
  deleteSession: (sessionId: number) => Promise<void>;
  start: (prompt: string, images?: string[]) => Promise<number | undefined>;
  cancel: (runId: number) => Promise<void>;
  resolveApproval: (runId: number, approved: boolean) => Promise<void>;
  setMode: (mode: Mode) => Promise<void>;
  decidePlan: (card: PlanCard, decision: PlanDecision) => Promise<PlanCard | undefined>;
}

// Non-reactive bookkeeping shared across StrictMode remounts of the
// connection hook. Lives outside the store — not render-visible.
let sessionsRevision = 0;
let projectsRevision = 0;
let detailRevision = 0;
let selectionRevision = 0;
let pendingCount = 0;
let selectedRef: number | null = null;
let activeProjectRef: number | null = null;
let mounted = false;
// Thinking-phase timing per run: start recorded on the first reasoning event,
// duration frozen on the first content event. Not reactive state.
const reasoningStart: Record<number, number> = {};

export const useKernelStore = create<KernelStore>()((set, get) => {
  const setError = (error: string | null) => set({ error });

  const selectSession = (id: number | null) => {
    selectionRevision++;
    if (selectedRef === id) return;
    selectedRef = id;
    detailRevision++;
    set({ detail: null, selectedSessionId: id, loadingSession: id !== null });
    if (id !== null) {
      void get().refreshDetail(id).catch((reason) => {
        if (mounted) setError(errorMessage(reason));
      });
    }
  };

  const refreshSessions = async () => {
    const revision = ++sessionsRevision;
    const items = await sessionsApi.sessions();
    if (!mounted || revision !== sessionsRevision) return;
    set({ sessions: items });
    const selected = items.find((item) => item.id === selectedRef);
    if (selected) activeProjectRef = selected.projectId;
    if (selectedRef === null || !selected) {
      const fallback = items.find((item) => !item.archived) ?? items[0];
      activeProjectRef = fallback?.projectId ?? null;
      selectSession(fallback?.id ?? null);
    }
  };

  const refreshProjects = async () => {
    const revision = ++projectsRevision;
    const items = await sessionsApi.projects();
    if (!mounted || revision !== projectsRevision) return;
    set({ projects: items });
    const active = items.find((item) => item.id === activeProjectRef);
    if (!active || active.archived) {
      activeProjectRef = items.find((item) => !item.archived)?.id ?? null;
    }
  };

  const refreshDetail = async (id = selectedRef) => {
    if (id === null || id !== selectedRef) return;
    const revision = ++detailRevision;
    set({ loadingSession: true });
    try {
      const result = await sessionsApi.session(id);
      if (mounted && revision === detailRevision && selectedRef === id) {
        set((s) => ({
          detail: result,
          metrics: { ...s.metrics, ...Object.fromEntries((result.metrics ?? []).map((v) => [v.runId, v])) },
        }));
      }
    } finally {
      if (mounted && revision === detailRevision) set({ loadingSession: false });
    }
  };

  const command = async <T,>(operation: () => Promise<T>): Promise<T | undefined> => {
    pendingCount++;
    set({ pending: pendingCount > 0, error: null });
    try {
      const result = await operation();
      const snapshot = await kernelApi.snapshot();
      if (mounted) get().dispatch({ type: 'snapshot', snapshot });
      return result;
    } catch (reason) {
      if (mounted) setError(errorMessage(reason));
      return undefined;
    } finally {
      pendingCount = Math.max(0, pendingCount - 1);
      if (mounted) set({ pending: pendingCount > 0 });
    }
  };

  return {
    state: initialSnapshot,
    sessions: [],
    projects: [],
    selectedSessionId: null,
    detail: null,
    loadingSession: false,
    error: null,
    pending: false,
    metrics: {},
    liveTools: {},
    liveReasoning: {},
    liveReasoningMs: {},
    connected: false,

    dispatch: (action) => set((s) => ({ state: kernelReducer(s.state, action) })),

    applySideEffects: (event) => {
      if (event.type === 'metrics') {
        set((s) => ({ metrics: { ...s.metrics, [event.metrics.runId]: event.metrics } }));
      }
      if (event.type === 'tool_call') {
        set((s) => ({ liveTools: applyToolEvent(s.liveTools, event) }));
      }
      if (event.type === 'reasoning') {
        if (reasoningStart[event.runId] === undefined) reasoningStart[event.runId] = Date.now();
        set((s) => ({ liveReasoning: { ...s.liveReasoning, [event.runId]: (s.liveReasoning[event.runId] ?? '') + event.token } }));
      }
      if (event.type === 'content' && reasoningStart[event.runId] !== undefined) {
        const elapsed = Date.now() - reasoningStart[event.runId];
        delete reasoningStart[event.runId];
        set((s) => ({ liveReasoningMs: { ...s.liveReasoningMs, [event.runId]: elapsed } }));
      }
      if (event.type === 'backend_error' || event.type === 'client_error') setError(event.message);
      if (event.type === 'status' && event.run.status === 'failed') setError(event.run.error ?? '模型请求失败');
      if (event.type === 'status' && event.run.status !== 'running') {
        // Stream ended: drop the ephemeral thinking block and any pending tool rows.
        delete reasoningStart[event.run.id];
        set((s) => {
          const { [event.run.id]: _reasoning, ...liveReasoning } = s.liveReasoning;
          const calls = s.liveTools[event.run.id];
          const liveTools = calls?.some((call) => call.pending)
            ? { ...s.liveTools, [event.run.id]: calls.map((call) => call.pending ? { ...call, pending: undefined, awaitingApproval: undefined } : call) }
            : s.liveTools;
          return { liveReasoning, liveTools };
        });
      }
    },

    selectSession,
    selectProject: (projectId) => {
      activeProjectRef = projectId;
      if (projectId === null) return;
      const target = get().sessions.find((session) => session.projectId === projectId);
      if (target) { selectSession(target.id); return; }
      // Project has no sessions yet — create one so the workspace switches over.
      void (async () => {
        try {
          const session = await sessionsApi.createSession(projectId);
          if (mounted) { await refreshSessions(); selectSession(session.id); }
        } catch (error) { if (mounted) setError(error instanceof Error ? error.message : 'Cannot create session'); }
      })();
    },

    refreshSessions,
    refreshProjects,
    refreshDetail,

    setConnected: (connected) => { mounted = connected; set({ connected }); },
    disconnect: () => {
      // The store is a module singleton, but hook mounts own fresh state —
      // reset everything so a remount behaves like the old per-mount hook.
      mounted = false;
      sessionsRevision++;
      projectsRevision++;
      detailRevision++;
      selectionRevision++;
      pendingCount = 0;
      selectedRef = null;
      activeProjectRef = null;
      set({
        state: initialSnapshot,
        sessions: [],
        projects: [],
        selectedSessionId: null,
        detail: null,
        loadingSession: false,
        error: null,
        pending: false,
        metrics: {},
        liveTools: {},
        liveReasoning: {},
        liveReasoningMs: {},
        connected: false,
      });
    },

    createSession: (projectId) => command(async () => {
      const revision = selectionRevision;
      const session = await sessionsApi.createSession(projectId ?? activeProjectRef ?? undefined);
      if (mounted && revision === selectionRevision) selectSession(session.id);
      await refreshSessions();
      return session;
    }),

    createProject: async (name, path) => {
      try {
        const project = await sessionsApi.createProject(name, path);
        await refreshProjects();
        activeProjectRef = project.id;
        return { project };
      } catch (reason) {
        const message = errorMessage(reason);
        if (mounted) setError(message);
        return { error: message };
      }
    },

    updateProjectPath: (projectId, path) => command(async () => {
      await sessionsApi.updateProjectPath(projectId, path);
      await refreshProjects();
    }),
    renameProject: (projectId, name) => command(async () => {
      await sessionsApi.updateProjectName(projectId, name);
      await refreshProjects();
    }),

    setProjectArchived: (projectId, archived) => command(async () => {
      const project = await sessionsApi.setProjectArchived(projectId, archived);
      await refreshProjects();
      return project;
    }),

    deleteProject: async (projectId) => {
      try {
        await sessionsApi.deleteProject(projectId);
        await refreshProjects();
        await refreshSessions();
        return {};
      } catch (reason) {
        const message = errorMessage(reason);
        if (mounted) setError(message);
        return { error: message };
      }
    },

    setSessionArchived: (sessionId, archived) => command(async () => {
      const session = await sessionsApi.setSessionArchived(sessionId, archived);
      await refreshSessions();
      if (archived && selectedRef === sessionId) {
        selectSession((await sessionsApi.sessions()).find((item) => !item.archived)?.id ?? null);
      }
      return session;
    }),

    renameSession: (sessionId, title) => command(async () => {
      const session = await sessionsApi.renameSession(sessionId, title);
      await refreshSessions();
      return session;
    }),

    deleteSession: (sessionId) => command(async () => {
      await sessionsApi.deleteSession(sessionId);
      await refreshSessions();
      if (selectedRef === sessionId) {
        selectSession((await sessionsApi.sessions()).find((item) => !item.archived)?.id ?? null);
      }
    }),

    start: (prompt, images) => command(async () => {
      const settings = validateLlmSettings(readLlmSettings());
      let id = selectedRef;
      if (id === null) {
        const revision = selectionRevision;
        const session = await sessionsApi.createSession(activeProjectRef ?? undefined);
        id = session.id;
        if (mounted && revision === selectionRevision) selectSession(id);
      }
      const context = await sessionsApi.session(id);
      const runId = await kernelApi.start(id, chatMessages(context.runs, id, prompt, images), settings);
      await refreshSessions();
      await refreshDetail(id);
      return runId;
    }),

    cancel: (runId) => {
      // Stop the UI immediately — drop the thinking block and pending tool
      // rows before the backend even acknowledges the abort.
      set((s) => {
        const { [runId]: _reasoning, ...liveReasoning } = s.liveReasoning;
        const calls = s.liveTools[runId];
        const liveTools = calls?.some((call) => call.pending)
          ? { ...s.liveTools, [runId]: calls.map((call) => call.pending ? { ...call, pending: undefined, awaitingApproval: undefined } : call) }
          : s.liveTools;
        return { liveReasoning, liveTools };
      });
      return command(() => kernelApi.abort(runId)).then(() => undefined);
    },
    resolveApproval: (runId, approved) => command(() => kernelApi.resolveApproval(runId, approved)).then(() => undefined),
    setMode: (mode) => command(() => kernelApi.setMode(mode)).then(() => undefined),
    decidePlan: (card, decision) => command(async () => {
      const plan = await sessionsApi.decidePlan(card.sessionId, card.id, decision);
      await refreshDetail(card.sessionId);
      return plan;
    }),
  };
});

export function conversationRunsFor(store: Pick<KernelStore, 'detail' | 'selectedSessionId' | 'state'>) {
  return conversationRuns(store.detail, store.selectedSessionId, store.state);
}
