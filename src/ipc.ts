import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { LlmSettings } from './settings';

export interface ChatMessage { role: 'user' | 'assistant'; content: string }
export interface ChatToken { sequence: number; runId: number; sessionId: number; token: string }
export interface ChatTerminal { sequence: number; run: RunView; message?: string }

export type Mode = 'plan' | 'execute';
export type RunStatus = 'running' | 'completed' | 'cancelled' | 'failed';

export interface RunView {
  id: number;
  sessionId: number;
  prompt: string;
  mode: Mode;
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
export interface SessionDetail { session: SessionView; runs: RunView[]; plans: PlanCard[] }

export type KernelEvent = { sequence: number } & (
  | { type: 'ready' }
  | { type: 'backend_error'; message: string }
  | { type: 'client_error'; message: string }
  | { type: 'mode_changed'; mode: Mode }
  | { type: 'status'; run: RunView }
  | { type: 'stream_token'; runId: number; sessionId?: number; token: string }
  | { type: 'plan_updated'; plan: PlanCard }
  | { type: 'sessions_changed' }
  | { type: 'tool_blocked'; runId: number; tool: string; reason: string }
);

export const backend = {
  snapshot: () => invoke<Snapshot>('get_snapshot'),
  history: () => invoke<RunView[]>('list_runs'),
  setMode: (mode: Mode) => invoke<void>('set_mode', { mode }),
  start: (sessionId: number, messages: ChatMessage[], settings: LlmSettings) => invoke<number>('stream_chat', {
    session_id: sessionId, messages, api_key: settings.apiKey, base_url: settings.baseUrl, model: settings.model,
  }),
  cancel: (runId: number) => invoke<void>('cancel_agent', { runId }),
  sessions: () => invoke<SessionView[]>('list_sessions'),
  createSession: () => invoke<SessionView>('create_session'),
  session: (sessionId: number) => invoke<SessionDetail>('get_session', { sessionId }),
  decidePlan: (sessionId: number, planId: number, decision: PlanDecision) =>
    invoke<PlanCard>('decide_plan', { sessionId, planId, decision }),
  listen: async (receive: (event: KernelEvent) => void) => {
    const stops: (() => void)[] = [];
    try {
      stops.push(await listen<KernelEvent>('kernel-event', (event) => receive(event.payload)));
      stops.push(await listen<ChatToken>('chat-token', ({ payload }) => {
        receive({ type: 'stream_token', ...payload });
      }));
      stops.push(await listen<ChatTerminal>('chat-end', ({ payload }) => {
        receive({ type: 'status', sequence: payload.sequence, run: payload.run });
      }));
      stops.push(await listen<ChatTerminal | { sequence: number; message: string } | string>('chat-error', ({ payload }) => {
        if (typeof payload === 'string') receive({ type: 'client_error', sequence: 0, message: payload });
        else if ('run' in payload) receive({ type: 'status', sequence: payload.sequence, run: payload.run });
        else receive({ type: 'backend_error', sequence: payload.sequence, message: payload.message });
      }));
      return () => stops.forEach((stop) => stop());
    } catch (error) {
      stops.forEach((stop) => stop());
      throw error;
    }
  },
};
