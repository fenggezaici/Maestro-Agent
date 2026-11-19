/// 节点库纯数据层：把引擎节点类型映射为用户心智分类（输入 / 权限 / 触发器 /
/// 流程关卡 / 结果 / 扩展文件夹 / 预置流程），供 Tab 节点库浮层与左侧分类树共用。
/// 每一项只是「节点类型 + 预置 data」的工厂描述，真正建节点仍走 architectureGraph 的工厂。
import { fuzzyScore } from '../commandCenter';
import type { ExtensionData, GraphNodeData } from '../api/architecture';
import { SANDBOX_TEMPLATES, type KnownExtensions, type NodeKind } from './architectureGraph';

export type LibraryNodeKind = NodeKind | 'template';

/// 条目详情卡：描述来自扩展自身（SKILL.md frontmatter / tools:list / 插件清单），
/// usage 说明该节点在规则图中的语义——两层分开，避免把「它是什么」和「它起什么作用」混在一起。
export interface ItemDetail {
  /** 用途描述（无原生描述的条目缺省）。 */
  description?: string;
  /** 元信息键值行：来源/路径/命令/工具数/只读/解析目标… */
  meta?: [string, string][];
  /** 放入图中后的语义（如「必选：模型须实际调用 load_skill」）。 */
  usage?: string;
}

export interface LibraryItem {
  /** 稳定 id（'trigger:prompt' / 'ext:skill:ui-guidelines' / 'tpl:xxx'），recent 与 drop 回退都靠它。 */
  id: string;
  title: string;
  subtitle?: string;
  /** 模糊匹配语料：中/英/别名拼接。 */
  keywords: string;
  /** 分类路径：['权限'] / ['扩展','MCP Servers','postgres']；顶层段即左侧树/浮层分组。 */
  category: string[];
  nodeKind: LibraryNodeKind;
  seed?: Partial<GraphNodeData>;
  extension?: ExtensionData;
  templateId?: string;
  /** 非空 = 禁用原因（如 'MCP 服务未运行'），仍展示但不可放置。 */
  disabled?: string;
  /** 卡片上的小徽标（如权限分类的 增/删/改/查）。 */
  badge?: string;
  /** hover/键盘选中时展示的详情卡（描述 + 元信息 + 图中语义）。 */
  detail?: ItemDetail;
}

export interface LibrarySection {
  label: string;
  items: LibraryItem[];
}

const item = (partial: Omit<LibraryItem, 'keywords'> & { keywords?: string }): LibraryItem => ({
  ...partial,
  keywords: `${partial.title} ${partial.subtitle ?? ''} ${partial.category.join(' ')} ${partial.keywords ?? ''}`,
});

// ---- 核心节点库（与引擎语义一一对应） -------------------------------------------

/// 各核心条目的图中语义，集中维护；key = item id。
const CORE_USAGE: Record<string, string> = {
  'trigger:prompt': '作为链路起点：本轮用户消息包含关键词时，下游关卡对本轮全部工具调用生效。',
  'perm:create': '作为链路起点：模型调用 write_file / apply_patch 创建文件时启用下游关卡。',
  'perm:modify': '作为链路起点：模型修改既有文件时启用下游关卡。',
  'perm:delete': '作为链路起点：删除文件（apply_patch）时启用；rm 类命令请改用「执行命令」触发器。',
  'perm:read': '作为链路起点：read_file / list_dir / grep 等查看类调用时启用下游关卡。',
  'trigger:command': '作为链路起点：run_command 命中时启用；目标栏填命令子串可收窄（如 "rm -rf"）。',
  'trigger:plan': '作为链路起点：update_plan 提交/更新计划时启用下游关卡。',
  'trigger:any': '作为链路起点：任意工具调用（含 MCP、子智能体）都启用下游关卡。',
  'gate:read-doc': '流程关卡：关联工具调用发生前，模型须先 read_file 指定文档，否则被拦截。',
  'gate:plan': '流程关卡：关联工具调用发生前，模型须先 update_plan，否则被拦截。',
  'gate:tool': '流程关卡：关联工具调用发生前，模型须先调用指定工具，否则被拦截。',
  'gate:subagent': '流程关卡：关联工具调用发生前，模型须先 spawn_subagent 拆分调研，否则被拦截。',
  'gate:deny-path': '流程关卡：目标路径命中禁止规则即拦截（如禁止改 src/db）。',
  'gate:deny-content': '流程关卡：写入内容命中禁止规则即拦截（如禁止引入 src/db）。',
  'gate:after-tool': '事后关卡：触发调用执行后欠下验收——模型须先调用指定工具，否则后续实质操作与结束回复都被拦。',
  'gate:after-plan': '事后关卡：触发调用执行后欠下验收——模型须先 update_plan，否则后续实质操作与结束回复都被拦。',
  'result:block': '链路终点：硬性拦截，提示语原样返回给模型，文件不落盘、命令不执行。留空自动按上游关卡生成；{call}/{target} 占位符手动引用。',
  'result:warn': '链路终点：拦截并以「架构警告」语气提示模型，黄色语义。留空自动按上游关卡生成；支持 {call}/{target} 占位符。',
  'result:remind': '链路终点但放行：调用照常执行，提示语拼进工具结果喂给模型——做「时刻提醒」不阻断流程。留空自动生成提醒；支持 {call} 占位符。',
};

