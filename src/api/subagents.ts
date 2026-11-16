import { invoke } from '@tauri-apps/api/core';
import type { SubagentView } from './types';

export const subagentsApi = {
  list: (sessionId: number) => invoke<SubagentView[]>('list_subagent_traces', { sessionId }),
  cancel: (runId: number, subagentId: string) => invoke<void>('cancel_subagent', { runId, subagentId }),
};
