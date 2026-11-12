import { useCallback, useEffect, useState } from 'react';
import {
  extensionsApi,
  type McpServerView,
  type RecommendedItem,
  type SkillView,
} from '../api/extensions';

export interface ExtensionsController {
  skills: SkillView[];
  servers: McpServerView[];
  recommendedSkills: RecommendedItem[];
  recommendedMcps: RecommendedItem[];
  mcpConfigText: string;
  loading: boolean;
  pending: string | null;
  importSummary: string | null;
  real: boolean;
  installErrors: Record<string, string>;
  isInstalled: (item: RecommendedItem) => boolean;
  refresh: () => Promise<void>;
  createSkill: (name: string, description: string) => Promise<string | null>;
  importSkill: (path: string) => Promise<string | null>;
  importSkillGithub: (repoUrl: string) => Promise<string | null>;
  addMcpServer: (name: string, command: string, args: string[]) => Promise<string | null>;
  setMcpTrust: (server: McpServerView, trust: boolean) => Promise<void>;
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
  const [importSummary, setImportSummary] = useState<string | null>(null);
  const [installErrors, setInstallErrors] = useState<Record<string, string>>({});
  const [installedRepos, setInstalledRepos] = useState<string[]>(() => {
    try { return JSON.parse(localStorage.getItem('maestro.extensions.installed') ?? '[]'); } catch { return []; }
  });
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
    importSummary, real: true, installErrors,
    isInstalled: (item) => item.kind === 'skill'
      ? installedRepos.includes(item.id) || skills.some((skill) => skill.name === item.id)
      : servers.some((server) => server.name === item.id),
    createSkill: (name, description) =>
      action('create', () => extensionsApi.createSkill(name, description), refresh),
    importSkill: (path) => action('import', async () => {
      const outcomes = await extensionsApi.importSkill(path);
      const imported = outcomes.filter((item) => item.status === 'imported');
      const failed = outcomes.filter((item) => item.status !== 'imported');
      if (imported.length === 0) {
        throw new Error(failed.map((item) => `${item.name}: ${item.error ?? '导入失败'}`).join('；') || '未导入任何技能');
      }
      setImportSummary(`已导入 ${imported.length} 个技能` +
        (failed.length ? `；跳过/失败 ${failed.length}：${failed.map((item) => item.name).join('、')}` : ''));
      return outcomes;
    }, refresh),
    importSkillGithub: (repoUrl) =>
      action('github', () => extensionsApi.importSkillGithub(repoUrl), refresh),
    addMcpServer: (name, command, args) =>
      action('mcp-add', () => extensionsApi.addMcpServer(name, command, args), refresh),
    setMcpTrust: async (server, trust) => {
      setPending(`mcp-trust-${server.name}`);
      try {
        await extensionsApi.setMcpServerTrust(server.name, trust);
        setServers((items) => items.map((item) => (item.name === server.name ? { ...item, trust } : item)));
      } catch (reason) {
        setServers((items) => items.map((item) => (item.name === server.name ? { ...item, error: errorMessage(reason) } : item)));
      } finally {
        setPending(null);
      }
    },
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
    installRecommended: async (item) => {
      const message = await action(`install-${item.id}`, async () => {
        if (item.kind === 'skill') {
          await extensionsApi.importSkillGithub(item.installPayload);
        } else {
          const config = JSON.parse(item.installPayload) as { command: string; args: string[] };
          await extensionsApi.addMcpServer(item.id, config.command, config.args ?? []);
        }
        await refresh();
      });
      setInstallErrors((errors) => ({ ...errors, [item.id]: message ?? '' }));
      if (!message && item.kind === 'skill') {
        setInstalledRepos((ids) => {
          const next = [...new Set([...ids, item.id])];
          try { localStorage.setItem('maestro.extensions.installed', JSON.stringify(next)); } catch { /* quota */ }
          return next;
        });
      }
      return message;
    },
  };
}