function coreItems(): LibraryItem[] {
  return [
    item({
      id: 'trigger:prompt',
      title: '用户输入匹配',
      subtitle: '用户消息包含关键词时启用链路（本轮全部工具调用）',
      category: ['输入'],
      nodeKind: 'trigger',
      seed: { action: 'prompt' },
    }),
    item({
      id: 'perm:create',
      title: '新增/写入文件',
      subtitle: '模型调用 write_file / apply_patch 时命中',
      keywords: '新增 写入 创建 create write 文件',
      category: ['权限'],
      badge: '增',
      nodeKind: 'trigger',
      seed: { action: 'modify' },
    }),
    item({
      id: 'perm:modify',
      title: '修改文件',
      subtitle: '修改既有文件（write_file / apply_patch）',
      keywords: '修改 编辑 更新 modify edit update patch',
      category: ['权限'],
      badge: '改',
      nodeKind: 'trigger',
      seed: { action: 'modify' },
    }),
    item({
      id: 'perm:delete',
      title: '删除文件',
      subtitle: 'apply_patch 删除同走 write；rm 类命令请用「执行命令」',
      keywords: '删除 移除 delete remove rm',
      category: ['权限'],
      badge: '删',
      nodeKind: 'trigger',
      seed: { action: 'modify' },
    }),
    item({
      id: 'perm:read',
      title: '查看/检索文件',
      subtitle: 'read_file / list_dir / grep 等查看、获取类工具',
      keywords: '读取 查看 检索 查询 获取 read grep list search 查',
      category: ['权限'],
      badge: '查',
      nodeKind: 'trigger',
      seed: { action: 'read' },
    }),
    item({
      id: 'trigger:command',
      title: '执行命令',
      subtitle: 'run_command 命中；目标栏填命令子串可收窄',
      keywords: '命令 终端 command shell run',
      category: ['触发器'],
      nodeKind: 'trigger',
      seed: { action: 'command' },
    }),
    item({
      id: 'trigger:plan',
      title: '提交计划',
      subtitle: 'update_plan 生成/更新计划时命中',
      keywords: '计划 plan update_plan',
      category: ['触发器'],
      nodeKind: 'trigger',
      seed: { action: 'plan' },
    }),
    item({
      id: 'trigger:any',
      title: '任意工具',
      subtitle: '兜底触发：MCP 调用、子智能体等一切工具',
      keywords: '任意 any all 所有 兜底',
      category: ['触发器'],
      nodeKind: 'trigger',
      seed: { action: 'any' },
    }),
    item({
      id: 'gate:read-doc',
      title: '必须先读取文档',
      subtitle: '要求先 read_file 指定文档（targetFile 可改）',
      keywords: '文档 阅读 doc requirement read_file',
      category: ['流程关卡'],
      nodeKind: 'requirement',
      seed: { toolName: 'read_file', targetFile: 'docs/architecture.md', scope: 'session' },
    }),
    item({
      id: 'gate:plan',
      title: '必须先提交计划',
      subtitle: '要求先 update_plan',
      keywords: '计划 plan requirement update_plan',
      category: ['流程关卡'],
      nodeKind: 'requirement',
      seed: { updatePlan: true, scope: 'session' },
    }),
    item({
      id: 'gate:tool',
      title: '必须先调用工具',
      subtitle: '要求先调用指定工具（toolName 可改）',
      keywords: '工具 tool requirement call',
      category: ['流程关卡'],
      nodeKind: 'requirement',
      seed: { toolName: '', scope: 'session' },
    }),
    item({
      id: 'gate:subagent',
      title: '必须先拆分调研',
      subtitle: '要求先 spawn_subagent 拆分任务',
      keywords: '拆分 调研 子代理 subagent decompose',
      category: ['流程关卡'],
      nodeKind: 'requirement',
      seed: { toolName: 'spawn_subagent', scope: 'run' },
    }),
    item({
      id: 'gate:after-tool',
      title: '事后验收·必须调用工具',
      subtitle: '触发后模型必须先调用指定工具验收，否则后续操作被拦',
      keywords: '事后 验收 审查 测试 postcondition review test',
      category: ['流程关卡'],
      nodeKind: 'requirement',
      seed: { toolName: 'run_command', timing: 'after' },
    }),
    item({
      id: 'gate:after-plan',
      title: '事后验收·补交计划',
      subtitle: '触发后模型必须先 update_plan 补交计划',
      keywords: '事后 验收 计划 postcondition plan',
      category: ['流程关卡'],
      nodeKind: 'requirement',
      seed: { updatePlan: true, timing: 'after' },
    }),
    item({
      id: 'gate:deny-path',
      title: '架构限制·禁止路径',
      subtitle: '禁止操作匹配 denyRule 的路径',
      keywords: '禁止 限制 路径 restriction deny',
      category: ['流程关卡'],
      nodeKind: 'restriction',
      seed: { denyRule: '', scope: 'run' },
    }),
    item({
      id: 'gate:deny-content',
      title: '架构限制·禁止内容',
      subtitle: '写入内容命中 denyRule 即拦截（如禁止引入 src/db）',
      keywords: '禁止 内容 依赖 引入 restriction deny import',
      category: ['流程关卡'],
      nodeKind: 'restriction',
      seed: { denyRule: '', scope: 'session' },
    }),
    item({
      id: 'result:block',
      title: '⛔ Block 拦截',
      subtitle: '硬性拦截并把提示语返回给模型',
      keywords: '拦截 阻止 block deny stop 红',
      category: ['结果'],
      nodeKind: 'action',
      seed: { result: 'block' },
    }),
    item({
      id: 'result:warn',
      title: '⚠ Warn 警告拦截',
      subtitle: '拦截并以警告语气提示（黄色语义）',
      keywords: '警告 warn yellow 黄',
      category: ['结果'],
      nodeKind: 'action',
      seed: { result: 'warn', message: '' },
    }),
    item({
      id: 'result:remind',
      title: '🔔 Remind 提醒放行',
      subtitle: '放行调用，把提示语拼进工具结果持续提醒模型',
      keywords: '提醒 remind notify hint 提示 时刻提醒',
      category: ['结果'],
      nodeKind: 'action',
      seed: { result: 'remind', message: '' },
    }),
  ].map((entry) => ({ ...entry, detail: { usage: CORE_USAGE[entry.id] ?? '' } }));
}

