import { useCallback, useEffect, useRef, useState } from 'react';
import {
  extensionsApi,
  type McpServerView,
  type PluginView,
  type RecommendedItem,
  type SkillSource,
  type SkillView,
} from '../api/extensions';
import {
  agentDefaultsApi,
  deleteAgentApi,
  deleteAgentsApi,
  importAgentsDirApi,
  importAgentsGithubApi,
  listAgentsApi,
  setAgentsHiddenApi,
  type AgentDefaults,
  type AgentDefinition,
  type AgentItemRef,
} from '../api/agents';
import { workspaceApi } from '../api/workspace';
import { useKernelStore } from '../stores/useKernelStore';

export interface ExtensionsController {
  skills: SkillView[];
  servers: McpServerView[];
  /** 声明式插件清单（`.maestro/plugins/*.json`）。 */
  plugins: PluginView[];
  /** 子智能体（builtin < global < project 合并视图）。 */
  agents: AgentDefinition[];
  /** 全局子智能体默认；加载失败时为 null（按无默认处理）。 */
  agentDefaults: AgentDefaults | null;
  recommendedSkills: RecommendedItem[];
  recommendedMcps: RecommendedItem[];
  mcpConfigText: string;
  loading: boolean;
  pending: string | null;
  /** First failed list call — a broken mcp.json must not blank the whole page. */
  loadError: string | null;
  importSummary: string | null;
  agentImportSummary: string | null;
  real: boolean;
  hasWorkspace: boolean;
  installErrors: Record<string, string>;
  isInstalled: (item: RecommendedItem) => boolean;
  refresh: () => Promise<void>;
  createSkill: (name: string, description: string, scope?: SkillSource) => Promise<string | null>;
  deleteSkill: (skill: SkillView) => Promise<string | null>;
  importSkill: (path: string) => Promise<string | null>;
  importSkillGithub: (repoUrl: string) => Promise<string | null>;
  addMcpServer: (name: string, command: string, args: string[], env?: Record<string, string>, trust?: boolean, transport?: string, url?: string, headers?: Record<string, string>, timeoutMs?: number) => Promise<string | null>;
  setMcpTrust: (server: McpServerView, trust: boolean) => Promise<void>;
  importMcpJson: (path: string) => Promise<string | null>;
  toggleMcpServer: (server: McpServerView, enable: boolean) => Promise<void>;
  removeMcpServer: (name: string) => Promise<string | null>;
  installRecommended: (item: RecommendedItem) => Promise<string | null>;
  deleteAgent: (agent: AgentDefinition) => Promise<string | null>;
  importAgentsDir: (path: string) => Promise<string | null>;
  importAgentsGithub: (repoUrl: string) => Promise<string | null>;
  /** 批量启用/禁用（hidden 即禁用语义）；逐项三态汇总进 agentImportSummary。 */
  setAgentsHidden: (items: AgentItemRef[], hidden: boolean) => Promise<string | null>;
  /** 批量删除；builtin 在后端记 failed。逐项三态汇总进 agentImportSummary。 */
  deleteAgents: (items: AgentItemRef[]) => Promise<string | null>;
}

const errorMessage = (reason: unknown) =>
  reason instanceof Error ? reason.message : typeof reason === 'string' ? reason : '扩展操作失败';

