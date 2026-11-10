import { invoke } from '@tauri-apps/api/core';
import { providerModel } from './providerModels';
import { wireProvider } from './customProviders';
import { listen } from '@tauri-apps/api/event';
import type { LlmSettings } from './settings';

export interface ChatMessage { role: 'user' | 'assistant'; content: string; images?: string[] }

export type Mode = 'plan' | 'code' | 'confirm' | 'auto';
export interface RunMetrics { runId: number; sessionId: number; provider: string; model: string; inputTokens: number | null; outputTokens: number | null; cacheHitTokens?: number | null; elapsedMs: number; firstTokenMs: number | null }
export type RunStatus = 'running' | 'completed' | 'cancelled' | 'failed';

export interface RunView {
  id: number;
  sessionId: number;
  prompt: string;
  mode: Mode | 'execute';
  status: RunStatus;
  output: string;
  error: string | null;
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
export interface ToolCallView { runId: number; tool: string; detail: string; allowed: boolean; pending?: boolean }
export interface SessionDetail { session: SessionView; runs: RunView[]; plans: PlanCard[]; metrics?: RunMetrics[]; tools?: ToolCallView[] }

export type KernelEvent = { sequence: number } & (
  | { type: 'ready' }
  | { type: 'metrics'; metrics: RunMetrics }
  | { type: 'backend_error'; message: string }
  | { type: 'client_error'; message: string }
  | { type: 'mode_changed'; mode: Mode }
  | { type: 'status'; run: RunView }
  | { type: 'content'; runId: number; sessionId: number; token: string }
  | { type: 'plan_updated'; plan: PlanCard }
  | { type: 'sessions_changed' }
  | { type: 'tool_call'; runId: number; tool: string; phase: 'started' | 'finished' | 'failed' | 'blocked'; detail: string }
  | { type: 'reasoning'; runId: number; token: string }
);

export const backend = {
  snapshot: () => invoke<Snapshot>('get_snapshot'),
  history: () => invoke<RunView[]>('list_runs'),
  setMode: (mode: Mode) => invoke<void>('set_mode', { mode }),
  start: (sessionId: number, messages: ChatMessage[], settings: LlmSettings) => invoke<number>('stream_chat', {
    session_id: sessionId, messages, api_key: settings.apiKey, base_url: settings.baseUrl, model: settings.model,
    ...(settings.provider ? { provider: wireProvider(settings.provider) } : {}),
    ...(settings.reasoning ? { reasoning_effort: settings.reasoning } : {}),
    ...(providerModel(settings.provider ?? 'openai', settings.model)?.output ? { max_output: providerModel(settings.provider ?? 'openai', settings.model)!.output } : {}),
  }),
  cancel: (runId: number) => invoke<void>('cancel_agent', { runId }),
  sessions: () => invoke<SessionView[]>('list_sessions'),
  projects: () => invoke<ProjectView[]>('list_projects'),
  createProject: (name: string, path?: string) =>
    invoke<ProjectView>('create_project', { name, path: path ?? null }),
  setProjectArchived: (projectId: number, archived: boolean) =>
    invoke<ProjectView>('set_project_archived', { projectId, archived }),
  updateProjectName: (projectId: number, name: string) => invoke<void>('update_project_name', { projectId, name }),
  updateProjectPath: (projectId: number, path: string) => invoke<void>('update_project_path', { projectId, path }),
  deleteProject: (projectId: number) => invoke<void>('delete_project', { projectId }),
  createSession: (projectId?: number) =>
    invoke<SessionView>('create_session', { projectId: projectId ?? null }),
  setSessionArchived: (sessionId: number, archived: boolean) =>
    invoke<SessionView>('set_session_archived', { sessionId, archived }),
  renameSession: (sessionId: number, title: string) =>
    invoke<SessionView>('rename_session', { sessionId, title }),
  deleteSession: (sessionId: number) => invoke<void>('delete_session', { sessionId }),
  session: (sessionId: number) => invoke<SessionDetail>('get_session', { sessionId }),
  decidePlan: (sessionId: number, planId: number, decision: PlanDecision) =>
    invoke<PlanCard>('decide_plan', { sessionId, planId, decision }),
  listen: async (receive: (event: KernelEvent) => void) => {
    // Single channel: the backend emits normalized typed events (content,
    // reasoning, tool_call, status…) — no client-side reassembly.
    return listen<KernelEvent>('kernel-event', (event) => receive(event.payload));
  },
};