// ---- 用途分类：Skill/MCP 按关键词归入中文用途文件夹 -------------------------------
// 描述多为英文，中文文件夹提供快速扫描层；命中计数归组，fallback「其它」。

const USAGE_CATEGORIES: [string, string[]][] = [
  ['代码审查·质量', ['review', 'lint', 'audit', 'quality', 'security', 'inspect', 'refactor', '审查']],
  ['前端·UI', ['ui', 'ux', 'css', 'tailwind', 'react', 'vue', 'frontend', 'component', 'design', 'responsive', '界面', '前端']],
  ['架构·设计', ['architect', 'architecture', 'pattern', 'diagram', 'schema', 'structure', '架构']],
  ['算法·数据', ['algorithm', 'data', 'parse', 'csv', 'json', 'sql', 'ml', 'ai', 'math', '算法', '数据']],
  ['测试·QA', ['test', 'e2e', 'qa', 'coverage', 'playwright', '测试']],
  ['文档·内容', ['doc', 'docs', 'readme', 'write', 'blog', 'markdown', 'spec', '文档', '写作']],
  ['规划·任务', ['plan', 'task', 'brainstorm', 'breakdown', 'roadmap', 'todo', '规划', '任务']],
  ['调试·诊断', ['debug', 'trace', 'diagnose', 'profile', 'log', 'error', '调试', '诊断']],
  ['运维·命令', ['deploy', 'docker', 'ci', 'git', 'shell', 'command', 'ops', 'devops', 'release', '运维', '部署']],
];

