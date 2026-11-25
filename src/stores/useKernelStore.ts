/// Global kernel store (zustand): snapshot, sessions/projects, stream
/// side-channels and every backend command. The `useAgentStream` hook writes
/// events in; components read state and call actions — they never invoke IPC
/// themselves.
import { create } from 'zustand';

import { kernelApi } from '../api/kernel';
import { sessionsApi } from '../api/sessions';
import { subagentsApi } from '../api/subagents';
import type {
  ArchitectureTrace, KernelEvent, Mode, PlanCard, PlanDecision, PlanStep, ProjectView, RunMetrics,
  ApprovalDecision, CommandScope, ContextCompaction, QuestionAnswer, SessionDetail, SessionView, Snapshot, SubagentView, ToolCallView, UserQuestion,
} from '../api/types';
import {
  appendContent, appendReasoning, appendTool, applyToolEventIndexed, chatMessages, closeReasoning,
  conversationRuns, errorMessage, initialSnapshot, kernelReducer, type KernelAction, type TimelineItem,
} from '../lib/conversation';
import { hasProviderCredential, readLlmSettings, validateLlmSettings } from '../settings';

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
  /** Live and persisted child traces keyed by subagent id. */
  subagents: Record<string, SubagentView>;
  /** Active ask_user request, if any. */
  pendingQuestion: { runId: number; requestId: string; questions: UserQuestion[] } | null;
  compactions: ContextCompaction[];
  /// Bumped on every `workspace_changed` kernel event — workspace views
  /// subscribe and refresh their tree/preview without polling.
  workspaceTick: number;
  /// Bumped on every `extensions_changed` kernel event — the extensions page
  /// re-pulls MCP server state (startup restore completes per-server).
  extensionsTick: number;
  /// Untrusted `.maestro/` assets awaiting a trust decision (null = none).
  trustPrompt: { path: string; assets: string[] } | null;
  /** Ordered reasoning bursts and tool rows for the live run. */
  liveTimeline: Record<number, TimelineItem[]>;
  /** Run whose stop was requested but not yet acknowledged by the backend. */
  stoppingRunId: number | null;
  /** Prompt queued while the previous run is stopping. */
  queuedStart: { prompt: string; images?: string[] } | null;
  /** Task-tracker steps per session — the single source of truth. */
  taskPlans: Record<number, PlanStep[]>;
  /** Projection of `taskPlans` for the selected session. */
  currentPlan: PlanStep[];
  /** Latest applied write path, used to follow the live diff. */
  lastChangedPath: string | null;
  /** 最近一次架构沙盘轨迹：节点状态高亮 + 被拦截时的警告文案。 */
  architectureTrace: ArchitectureTrace | null;
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
  updateProjectPath: (projectId: number, path: string) => Promise<{ error?: string }>;
  renameProject: (projectId: number, name: string) => Promise<{ error?: string }>;
  deleteProject: (projectId: number) => Promise<{ error?: string }>;
  setSessionArchived: (sessionId: number, archived: boolean) => Promise<SessionView | undefined>;
  renameSession: (sessionId: number, title: string) => Promise<SessionView | undefined>;
  deleteSession: (sessionId: number) => Promise<void>;
  start: (prompt: string, images?: string[]) => Promise<number | undefined>;
  cancel: (runId: number) => Promise<void>;
  answerQuestion: (runId: number, requestId: string, answers: QuestionAnswer[]) => Promise<void>;
  compactSession: (sessionId: number) => Promise<void>;
  queueStart: (prompt: string, images?: string[]) => void;
  resolveApproval: (runId: number, decision: ApprovalDecision, commandScope?: CommandScope, feedback?: string) => Promise<void>;
  setMode: (mode: Mode) => Promise<void>;
  decidePlan: (card: PlanCard, decision: PlanDecision) => Promise<PlanCard | undefined>;
  /** Grant `.maestro/` trust for the workspace path; the `workspace_trusted`
   * event clears the prompt and rescans extension sources. */
  trustWorkspace: (path: string) => Promise<void>;
  /** Poll the live workspace for untrusted assets (mount/project change —
   * the `workspace_trust_required` event does not replay). */
  refreshTrustStatus: () => Promise<void>;
  dismissTrustPrompt: () => void;
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

