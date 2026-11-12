import { invoke } from '@tauri-apps/api/core';

export type SkillSource = 'global' | 'project';

export interface SkillView {
  name: string;
  description: string;
  source: SkillSource;
  path: string;
}

export interface McpServerView {
  name: string;
  command: string;
  args: string[];
  disabled: boolean;
  /** Explicit user opt-in for honoring this server's readOnlyHint. */
  trust: boolean;
  running: boolean;
  toolCount: number;
  error: string | null;
}

export interface SkillImportOutcome {
  name: string;
  status: 'imported' | 'skipped' | 'failed' | string;
  error: string | null;
}

export interface RecommendedItem {
  id: string;
  kind: 'skill' | 'mcp' | string;
  name: string;
  description: string;
  sourceUrl: string;
  installPayload: string;
}

export const extensionsApi = {
  scanSkills: () => invoke<SkillView[]>('scan_skills'),
  createSkill: (name: string, description: string) =>
    invoke<SkillView>('create_skill', { name, description }),
  importSkill: (path: string) => invoke<SkillImportOutcome[]>('import_skill', { path }),
  importSkillGithub: (repoUrl: string) =>
    invoke<SkillView[]>('import_skill_github', { repoUrl }),
  listMcpServers: () => invoke<McpServerView[]>('list_mcp_servers'),
  addMcpServer: (name: string, command: string, args: string[], trust = false) =>
    invoke<void>('add_mcp_server', { name, command, args, trust }),
  setMcpServerTrust: (name: string, trust: boolean) =>
    invoke<void>('set_mcp_server_trust', { name, trust }),
  importMcpJson: (path: string) => invoke<number>('import_mcp_json', { path }),
  startMcpServer: (name: string) => invoke<McpServerView>('start_mcp_server', { name }),
  stopMcpServer: (name: string) => invoke<void>('stop_mcp_server', { name }),
  removeMcpServer: (name: string) => invoke<void>('remove_mcp_server', { name }),
  recommendedSkills: () => invoke<RecommendedItem[]>('recommended_skills'),
  recommendedMcps: () => invoke<RecommendedItem[]>('recommended_mcps'),
  readMcpConfig: () => invoke<string>('read_mcp_config'),
};
