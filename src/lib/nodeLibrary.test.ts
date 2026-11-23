import { describe, expect, it } from 'vitest';
import {
  buildLibraryItems, detailForExtension, filterLibrary, itemFitsPending, readRecentIds, recordRecentId,
  type LibraryItem,
} from './nodeLibrary';
import type { KnownExtensions } from './architectureGraph';
import type { McpServerView, PluginView, SkillView } from '../api/extensions';

const skills: SkillView[] = [
  { name: 'ui-guidelines', description: '前端规范', source: 'project', path: '/p' },
  { name: 'rust-rules', description: 'Rust 规范', source: 'global', path: '/g' },
];

const server = (running: boolean): McpServerView => ({
  name: 'postgres', command: 'pg', args: [], transport: 'stdio', url: null, headerKeys: [], timeoutMs: null, env: {}, disabled: false,
  autoStart: true, trust: false, running, toolCount: running ? 2 : 0,
  tools: running ? [
    { name: 'query', description: 'SQL 查询', inputSchema: {}, readOnly: true },
    { name: 'migrate', description: '迁移', inputSchema: {}, readOnly: false },
  ] : [],
  error: null,
});

const plugins: PluginView[] = [
  { name: 'db-plugin', description: 'DB 助手', kind: 'mcp', target: 'postgres', tool: 'query', condition: 'required', params: {}, source: 'project', path: '/x', error: null },
  { name: 'broken', description: '', kind: null, target: null, tool: null, condition: 'required', params: {}, source: 'project', path: '/y', error: '缺少 target' },
];

const known = (running = true): KnownExtensions => ({ skills, servers: [server(running)], plugins });

const byId = (items: LibraryItem[], id: string) => items.find((entry) => entry.id === id);

describe('buildLibraryItems', () => {
  it('核心分类齐全：输入/权限/触发器/流程关卡/结果/预置流程', () => {
    const items = buildLibraryItems({ skills: [], servers: [], plugins: [] });
    const top = new Set(items.map((entry) => entry.category[0]));
    for (const label of ['输入', '权限', '触发器', '流程关卡', '结果', '预置流程']) {
      expect(top.has(label), label).toBe(true);
    }
    expect(byId(items, 'trigger:prompt')?.seed?.action).toBe('prompt');
    expect(byId(items, 'perm:read')?.seed?.action).toBe('read');
    expect(byId(items, 'perm:create')?.seed?.action).toBe('modify');
    expect(byId(items, 'perm:delete')?.badge).toBe('删');
    expect(byId(items, 'perm:create')?.category).toEqual(['权限']);
    expect(byId(items, 'result:block')?.seed?.result).toBe('block');
  });

  it('扩展按用途文件夹分类：Skills/MCP/Plugins 按名称+描述归组，来源进 badge', () => {
    const items = buildLibraryItems(known());
    // ui-guidelines（前端规范）→ 前端·UI；来源「项目」进 badge 而不再是路径。
    const ui = byId(items, 'ext:skill:project:ui-guidelines');
    expect(ui?.category).toEqual(['扩展', 'Skills', '前端·UI']);
    expect(ui?.badge).toBe('项目');
    const rust = byId(items, 'ext:skill:global:rust-rules');
    expect(rust?.category).toEqual(['扩展', 'Skills', '其它']);
    expect(rust?.badge).toBe('全局');
    // MCP：库面板只到服务级，工具级条目不再拆出（工具颗粒度走属性面板「限定工具」）。
    expect(byId(items, 'ext:mcp:postgres')?.category).toEqual(['扩展', 'MCP Servers']);
    expect(byId(items, 'ext:mcp:postgres:query')).toBeUndefined();
    // Plugins：用途文件夹 + kind badge。
    const plugin = byId(items, 'ext:plugin:db-plugin');
    expect(plugin?.category).toEqual(['扩展', 'Plugins', '其它']);
    expect(plugin?.badge).toBe('MCP');
  });

  it('未运行 MCP 标记禁用；损坏插件禁用', () => {
    const items = buildLibraryItems(known(false));
    expect(byId(items, 'ext:mcp:postgres')?.disabled).toBeTruthy();
    expect(byId(items, 'ext:mcp:postgres:query')).toBeUndefined();
    expect(byId(items, 'ext:plugin:broken')?.disabled).toBe('缺少 target');
  });

  it('条目 detail：skill/tool/plugin 带真实描述与元信息，核心条目带 usage', () => {
    const items = buildLibraryItems(known());
    // skill：frontmatter 描述 + 来源/路径元信息 + 强制调用语义
    const skill = byId(items, 'ext:skill:project:ui-guidelines')?.detail;
    expect(skill?.description).toBe('前端规范');
    expect(skill?.meta?.some(([label]) => label === '来源')).toBe(true);
    expect(skill?.usage).toContain('load_skill');
    // mcp server：无原生描述 → meta 合成状态 + 全服务调用语义
    const server = byId(items, 'ext:mcp:postgres')?.detail;
    expect(server?.meta?.some(([label, value]) => label === '状态' && value.includes('运行中'))).toBe(true);
    expect(server?.usage).toContain('mcp__postgres__*');
    // mcp tool：库面板不拆工具条目，但工具 detail 仍可按 ext 反查（限定工具节点的详情卡）
    const tool = detailForExtension({ kind: 'mcp', target: 'postgres', tool: 'query' });
    expect(tool?.description).toBe('SQL 查询');
    expect(tool?.meta?.some(([label, value]) => label === '读写性' && value === '只读')).toBe(true);
    expect(tool?.usage).toContain('mcp__postgres__query');
    // plugin：清单描述 + 解析目标 + 错误项 meta 含错误
    expect(byId(items, 'ext:plugin:db-plugin')?.detail?.description).toBe('DB 助手');
    expect(byId(items, 'ext:plugin:db-plugin')?.detail?.meta?.some(([label]) => label === '解析为')).toBe(true);
    expect(byId(items, 'ext:plugin:broken')?.detail?.meta?.some(([label, value]) => label === '错误' && value === '缺少 target')).toBe(true);
    // 核心条目与模板均有 usage 语义行
    expect(byId(items, 'perm:create')?.detail?.usage).toContain('write_file');
    expect(byId(items, 'result:block')?.detail?.usage).toContain('硬性拦截');
    expect(items.find((entry) => entry.nodeKind === 'template')?.detail?.usage).toBeTruthy();
  });

  it('detailForExtension 按 kind:target:tool 反查详情', () => {
    buildLibraryItems(known());
    expect(detailForExtension({ kind: 'skill', target: 'ui-guidelines' })?.description).toBe('前端规范');
    expect(detailForExtension({ kind: 'mcp', target: 'postgres', tool: 'migrate' })?.description).toBe('迁移');
    expect(detailForExtension({ kind: 'mcp', target: 'postgres' })?.usage).toContain('mcp__postgres__*');
    expect(detailForExtension({ kind: 'plugin', target: 'db-plugin' })?.usage).toContain('清单插件');
    expect(detailForExtension({ kind: 'skill', target: '不存在' })).toBeUndefined();
    expect(detailForExtension(undefined)).toBeUndefined();
  });

  it('agents 生成「智能体」分类条目：hidden/非 subagent 被过滤', () => {
    const items = buildLibraryItems({
      ...known(),
      agents: [
        { name: 'explore', description: '只读调研', category: '研究', mode: 'subagent', prompt: '', tools: [], hidden: false, source: 'builtin' },
        { name: 'hidden-one', description: '', mode: 'subagent', prompt: '', tools: [], hidden: true, source: 'builtin' },
        { name: 'main-mode', description: '', mode: 'main', prompt: '', tools: [], hidden: false, source: 'global' },
      ],
    });
    const explore = byId(items, 'agent:explore');
    expect(explore?.nodeKind).toBe('subagent');
    expect(explore?.seed?.agent).toBe('explore');
    expect(explore?.category).toEqual(['智能体', '研究']);
    expect(explore?.badge).toBe('内置');
    expect(explore?.detail?.usage).toContain('装配');
    expect(byId(items, 'agent:hidden-one')).toBeUndefined();
    expect(byId(items, 'agent:main-mode')).toBeUndefined();
  });
});