export const useKernelStore = create<KernelStore>()((set, get) => {
  const setError = (error: string | null) => set({ error });

  const selectSession = (id: number | null) => {
    selectionRevision++;
    if (selectedRef === id) return;
    selectedRef = id;
    detailRevision++;
    set({
      detail: null,
      selectedSessionId: id,
      loadingSession: id !== null,
      currentPlan: id === null ? [] : (get().taskPlans[id] ?? []),
    });
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
      const [result, traces] = await Promise.all([
        sessionsApi.session(id),
        subagentsApi.list(id).catch(() => [] as SubagentView[]),
      ]);
      if (mounted && revision === detailRevision && selectedRef === id) {
        set((s) => ({
          detail: result,
          // Live metrics events are emitted only after the DB write, so for a
          // run seen live the in-memory entry is never older than the row —
          // let it win over the persisted snapshot instead of regressing the
          // stats line on every refresh.
          metrics: { ...Object.fromEntries((result.metrics ?? []).map((v) => [v.runId, v])), ...s.metrics },
          subagents: { ...Object.fromEntries(traces.map((trace) => [trace.id, trace])), ...s.subagents },
          compactions: result.compactions ?? [],
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
    subagents: {},
    pendingQuestion: null,
    compactions: [],
    workspaceTick: 0,
    extensionsTick: 0,
    trustPrompt: null,
    liveTimeline: {},
    stoppingRunId: null,
    queuedStart: null,
    taskPlans: {},
    currentPlan: [],
    lastChangedPath: null,
    architectureTrace: null,
    connected: false,

    dispatch: (action) => {
      // Hard stop: once the user pressed stop, ignore that run's answer tokens
      // immediately instead of waiting for the backend acknowledgement.
      if (action.type === 'event' && action.event.type === 'content' && get().stoppingRunId === action.event.runId) return;
      set((s) => {
        const state = kernelReducer(s.state, action);
        if (action.type === 'event' && action.event.type === 'content') {
          const { runId, token } = action.event;
          return { state, liveTimeline: { ...s.liveTimeline, [runId]: appendContent(s.liveTimeline[runId], token) } };
        }
        return { state };
      });
    },

    applySideEffects: (event) => {
      if (event.type === 'metrics') {
        set((s) => ({ metrics: { ...s.metrics, [event.metrics.runId]: event.metrics } }));
      }
      if (event.type === 'tool_call') {
        if (get().stoppingRunId === event.runId) return;
        if (event.phase === 'finished') {
          try {
            const value = JSON.parse(event.detail) as { kind?: string; path?: string };
            if (value.kind === 'write_result' && typeof value.path === 'string') set({ lastChangedPath: value.path });
          } catch { /* Non-write result. */ }
        }
        set((s) => {
          const { next, index } = applyToolEventIndexed(s.liveTools, event);
          return {
            liveTools: next,
            liveTimeline: { ...s.liveTimeline, [event.runId]: appendTool(s.liveTimeline[event.runId], index, Date.now()) },
          };
        });
      }
      if (event.type === 'context_compaction') {
        set((s) => ({ compactions: [...s.compactions.filter((item) => item.operationId !== event.compaction.operationId), event.compaction] }));
      }
      if (event.type === 'user_question') {
        set({ pendingQuestion: { runId: event.runId, requestId: event.requestId, questions: event.questions } });
      }
      if (event.type === 'subagent_started') {
        set((s) => ({ subagents: { ...s.subagents, [event.subagent.id]: { ...event.subagent, timeline: [] } } }));
      }
      if (event.type === 'subagent_tool') {
        set((s) => {
          const trace = s.subagents[event.subagentId];
          if (!trace) return {};
          const now = Date.now();
          const { next, index } = applyToolEventIndexed({ [event.runId]: trace.tools }, { ...event, type: 'tool_call' as const });
          const tools = next[event.runId] ?? trace.tools;
          const timeline = appendTool(closeReasoning(trace.timeline, now), index, now);
          return { subagents: { ...s.subagents, [event.subagentId]: { ...trace, tools, toolCount: tools.length, timeline } } };
        });
      }
      if (event.type === 'subagent_token') {
        set((s) => {
          const trace = s.subagents[event.subagentId];
          if (!trace) return {};
          const now = Date.now();
          if (event.kind === 'reasoning') {
            return { subagents: { ...s.subagents, [event.subagentId]: { ...trace, reasoning: trace.reasoning + event.token, timeline: appendReasoning(trace.timeline, event.token, now) } } };
          }
          return { subagents: { ...s.subagents, [event.subagentId]: { ...trace, content: trace.content + event.token, timeline: appendContent(closeReasoning(trace.timeline, now), event.token) } } };
        });
      }
      if (event.type === 'subagent_finished') {
        set((s) => {
          const trace = s.subagents[event.subagentId];
          if (!trace) return {};
          return {
            subagents: {
              ...s.subagents,
              [event.subagentId]: {
                ...trace,
                status: event.status,
                summary: event.summary,
                error: event.error,
                toolCount: event.toolCount,
                reasoningTokens: event.reasoningTokens ?? trace.reasoningTokens,
                firstTokenMs: event.firstTokenMs ?? trace.firstTokenMs,
                elapsedMs: event.elapsedMs ?? trace.elapsedMs,
                decodeMs: event.decodeMs ?? trace.decodeMs,
                decodeTokens: event.decodeTokens ?? trace.decodeTokens,
                lastInputTokens: event.lastInputTokens ?? trace.lastInputTokens,
                inputTokens: event.inputTokens,
                outputTokens: event.outputTokens,
                finishedAt: event.finishedAt,
                timeline: closeReasoning(trace.timeline, Date.now()),
              },
            },
          };
        });
      }
      if (event.type === 'workspace_changed') {
        set((s) => ({ workspaceTick: s.workspaceTick + 1 }));
      }
      if (event.type === 'extensions_changed') {
        set((s) => ({ extensionsTick: s.extensionsTick + 1 }));
      }
      if (event.type === 'workspace_trust_required') {
        set({ trustPrompt: { path: event.path, assets: event.assets } });
      }
      if (event.type === 'workspace_trusted') {
        // Trusted: clear the prompt and bump the tick so skills/commands and
        // workspace views rescan with project `.maestro/` assets included.
        set((s) => ({ trustPrompt: null, workspaceTick: s.workspaceTick + 1 }));
      }
      if (event.type === 'architecture_trace') {
        set({
          architectureTrace: {
            runId: event.runId,
            sessionId: event.sessionId,
            tool: event.tool,
            target: event.target,
            blocked: event.blocked,
            message: event.message ?? null,
            steps: event.steps,
          },
        });
      }
      if (event.type === 'task_plan_updated') {
        set((s) => ({
          taskPlans: { ...s.taskPlans, [event.sessionId]: event.steps },
          currentPlan: s.selectedSessionId === event.sessionId ? event.steps : s.currentPlan,
        }));
      }
      if (event.type === 'reasoning') {
        if (get().stoppingRunId === event.runId) return;
        set((s) => ({ liveTimeline: { ...s.liveTimeline, [event.runId]: appendReasoning(s.liveTimeline[event.runId], event.token, Date.now()) } }));
      }
      if (event.type === 'content') {
        // The answer started — the current thinking burst is finished.
        const closed = closeReasoning(get().liveTimeline[event.runId], Date.now());
        if (closed && closed !== get().liveTimeline[event.runId]) {
          set((s) => ({ liveTimeline: { ...s.liveTimeline, [event.runId]: closed } }));
        }
      }
      if (event.type === 'backend_error' || event.type === 'client_error') setError(event.message);
      if (event.type === 'status' && event.run.status === 'running') {
        // 新一轮 Run 开始：清理上一轮遗留的架构 Trace，避免陈旧拦截警告。
        set((s) => s.architectureTrace && s.architectureTrace.runId !== event.run.id ? { architectureTrace: null } : s);
      }
      if (event.type === 'status' && event.run.status === 'failed') setError(event.run.error ?? '模型请求失败');
      if (event.type === 'status' && event.run.status !== 'running') {
        const flushQueued = get().stoppingRunId === event.run.id;
        // Stream ended: seal the open thinking burst and settle pending rows.
        set((s) => {
          const calls = s.liveTools[event.run.id];
          const liveTools = calls?.some((call) => call.pending)
            ? { ...s.liveTools, [event.run.id]: calls.map((call) => call.pending ? { ...call, pending: undefined, awaitingApproval: undefined } : call) }
            : s.liveTools;
          const closed = closeReasoning(s.liveTimeline[event.run.id], Date.now());
          const liveTimeline = closed && closed !== s.liveTimeline[event.run.id]
            ? { ...s.liveTimeline, [event.run.id]: closed }
            : s.liveTimeline;
          return {
            liveTools,
            liveTimeline,
            stoppingRunId: s.stoppingRunId === event.run.id ? null : s.stoppingRunId,
            pendingQuestion: s.pendingQuestion?.runId === event.run.id ? null : s.pendingQuestion,
          };
        });
        if (event.run.status === 'completed') {
          const sessionId = event.run.sessionId;
          set((s) => {
            const steps = s.taskPlans[sessionId];
            if (!steps?.length) return {};
            const next = steps.map((step) => step.status === 'completed' ? step : { ...step, status: 'completed' as const });
            return {
              taskPlans: { ...s.taskPlans, [sessionId]: next },
              currentPlan: s.selectedSessionId === sessionId ? next : s.currentPlan,
            };
          });
        }
        if (flushQueued) {
          window.setTimeout(() => {
            const queued = get().queuedStart;
            if (!queued) return;
            set({ queuedStart: null });
            void get().start(queued.prompt, queued.images);
          }, 0);
        }
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
        subagents: {},
        pendingQuestion: null,
        liveTimeline: {},
        stoppingRunId: null,
        queuedStart: null,
        taskPlans: {},
        currentPlan: [],
        lastChangedPath: null,
        architectureTrace: null,
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

    updateProjectPath: async (projectId, path) => {
      try {
        await sessionsApi.updateProjectPath(projectId, path);
        await refreshProjects();
        return {};
      } catch (reason) {
        const message = errorMessage(reason);
        if (mounted) setError(message);
        return { error: message };
      }
    },
    renameProject: async (projectId, name) => {
      try {
        await sessionsApi.updateProjectName(projectId, name);
        await refreshProjects();
        return {};
      } catch (reason) {
        const message = errorMessage(reason);
        if (mounted) setError(message);
        return { error: message };
      }
    },

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
      set({ lastChangedPath: null, queuedStart: null });
      const settings = validateLlmSettings(readLlmSettings(), { requireKey: false });
      const provider = settings.provider ?? 'openai';
      if (provider !== 'ollama' && !hasProviderCredential(provider)) {
        throw new Error('请先在 Settings 中保存该 Provider 的 API Key');
      }
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

    queueStart: (prompt, images) => set({ queuedStart: { prompt, images } }),

    answerQuestion: async (runId, requestId, answers) => {
      try {
        await kernelApi.answerQuestion(runId, requestId, answers);
        set({ pendingQuestion: null, error: null });
      } catch (reason) {
        setError(errorMessage(reason));
      }
    },

    compactSession: async (sessionId) => {
      await command(() => kernelApi.compactSession(sessionId));
      await refreshDetail(sessionId);
    },

    cancel: (runId) => {
      // Stop the UI immediately — seal the open thinking burst and settle
      // pending tool rows before the backend even acknowledges the abort.
      set((s) => {
        const calls = s.liveTools[runId];
        const liveTools = calls?.some((call) => call.pending)
          ? { ...s.liveTools, [runId]: calls.map((call) => call.pending ? { ...call, pending: undefined, awaitingApproval: undefined } : call) }
          : s.liveTools;
        const closed = closeReasoning(s.liveTimeline[runId], Date.now());
        const liveTimeline = closed && closed !== s.liveTimeline[runId]
          ? { ...s.liveTimeline, [runId]: closed }
          : s.liveTimeline;
        const activeRun = s.state.activeRun?.id === runId
          ? { ...s.state.activeRun, status: 'cancelled' as const }
          : s.state.activeRun;
        const lastRun = s.state.lastRun?.id === runId
          ? { ...s.state.lastRun, status: 'cancelled' as const }
          : s.state.lastRun;
        return { state: { ...s.state, activeRun, lastRun }, liveTools, liveTimeline, stoppingRunId: runId };
      });
      // Send the abort immediately: the generic command wrapper would wait for
      // a snapshot and can be gated by unrelated in-flight commands, leaving
      // the stop button feeling dead.
      const abort = () => kernelApi.abort(runId).catch((reason) => { if (mounted) setError(errorMessage(reason)); });
      void abort();
      // Retry once if the run is still alive after 1.2s, then surface the
      // failure instead of leaving the UI silently streaming.
      window.setTimeout(() => {
        if (get().state.activeRun?.id === runId) void abort();
      }, 1200);
      // If the backend is stuck inside a long tool, settle the optimistic run
      // after 3s but keep stoppingRunId so late tokens stay suppressed.
      window.setTimeout(() => {
        const current = get();
        if (current.stoppingRunId !== runId || current.state.activeRun?.id !== runId) return;
        set({ state: { ...current.state, activeRun: null, lastRun: { ...current.state.activeRun, status: 'cancelled' } } });
      }, 3000);
      return Promise.resolve();
    },
    resolveApproval: (runId, decision, commandScope, feedback) => command(() => kernelApi.resolveApproval(runId, decision, commandScope, feedback)).then(() => undefined),
    setMode: (mode) => command(() => kernelApi.setMode(mode)).then(() => undefined),
    decidePlan: (card, decision) => command(async () => {
      const plan = await sessionsApi.decidePlan(card.sessionId, card.id, decision);
      await refreshDetail(card.sessionId);
      return plan;
    }),
    trustWorkspace: (path) => kernelApi.trustWorkspace(path),
    refreshTrustStatus: async () => {
      const status = await kernelApi.workspaceTrustStatus();
      if (!mounted) return;
      set({ trustPrompt: status });
    },
    dismissTrustPrompt: () => set({ trustPrompt: null }),
  };
});

export function conversationRunsFor(store: Pick<KernelStore, 'detail' | 'selectedSessionId' | 'state'>) {
  return conversationRuns(store.detail, store.selectedSessionId, store.state);
}
