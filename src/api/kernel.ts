/// Kernel channel API: snapshot, run lifecycle, mode, and the single
/// `kernel-event` subscription. All `invoke`/`listen` calls for the kernel
/// live here — nothing else may touch Tauri IPC for these.
import { invoke } from '@tauri-apps/api/core';
import type { ApprovalDecision, ApprovalRuleView, CommandScope, QuestionAnswer } from './types';
import { listen } from '@tauri-apps/api/event';
import { providerModel } from '../providerModels';
import { resolveCapability } from '../modelCatalog';
import { wireProvider } from '../customProviders';
import type { LlmSettings } from '../settings';
import type { ChatMessage, KernelEvent, Mode, RunView, Snapshot } from './types';

export const kernelApi = {
  snapshot: () => invoke<Snapshot>('get_snapshot'),
  history: () => invoke<RunView[]>('list_runs'),
  setMode: (mode: Mode) => invoke<void>('set_mode', { mode }),
  start: (sessionId: number, messages: ChatMessage[], settings: LlmSettings) => invoke<number>('stream_chat', {
    session_id: sessionId, messages, base_url: settings.baseUrl, model: settings.model,
    ...(settings.provider ? { provider: wireProvider(settings.provider) } : {}),
    ...(settings.provider ? { credential_provider: settings.provider } : {}),
    ...(settings.reasoning ? { reasoning_effort: settings.reasoning } : {}),
    ...(providerModel(settings.provider ?? 'openai', settings.model)?.output ? { max_output: providerModel(settings.provider ?? 'openai', settings.model)!.output } : {}),
    ...(providerModel(settings.provider ?? 'openai', settings.model)?.context ? { context_window: providerModel(settings.provider ?? 'openai', settings.model)!.context } : {}),
    // 视觉门控进后端：read_file 读图据此决定注入图像块还是降级元数据文本。
    vision_capable: resolveCapability(settings.model, settings.provider ?? 'openai').vision,
  }),
  cancel: (runId: number) => invoke<void>('cancel_agent', { runId }),
  resolveApproval: (runId: number, decision: ApprovalDecision, commandScope?: CommandScope, feedback?: string) =>
    invoke<void>('resolve_tool_approval', { runId, decision, commandScope: commandScope ?? null, feedback: feedback ?? null }),
  listApprovalRules: (runId: number) => invoke<ApprovalRuleView[]>('list_approval_rules', { runId }),
  revokeApprovalRule: (runId: number, rule: ApprovalRuleView) =>
    invoke<boolean>('revoke_approval_rule', { runId, tool: rule.tool, pattern: rule.pattern, prefix: rule.prefix }),
  abort: (runId: number) => invoke<void>('abort_chat', { runId }),
  answerQuestion: (runId: number, requestId: string, answers: QuestionAnswer[]) =>
    invoke<void>('answer_user_question', { runId, requestId, answers }),
  compactSession: (sessionId: number) => invoke<void>('compact_session', { sessionId }),
  /// Untrusted `.maestro/` assets in the live workspace, if any — polled on
  /// mount because the `workspace_trust_required` event does not replay.
  workspaceTrustStatus: () => invoke<import('./types').WorkspaceTrustStatus | null>('workspace_trust_status'),
  trustWorkspace: (path: string) => invoke<void>('trust_workspace', { path }),
  listen: async (receive: (event: KernelEvent) => void) => {
    // Single channel: the backend emits normalized typed events (content,
    // reasoning, tool_call, status…) — no client-side reassembly.
    return listen<KernelEvent>('kernel-event', (event) => receive(event.payload));
  },
};