export default function useExtensions(enabled: boolean): ExtensionsController {
  const [skills, setSkills] = useState<SkillView[]>([]);
  const [servers, setServers] = useState<McpServerView[]>([]);
  const [plugins, setPlugins] = useState<PluginView[]>([]);
  const [agents, setAgents] = useState<AgentDefinition[]>([]);
  const [agentDefaults, setAgentDefaults] = useState<AgentDefaults | null>(null);
  const [recommendedSkills, setRecommendedSkills] = useState<RecommendedItem[]>([]);
  const [recommendedMcps, setRecommendedMcps] = useState<RecommendedItem[]>([]);
  const [mcpConfigText, setMcpConfigText] = useState('');
  const [importSummary, setImportSummary] = useState<string | null>(null);
  const [agentImportSummary, setAgentImportSummary] = useState<string | null>(null);
  const [installErrors, setInstallErrors] = useState<Record<string, string>>({});
  const [installedRepos, setInstalledRepos] = useState<string[]>(() => {
    try { return JSON.parse(localStorage.getItem('maestro.extensions.installed') ?? '[]'); } catch { return []; }
  });
  const [loading, setLoading] = useState(false);
  const [pending, setPending] = useState<string | null>(null);
  const [hasWorkspace, setHasWorkspace] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!enabled) return;
    setLoading(true);
    // 每个接口独立兜底：一个坏配置（如 mcp.json 解析失败）不能拖垮整页。
    let firstError: string | null = null;
    const track = <T,>(request: Promise<T>, fallback: T): Promise<T> =>
      request.catch((reason) => { firstError ??= errorMessage(reason); return fallback; });
    try {
      const [skillList, serverList, skills, mcps, config, info, pluginList, agentList, defaults] = await Promise.all([
        track(extensionsApi.scanSkills(), [] as SkillView[]),
        track(extensionsApi.listMcpServers(), [] as McpServerView[]),
        track(extensionsApi.recommendedSkills(), [] as RecommendedItem[]),
        track(extensionsApi.recommendedMcps(), [] as RecommendedItem[]),
        track(extensionsApi.readMcpConfig(), ''),
        workspaceApi.info().catch(() => ({ name: '', path: null })),
        extensionsApi.scanPlugins().catch(() => []),
        track(listAgentsApi('all'), [] as AgentDefinition[]),
        track(agentDefaultsApi(), null),
      ]);
      setSkills(skillList);
      setServers(serverList);
      setPlugins(pluginList);
      setAgents(agentList);
      setAgentDefaults(defaults);
      setRecommendedSkills(skills);
      setRecommendedMcps(mcps);
      setMcpConfigText(config);
      setHasWorkspace(Boolean(info.path));
      setLoadError(firstError);
    } finally {
      setLoading(false);
    }
  }, [enabled]);

  useEffect(() => { void refresh(); }, [refresh]);

  // 启动恢复在后台逐服务 spawn：extensions_changed 事件驱动重读，
  // 页面逐行转绿/落错误文案（否则打开页面看到的是恢复中的"已停止"快照）。
  const extensionsTick = useKernelStore((s) => s.extensionsTick);
  const lastTick = useRef(extensionsTick);
  useEffect(() => {
    if (!enabled || extensionsTick === lastTick.current) return;
    lastTick.current = extensionsTick;
    const timer = setTimeout(() => { void refresh(); }, 250);
    return () => clearTimeout(timer);
  }, [enabled, extensionsTick, refresh]);

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
    skills, servers, plugins, agents, agentDefaults, recommendedSkills, recommendedMcps, mcpConfigText, loading, pending, loadError, refresh,
    importSummary, agentImportSummary, real: true, hasWorkspace, installErrors,
    isInstalled: (item) => item.kind === 'skill'
      ? installedRepos.includes(item.id) || skills.some((skill) => skill.name === item.id)
      : servers.some((server) => server.name === item.id),
    createSkill: (name, description, scope = 'global') =>
      action('create', () => extensionsApi.createSkill(name, description, scope), refresh),
    deleteSkill: (skill) =>
      action(`delete-skill-${skill.name}`, () => extensionsApi.deleteSkill(skill.name, skill.source), refresh),
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
    addMcpServer: (name, command, args, env = {}, trust = false, transport, url, headers, timeoutMs) =>
      action('mcp-add', () => extensionsApi.addMcpServer(name, command, args, env, trust, transport, url, headers, timeoutMs), refresh),
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
        const view = enable
          ? await extensionsApi.startMcpServer(server.name)
          : await extensionsApi.stopMcpServer(server.name);
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
    deleteAgent: (agent) =>
      action(`delete-agent-${agent.name}`, () => deleteAgentApi(agent.name, agent.source === 'project' ? 'project' : 'global'), refresh),
    importAgentsDir: (path) => action('agents-import', async () => {
      const outcomes = await importAgentsDirApi(path);
      const imported = outcomes.filter((item) => item.status === 'imported').length;
      const skipped = outcomes.filter((item) => item.status === 'skipped').length;
      const failed = outcomes.filter((item) => item.status === 'failed');
      setAgentImportSummary(`已导入 ${imported} 个子智能体`
        + (skipped ? `；跳过 ${skipped}（同名已存在）` : '')
        + (failed.length ? `；失败 ${failed.length}：${failed.map((item) => `${item.name}（${item.error ?? '导入失败'}）`).join('、')}` : ''));
      return outcomes;
    }, refresh),
    importAgentsGithub: (repoUrl) => action('agents-import', async () => {
      const outcomes = await importAgentsGithubApi(repoUrl);
      const imported = outcomes.filter((item) => item.status === 'imported').length;
      const skipped = outcomes.filter((item) => item.status === 'skipped').length;
      const failed = outcomes.filter((item) => item.status === 'failed');
      setAgentImportSummary(`已导入 ${imported} 个子智能体`
        + (skipped ? `；跳过 ${skipped}（同名已存在）` : '')
        + (failed.length ? `；失败 ${failed.length}：${failed.map((item) => `${item.name}（${item.error ?? '导入失败'}）`).join('、')}` : ''));
      return outcomes;
    }, refresh),
    setAgentsHidden: (items, hidden) => action('bulk-agents', async () => {
      const outcomes = await setAgentsHiddenApi(items, hidden);
      const verb = hidden ? '禁用' : '启用';
      const updated = outcomes.filter((item) => item.status === 'updated').length;
      const failed = outcomes.filter((item) => item.status === 'failed');
      setAgentImportSummary(`已${verb} ${updated} 个子智能体`
        + (failed.length ? `；失败 ${failed.length}：${failed.map((item) => `${item.name}（${item.error ?? '操作失败'}）`).join('、')}` : ''));
      return outcomes;
    }, refresh),
    deleteAgents: (items) => action('bulk-agents', async () => {
      const outcomes = await deleteAgentsApi(items);
      const deleted = outcomes.filter((item) => item.status === 'deleted').length;
      const failed = outcomes.filter((item) => item.status === 'failed');
      setAgentImportSummary(`已删除 ${deleted} 个子智能体`
        + (failed.length ? `；失败 ${failed.length}：${failed.map((item) => `${item.name}（${item.error ?? '删除失败'}）`).join('、')}` : ''));
      return outcomes;
    }, refresh),
  };
}