describe('filterLibrary', () => {
  const items = buildLibraryItems(known());

  it('空查询返回最近使用 + 分组', () => {
    const sections = filterLibrary(items, '', null, ['trigger:prompt']);
    expect(sections[0].label).toBe('最近使用');
    expect(sections[0].items[0].id).toBe('trigger:prompt');
    // 浮层分组显示「扩展 / 子类 / 用途」三级路径
    expect(sections.some((s) => s.label === '扩展 / Skills / 前端·UI')).toBe(true);
  });

  it('fuzzy 查询命中标题/关键词并保留分组', () => {
    const sections = filterLibrary(items, 'skill', null, []);
    const flat = sections.flatMap((s) => s.items);
    expect(flat.length).toBeGreaterThan(0);
    expect(flat.every((entry) => entry.id.includes('skill') || entry.keywords.toLowerCase().includes('skill'))).toBe(true);
    const zh = filterLibrary(items, '读取', null, []).flatMap((s) => s.items);
    expect(zh.some((entry) => entry.id === 'perm:read' || entry.id === 'gate:read-doc')).toBe(true);
  });

  it('pending 连线过滤：排除 trigger 与 template，禁用项隐藏', () => {
    const pending = { source: 'n1', sourceHandle: 'out' };
    const flat = filterLibrary(items, '', pending, []).flatMap((s) => s.items);
    expect(flat.every((entry) => itemFitsPending(entry, pending))).toBe(true);
    expect(flat.some((entry) => entry.nodeKind === 'trigger')).toBe(false);
    expect(flat.some((entry) => entry.nodeKind === 'template')).toBe(false);
    const disabledServer = filterLibrary(buildLibraryItems(known(false)), '', pending, []).flatMap((s) => s.items);
    expect(disabledServer.some((entry) => entry.id === 'ext:mcp:postgres')).toBe(false);
  });
});

describe('recent', () => {
  const fakeStorage = () => {
    const map = new Map<string, string>();
    return { getItem: (k: string) => map.get(k) ?? null, setItem: (k: string, v: string) => void map.set(k, v) };
  };

  it('recordRecentId 去重前置并截断 8 条', () => {
    const storage = fakeStorage();
    for (let i = 0; i < 10; i++) recordRecentId(`i${i}`, storage);
    recordRecentId('i5', storage);
    const ids = readRecentIds(storage);
    expect(ids[0]).toBe('i5');
    expect(ids).toHaveLength(8);
  });
});