/// 按 name+text 关键词命中数归组；并列时取类别表中靠前项。
export function classifyUsage(name: string, text = ''): string {
  const hay = `${name} ${text}`.toLowerCase();
  let best = '其它';
  let bestHits = 0;
  for (const [label, keywords] of USAGE_CATEGORIES) {
    const hits = keywords.reduce((count, keyword) => count + (hay.includes(keyword.toLowerCase()) ? 1 : 0), 0);
    if (hits > bestHits) {
      bestHits = hits;
      best = label;
    }
  }
  return best;
}

// ---- 扩展节点库（动态来自已安装扩展） -------------------------------------------

function extensionItems(known: KnownExtensions): LibraryItem[] {
  const items: LibraryItem[] = [];
  for (const skill of known.skills) {
    items.push(item({
      id: `ext:skill:${skill.source}:${skill.name}`,
      title: `📜 ${skill.name}`,
      subtitle: skill.description || '必选后模型须实际调用 load_skill',
      keywords: `skill 技能 ${skill.name} ${skill.description ?? ''} ${classifyUsage(skill.name, skill.description)}`,
      badge: skill.source === 'project' ? '项目' : '全局',
      // 用途分类文件夹：按 名称+描述 关键词命中，英文描述也能归入中文目录。
      category: ['扩展', 'Skills', classifyUsage(skill.name, skill.description)],
      nodeKind: 'extension',
      extension: { kind: 'skill', name: skill.name, target: skill.name, requirement: 'required', scope: 'session' },
      detail: {
        description: skill.description || undefined,
        meta: [
          ['来源', skill.source === 'project' ? '项目 .maestro/skills' : '全局 ~/.maestro/skills'],
          ['路径', skill.path],
        ],
        usage: `必选：触发器命中后，模型须先实际调用 load_skill("${skill.name}")，否则关联工具调用被拦截。属性面板可改为「事后」做改后验收。`,
      },
    }));
  }
  for (const server of known.servers) {
    const disabled = server.disabled ? 'MCP 服务已禁用' : server.running ? undefined : 'MCP 服务未运行';
    const state = server.disabled ? '已禁用' : server.running ? `运行中 · ${server.tools.length} 个工具` : '未运行';
    const meta: [string, string][] = [
      ['命令', [server.command, ...server.args].join(' ')],
      ['状态', state],
      ['信任只读', server.trust ? '是' : '否'],
    ];
    if (server.error) meta.push(['错误', server.error]);
    items.push(item({
      id: `ext:mcp:${server.name}`,
      title: `🧩 ${server.name}`,
      subtitle: server.running ? `运行中 · ${server.tools.length} 个工具` : (disabled ?? ''),
      keywords: `mcp server 服务 ${server.name}`,
      category: ['扩展', 'MCP Servers', classifyUsage(server.name, server.command), server.name],
      nodeKind: 'extension',
      extension: { kind: 'mcp', name: server.name, target: server.name, requirement: 'required', scope: 'session' },
      disabled,
      detail: {
        meta,
        usage: `必选：触发器命中后，模型须调用 mcp__${server.name}__* 任一工具，否则关联工具调用被拦截。属性面板可改为「事后」做改后验收。`,
      },
    }));
    if (server.running) {
      for (const tool of server.tools) {
        items.push(item({
          id: `ext:mcp:${server.name}:${tool.name}`,
          title: `↳ ${tool.name}`,
          subtitle: tool.description || '必选后模型须实际调用该工具',
          keywords: `mcp tool 工具 ${server.name} ${tool.name} ${tool.description ?? ''}`,
          category: ['扩展', 'MCP Servers', classifyUsage(server.name, tool.description ?? tool.name), server.name],
          nodeKind: 'extension',
          extension: { kind: 'mcp', name: `${server.name}/${tool.name}`, target: server.name, tool: tool.name, requirement: 'required', scope: 'session' },
          detail: {
            description: tool.description || undefined,
            meta: [
              ['所属服务', server.name],
              ['读写性', tool.readOnly ? '只读' : '含副作用'],
            ],
            usage: `必选：触发器命中后，模型须实际调用 mcp__${server.name}__${tool.name}，否则关联工具调用被拦截。`,
          },
        }));
      }
    }
  }
  for (const plugin of known.plugins) {
    const meta: [string, string][] = plugin.error
      ? [['错误', plugin.error], ['路径', plugin.path]]
      : [
        ['解析为', `${plugin.kind === 'skill' ? 'Skill' : 'MCP'} → ${plugin.target ?? '?'}${plugin.tool ? `/${plugin.tool}` : ''}`],
        ['级别', plugin.condition === 'required' ? '必选' : '可选'],
        ['路径', plugin.path],
      ];
    items.push(item({
      id: `ext:plugin:${plugin.name}`,
      title: `🔌 ${plugin.name}`,
      subtitle: plugin.error ? `清单损坏：${plugin.error}` : (plugin.description || `${plugin.kind ?? '插件'} · ${plugin.condition}`),
      keywords: `plugin 插件 ${plugin.name} ${plugin.description ?? ''}`,
      badge: plugin.error ? '异常' : (plugin.kind === 'skill' ? 'Skill' : plugin.kind === 'mcp' ? 'MCP' : undefined),
      category: ['扩展', 'Plugins', classifyUsage(plugin.name, plugin.description ?? '')],
      nodeKind: 'extension',
      extension: { kind: 'plugin', name: plugin.name, target: plugin.name, requirement: 'required', scope: 'session' },
      disabled: plugin.error ?? undefined,
      detail: {
        description: plugin.description || undefined,
        meta,
        usage: '清单插件：运行时按声明解析为底层 Skill/MCP 依赖并强制实际调用。',
      },
    }));
  }
  return items;
}

