/// Kernel channel API: snapshot, run lifecycle, mode, and the single
/// `kernel-event` subscription. All `invoke`/`listen` calls for the kernel
/// live here — nothing else may touch Tauri IPC for these.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { providerModel } from '../providerModels';
import { wireProvider } from '../customProviders';
import type { LlmSettings } from '../settings';
import type { ChatMessage, KernelEvent, Mode, RunView, Snapshot } from './types';

export const kernelApi = {
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
  resolveApproval: (runId: number, approved: boolean) => invoke<void>('resolve_tool_approval', { runId, approved }),
  abort: (runId: number) => invoke<void>('abort_chat', { runId }),
  listen: async (receive: (event: KernelEvent) => void) => {
    // Single channel: the backend emits normalized typed events (content,
    // reasoning, tool_call, status…) — no client-side reassembly.
    return listen<KernelEvent>('kernel-event', (event) => receive(event.payload));
  },
};
