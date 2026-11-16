import { invoke } from '@tauri-apps/api/core';

export type AgentSource = 'builtin' | 'global' | 'project';
export type AgentScope = 'all' | 'global' | 'project';

export interface AgentDefinition {
  name: string;
  description: string;
  category?: string | null;
  mode: string;
  model?: string | null;
  prompt: string;
  tools: string[];
  steps?: number | null;
  hidden: boolean;
  color?: string | null;
  source?: AgentSource;
}

export const listAgentsApi = (scope: AgentScope = 'all') => invoke<AgentDefinition[]>('list_agents', { scope });
export const saveAgentApi = (agent: AgentDefinition, scope: 'global' | 'project' = 'global') => invoke<void>('save_agent', { scope, agent });
export const deleteAgentApi = (name: string, scope: 'global' | 'project' = 'global') => invoke<void>('delete_agent', { scope, name });
