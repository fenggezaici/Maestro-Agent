import { useCallback, useEffect, useState } from 'react';
import {
  extensionsApi,
  type McpServerView,
  type RecommendedItem,
  type SkillView,
} from './extensions';

export interface ExtensionsController {
  skills: SkillView[];
  servers: McpServerView[];
  recommendedSkills: RecommendedItem[];
  recommendedMcps: RecommendedItem[];
  mcpConfigText: string;
  loading: boolean;
  pending: string | null;
  refresh: () => Promise<void>;
  createSkill: (name: string, description: string) => Promise<string | null>;
  importSkill: (path: string) => Promise<string | null>;
  importSkillGithub: (repoUrl: string) => Promise<string | null>;
  addMcpServer: (name: string, command: string, args: string[]) => Promise<string | null>;
  importMcpJson: (path: string) => Promise<string | null>;
  toggleMcpServer: (server: McpServerView, enable: boolean) => Promise<void>;
  removeMcpServer: (name: string) => Promise<string | null>;
  installRecommended: (item: RecommendedItem) => Promise<string | null>;
}

const errorMessage = (reason: unknown) =>
  reason instanceof Error ? reason.message : typeof reason === 'string' ? reason : '扩展操作失败';

export default function useExtensions(enabled: boolean): ExtensionsController {
  const [skills, setSkills] = useState<SkillView[]>([]);
  const [servers, setServers] = useState<McpServerView[]>([]);
  const [recommendedSkills, setRecommendedSkills] = useState<RecommendedItem[]>([]);
  const [recommendedMcps, setRecommendedMcps] = useState<RecommendedItem[]>([]);
  const [mcpConfigText, setMcpConfigText] = useState('');
  const [loading, setLoading] = useState(false);
  const [pending, setPending] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!enabled) return;
    setLoading(true);
    try {
      const [skillList, serverList, skills, mcps, config] = await Promise.all([
        extensionsApi.scanSkills(),
        extensionsApi.listMcpServers(),
        extensionsApi.recommendedSkills(),
        extensionsApi.recommendedMcps(),
        extensionsApi.readMcpConfig(),
      ]);
      setSkills(skillList);
      setServers(serverList);
      setRecommendedSkills(skills);
      setRecommendedMcps(mcps);
      setMcpConfigText(config);
    } catch {
      // The workbench surfaces errors through action results; listing stays empty.
    } finally {
      setLoading(false);
    }
  }, [enabled]);

  useEffect(() => { void refresh(); }, [refresh]);

  const action = useCallback(async <T,>(key: string, run: () => Promise<T>, after?: (value: T) => unknown): Promise<string | null> => {
    setPending(key);
    try {
      const value = await run();
      await after?.(value);
      return null;
    } catch (reason) {
      return errorMessage(reason);
    } finally {
      setPending(null);
    }
  }, []);

  return {
    skills, servers, recommendedSkills, recommendedMcps, mcpConfigText, loading, pending, refresh,
    createSkill: (name, description) =>
      action('create', () => extensionsApi.createSkill(name, description), refresh),
    importSkill: (path) => action('import', () => extensionsApi.importSkill(path), refresh),
    importSkillGithub: (repoUrl) =>
      action('github', () => extensionsApi.importSkillGithub(repoUrl), refresh),
    addMcpServer: (name, command, args) =>
      action('mcp-add', () => extensionsApi.addMcpServer(name, command, args), refresh),
    importMcpJson: (path) => action('mcp-import', () => extensionsApi.importMcpJson(path), refresh),
    toggleMcpServer: async (server, enable) => {
      setPending(`mcp-${server.name}`);
      try {
        const view = await (enable ? extensionsApi.startMcpServer(server.name) : (await extensionsApi.stopMcpServer(server.name), { ...server, running: false, toolCount: 0 }));
        setServers((items) => items.map((item) => (item.name === view.name ? { ...item, ...view } : item)));
      } catch (reason) {
        setServers((items) => items.map((item) => (item.name === server.name ? { ...item, error: errorMessage(reason) } : item)));
      } finally {
        setPending(null);
      }
    },
    removeMcpServer: (name) => action(`mcp-rm-${name}`, () => extensionsApi.removeMcpServer(name), refresh),
    installRecommended: (item) => action(`install-${item.id}`, async () => {
      if (item.kind === 'skill') {
        await extensionsApi.importSkillGithub(item.installPayload);
      } else {
        const config = JSON.parse(item.installPayload) as { command: string; args: string[] };
        await extensionsApi.addMcpServer(item.id, config.command, config.args ?? []);
      }
      await refresh();
    }),
  };
}