// ---- 预置流程 ------------------------------------------------------------------

function templateItems(): LibraryItem[] {
  const usage = '整条规则链作为一组节点平移落到落点，可再逐节点调整参数。';
  return SANDBOX_TEMPLATES.map((template) => item({
    id: `tpl:${template.id}`,
    title: template.label,
    subtitle: template.description,
    keywords: `模板 流程 template ${template.label} ${template.description}`,
    category: ['预置流程'],
    nodeKind: 'template',
    templateId: template.id,
    detail: { description: template.description, usage },
  })).concat(item({
    id: 'tpl:sample',
    title: '验收示例图',
    subtitle: 'Trigger → Skill + MCP → Restriction → Action',
    keywords: '示例 验收 sample demo',
    category: ['预置流程'],
    nodeKind: 'template',
    templateId: 'sample',
    detail: { description: '覆盖完整验收路径的示例图：触发器 → Skill/MCP 必选扩展 → 限制 → 拦截结果。', usage },
  }));
}

// ---- 扩展详情反查（画布节点按 ext 取详情，不进序列化） ---------------------------

const extensionHints = new Map<string, ItemDetail>();

function extensionKey(ext: Pick<ExtensionData, 'kind' | 'target' | 'tool'>): string {
  return `${ext.kind}:${ext.target}:${ext.tool ?? ''}`;
}

/// 画布节点按 extension 反查详情卡；插件名与 target 一致时同 key 命中。
export function detailForExtension(ext: Pick<ExtensionData, 'kind' | 'target' | 'tool'> | undefined): ItemDetail | undefined {
  return ext ? extensionHints.get(extensionKey(ext)) : undefined;
}

export function buildLibraryItems(known: KnownExtensions): LibraryItem[] {
  const items = [...coreItems(), ...extensionItems(known), ...templateItems()];
  // 重建映射：扩展清单随扫描结果变化，旧 key 全部废弃。
  extensionHints.clear();
  for (const entry of items) {
    if (entry.extension && entry.detail) extensionHints.set(extensionKey(entry.extension), entry.detail);
  }
  return items;
}

// ---- 唤起上下文过滤 -------------------------------------------------------------

export interface PendingConnection {
  source: string;
  sourceHandle: string;
}

/// 连线松手唤起的库只显示「能作目标」的项：排除 trigger（不能作目标）与
/// template（无法把一条链并入单个 Handle）；cycle/重复连线在真正落线时复检。
export function itemFitsPending(itemEntry: LibraryItem, pending?: PendingConnection | null): boolean {
  if (!pending) return true;
  return itemEntry.nodeKind !== 'trigger' && itemEntry.nodeKind !== 'template';
}

