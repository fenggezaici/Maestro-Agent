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
  toolBudget?: number | null;
  hidden: boolean;
  color?: string | null;
  source?: AgentSource;
}

export const listAgentsApi = (scope: AgentScope = 'all') => invoke<AgentDefinition[]>('list_agents', { scope });
export const saveAgentApi = (agent: AgentDefinition, scope: 'global' | 'project' = 'global') => invoke<void>('save_agent', { scope, agent });
export const deleteAgentApi = (name: string, scope: 'global' | 'project' = 'global') => invoke<void>('delete_agent', { scope, name });

/** 批量操作目标项；set_hidden 后端按 effective 视图的 source 定位实际文件。 */
export interface AgentItemRef { scope: string; name: string }
export const agentRef = (agent: AgentDefinition): AgentItemRef => ({ scope: agent.source ?? 'global', name: agent.name });
/** 批量启用/禁用（hidden 即禁用语义），逐项三态（updated/skipped/failed）。 */
export const setAgentsHiddenApi = (items: AgentItemRef[], hidden: boolean) => invoke<AgentImportOutcome[]>('set_agents_hidden', { items, hidden });
/** 批量删除，逐项三态（deleted/failed）；builtin 记 failed。 */
export const deleteAgentsApi = (items: AgentItemRef[]) => invoke<AgentImportOutcome[]>('delete_agents', { items });

/** 批量导入的单文件结果（status: imported/skipped/failed）。 */
export interface AgentImportOutcome {
  name: string;
  status: string;
  error?: string | null;
}

/** 本地目录批量导入 `*.md` → 全局 ~/.maestro/agents（agency-agents-zh 类格式转换）。 */
export const importAgentsDirApi = (path: string) => invoke<AgentImportOutcome[]>('import_agents_dir', { path });
/** 浅克隆 GitHub 仓库后批量导入其中全部 Agent 定义。 */
export const importAgentsGithubApi = (repoUrl: string) => invoke<AgentImportOutcome[]>('import_agents_github', { repoUrl });

/** 全局子智能体默认（~/.maestro/agents.json）：per-agent 字段留空时的第二级回退。 */
export interface AgentDefaults {
  model: string | null;
  steps: number | null;
  toolBudget: number | null;
  maxParallel: number | null;
  maxTotal: number | null;
}
export const agentDefaultsApi = () => invoke<AgentDefaults>('get_agent_defaults');
export const saveAgentDefaultsApi = (defaults: AgentDefaults) => invoke<void>('save_agent_defaults', { defaults });