// ---- 最近使用 ------------------------------------------------------------------

const RECENT_KEY = 'maestro.node-library.recent.v1';
const RECENT_LIMIT = 8;

export function readRecentIds(storage: Pick<Storage, 'getItem'> = localStorage): string[] {
  try {
    const raw: unknown = JSON.parse(storage.getItem(RECENT_KEY) ?? 'null');
    return Array.isArray(raw) ? raw.filter((id): id is string => typeof id === 'string').slice(0, RECENT_LIMIT) : [];
  } catch { return []; }
}

export function recordRecentId(id: string, storage: Pick<Storage, 'getItem' | 'setItem'> = localStorage) {
  try {
    const next = [id, ...readRecentIds(storage).filter((item) => item !== id)].slice(0, RECENT_LIMIT);
    storage.setItem(RECENT_KEY, JSON.stringify(next));
  } catch { /* best effort */ }
}

// ---- 过滤与分组 -----------------------------------------------------------------

/// query 为空：最近使用 + 分类分组（顶层段）；有 query：fuzzy 平铺并按顶层段分组。
export function filterLibrary(
  items: LibraryItem[],
  query: string,
  pending?: PendingConnection | null,
  recentIds: string[] = [],
): LibrarySection[] {
  const visible = items.filter((entry) => !entry.disabled && itemFitsPending(entry, pending));
  const byId = new Map(visible.map((entry) => [entry.id, entry]));
  const trimmed = query.trim();

  if (!trimmed) {
    const recent = recentIds.map((id) => byId.get(id)).filter((entry): entry is LibraryItem => Boolean(entry));
    const grouped = new Map<string, LibraryItem[]>();
    for (const entry of visible) {
      // 扩展类显示「扩展 / Skills / 用途」三级路径，普通条目仍为首段。
      const label = entry.category.length > 1 ? entry.category.slice(0, 3).join(' / ') : (entry.category[0] ?? '其它');
      const list = grouped.get(label) ?? [];
      list.push(entry);
      grouped.set(label, list);
    }
    return [
      ...(recent.length ? [{ label: '最近使用', items: recent }] : []),
      ...[...grouped].map(([label, groupItems]) => ({ label, items: groupItems })),
    ];
  }

  const scored = visible
    .map((entry) => ({ entry, score: fuzzyScore(trimmed, `${entry.title} ${entry.keywords}`) }))
    .filter((row): row is { entry: LibraryItem; score: number } => row.score !== null)
    .sort((a, b) => b.score - a.score)
    .slice(0, 40);
  const grouped = new Map<string, LibraryItem[]>();
  for (const { entry } of scored) {
    const label = entry.category.length > 1 ? entry.category.slice(0, 3).join(' / ') : (entry.category[0] ?? '其它');
    const list = grouped.get(label) ?? [];
    list.push(entry);
    grouped.set(label, list);
  }
  return [...grouped].map(([label, groupItems]) => ({ label, items: groupItems }));
}

// ---- 左侧分类树（递归分组，category 路径即层级） ----------------------------------

export interface LibraryTreeGroup {
  label: string;
  items: LibraryItem[];
  children: LibraryTreeGroup[];
}

export function libraryTree(items: LibraryItem[]): LibraryTreeGroup[] {
  const insert = (groups: LibraryTreeGroup[], entry: LibraryItem, depth: number) => {
    const label = entry.category[depth];
    if (label === undefined) {
      // category 耗尽 → 直接挂在当前层（顶层段 items）。
      return;
    }
    let group = groups.find((g) => g.label === label);
    if (!group) {
      group = { label, items: [], children: [] };
      groups.push(group);
    }
    if (depth === entry.category.length - 1) group.items.push(entry);
    else insert(group.children, entry, depth + 1);
  };
  const roots: LibraryTreeGroup[] = [];
  for (const entry of items) insert(roots, entry, 0);
  return roots;
}

/// 树中叶子总数（分组徽标计数）。
export function treeLeafCount(group: LibraryTreeGroup): number {
  return group.items.length + group.children.reduce((sum, child) => sum + treeLeafCount(child), 0);
}
