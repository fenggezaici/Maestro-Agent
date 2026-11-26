/// 架构沙盘纯函数层：节点/边模型、连接规则、DAG 校验、序列化与预置模板。
/// 与 Rust `domain::architecture` 的校验语义保持一致（结构规则双端各查一次：
/// 画布给即时反馈，Rust 保存/执行时做权威判定）。
import type {
  ArchitectureGraph, ExtensionData, ExtensionKind, GraphEdge, GraphNode,
  GraphNodeData, SatisfyScope, SquadGraph, TriggerAction,
} from '../api/architecture';
import type { AgentDefinition } from '../api/agents';
import type { ArchitectureTraceStep } from '../api/types';
import type { McpServerView, PluginView, SkillView } from '../api/extensions';

export type NodeKind = GraphNode['type'];
export type TraceStatus = ArchitectureTraceStep['status'];

/// React Flow 画布上的节点（type='maestro' 统一自定义组件；
/// `data.nodeType` 承载规则语义并在序列化时写回 `type` 字段）。
export interface SandboxNode {
  id: string;
  type: 'maestro';
  position: { x: number; y: number };
  data: GraphNodeData & { nodeType: NodeKind };
}

/// React Flow 的 `Edge` 把 sourceHandle/targetHandle 声明为 `string | null`；
/// 序列化时再收窄回持久化契约（undefined = 缺省出口）。
export interface SandboxEdge extends Omit<GraphEdge, 'sourceHandle' | 'targetHandle'> {
  sourceHandle?: string | null;
  targetHandle?: string | null;
  /** React Flow 边类型；模板/示例插入时必须显式为 'maestro' 才能渲染语义色与流光。 */
  type?: string;
}

let counter = 0;
const nextId = () => `n${Date.now().toString(36)}${(counter++).toString(36)}`;

export function emptyGraph(): ArchitectureGraph {
  return { version: 1, nodes: [], edges: [] };
}

export const NODE_LABELS: Record<NodeKind, string> = {
  trigger: '触发器',
  requirement: '前置要求',
  restriction: '架构限制',
  action: '结果动作',
  extension: '扩展',
  subagent: '子智能体',
  squad: '军团',
};

export const ACTION_LABELS: Record<TriggerAction, string> = {
  modify: '修改文件',
  read: '读取文件',
  command: '执行命令',
  plan: '提交计划',
  any: '任意工具',
  prompt: '用户输入匹配',
  network: '网络访问',
};

export function createNode(kind: NodeKind, position: { x: number; y: number }, seed: Partial<GraphNodeData> = {}): SandboxNode {
  const data: SandboxNode['data'] = { nodeType: kind, ...seed };
  if (kind === 'trigger' && !data.action) data.action = 'modify';
  if (kind === 'action' && !data.result) data.result = 'block';
  if (kind === 'extension' && !data.extension) {
    data.extension = { kind: 'skill', name: '', target: '', requirement: 'required', scope: 'session', params: {} };
  }
  if (kind === 'subagent' && !data.agent) data.agent = 'explore';
  if (kind === 'squad' && !data.squad) data.squad = nextSquadName();
  return { id: nextId(), type: 'maestro', position, data };
}

/// 军团名缺省生成：team-N（字母/数字/-/_ 合法字符集）。
export function nextSquadName(taken: ReadonlySet<string> = new Set()): string {
  for (let i = 1; i < 1000; i++) {
    const name = `team-${i}`;
    if (!taken.has(name)) return name;
  }
  return `team-${Date.now().toString(36)}`;
}

/// 新军团子图骨架：general 指挥 --dispatch--> explore 必选成员。
/// 拖入 squad 物料即预置——开箱见两层 CTO 树；删掉 dispatch 边退化并行军团。
export function squadSkeleton(): { nodes: SandboxNode[]; edges: SandboxEdge[] } {
  const coordinator = createNode('subagent', { x: 0, y: 0 }, { agent: 'general', requirement: 'required' });
  const member = createNode('subagent', { x: 0, y: 170 }, { agent: 'explore', requirement: 'required' });
  return {
    nodes: [coordinator, member],
    edges: [{ id: nextId(), source: coordinator.id, target: member.id, sourceHandle: 'dispatch', targetHandle: 'in', type: 'maestro' }],
  };
}

export function createExtensionNode(
  position: { x: number; y: number },
  extension: ExtensionData,
): SandboxNode {
  return { id: nextId(), type: 'maestro', position, data: { nodeType: 'extension', extension: { scope: 'session', params: {}, ...extension } } };
}

/// 节点卡片副标题（物料区与画布共享的一行摘要）。
export function nodeSummary(node: SandboxNode | GraphNode): string {
  const data = node.data;
  const kind: NodeKind = 'nodeType' in data ? (data.nodeType as NodeKind) : (node as GraphNode).type;
  switch (kind) {
    case 'trigger':
      if (data.action === 'prompt') {
        return `用户输入包含 ${data.targetDir?.trim() ? `“${data.targetDir.trim()}”` : '任意内容'}`;
      }
      return `${ACTION_LABELS[data.action ?? 'modify']} · ${data.targetDir?.trim() || '任意目标'}`;
    case 'requirement': {
      const after = (data as { effectiveAfter?: boolean }).effectiveAfter ?? data.timing === 'after';
      const prefix = after ? '事后' : '先';
      if (data.updatePlan) return `${prefix} update_plan 提交计划`;
      if (data.toolName && data.targetFile) return `${prefix} ${data.toolName}（${data.targetFile}）`;
      if (data.toolName) return `${prefix}调用 ${data.toolName}`;
      if (data.targetFile) return `${prefix}读取 ${data.targetFile}`;
      return after ? '事后验收（未配置调用）' : '未配置前置条件';
    }
    case 'restriction':
      return `禁止 ${data.denyRule?.trim() || '（未配置）'}`;
    case 'action':
      return data.result === 'warn' ? '警告拦截' : data.result === 'remind' ? '提醒（放行附言）' : '拦截';
    case 'extension': {
      const ext = data.extension;
      if (!ext) return '未配置扩展';
      const label = ext.kind === 'mcp'
        ? (ext.tool ? `mcp__${ext.target}__${ext.tool}` : `MCP ${ext.target || '（未配置）'}`)
        : `${ext.kind === 'skill' ? 'Skill' : '插件'} ${ext.target || '（未配置）'}`;
      const after = ((data as { effectiveAfter?: boolean }).effectiveAfter ?? ext.timing === 'after') ? '事后验收 · ' : '';
      return `${after}${label} · ${ext.requirement === 'optional' ? '可选' : '必选'}`;
    }
    case 'subagent':
      return `type=${data.agent?.trim() || '（未配置）'} · 装配终点`;
    case 'squad':
      return `军团 ${data.squad?.trim() || '（未配置）'} · spawn_subagent 可调用`;
  }
}

// ---- 有效时机（事后语义沿 pass 边继承，与 Rust effective_after_set 同款规则） ----------

const isGateKind = (kind: NodeKind | undefined) => kind === 'requirement' || kind === 'extension';

const rawTiming = (node: SandboxNode): 'before' | 'after' | undefined =>
  node.data.nodeType === 'extension' ? node.data.extension?.timing : node.data.timing;

/// 种子 = 显式 timing:'after' 的关卡；沿 pass 出边传播到未设时机的关卡。
/// 显式 'before' 钉住不继承；Restriction 不传播。
export function effectiveAfterSet(nodes: SandboxNode[], edges: SandboxEdge[]): Set<string> {
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const set = new Set<string>();
  const queue: string[] = [];
  for (const node of nodes) {
    if (isGateKind(node.data.nodeType) && rawTiming(node) === 'after') {
      set.add(node.id);
      queue.push(node.id);
    }
  }
  while (queue.length > 0) {
    const id = queue.pop()!;
    for (const edge of edges) {
      if (edge.source !== id) continue;
      if ((edge.sourceHandle ?? 'pass') !== 'pass') continue;
      const child = byId.get(edge.target);
      // 显式时机不再继承；after 本身已在集合中。
      if (!child || !isGateKind(child.data.nodeType) || rawTiming(child) !== undefined) continue;
      if (!set.has(child.id)) {
        set.add(child.id);
        queue.push(child.id);
      }
    }
  }
  return set;
}

// ---- 连接规则 -----------------------------------------------------------------

export interface ConnectionCandidate {
  source: string;
  target: string;
  sourceHandle?: string | null;
  targetHandle?: string | null;
}

const handlesFor = (node: SandboxNode): string[] => {
  switch (node.data.nodeType) {
    case 'trigger': return ['out'];
    case 'requirement':
    case 'restriction': return ['pass', 'fail'];
    case 'extension':
      return node.data.extension?.requirement === 'optional' ? ['pass'] : ['pass', 'fail'];
    case 'action':
    case 'subagent':
    case 'squad': return [];
  }
};

/// BFS：target 是否能沿有向边回到 source（加上该候选边即成环）。
export function wouldCreateCycle(nodes: SandboxNode[], edges: SandboxEdge[], source: string, target: string): boolean {
  if (source === target) return true;
  const outgoing = new Map<string, string[]>();
  for (const edge of edges) {
    const list = outgoing.get(edge.source) ?? [];
    list.push(edge.target);
    outgoing.set(edge.source, list);
  }
  const stack = [target];
  const seen = new Set<string>();
  while (stack.length) {
    const id = stack.pop()!;
    if (id === source) return true;
    if (seen.has(id)) continue;
    seen.add(id);
    stack.push(...(outgoing.get(id) ?? []));
  }
  return false;
}

/// 整图 DAG 检测：三色 DFS，灰节点被再次访问即成环。
export function hasCycle(nodes: SandboxNode[], edges: SandboxEdge[]): boolean {
  const outgoing = new Map<string, string[]>();
  for (const edge of edges) {
    const list = outgoing.get(edge.source) ?? [];
    list.push(edge.target);
    outgoing.set(edge.source, list);
  }
  const WHITE = 0, GRAY = 1, BLACK = 2;
  const color = new Map<string, number>(nodes.map((node) => [node.id, WHITE]));
  const visit = (id: string): boolean => {
    color.set(id, GRAY);
    for (const next of outgoing.get(id) ?? []) {
      const state = color.get(next);
      if (state === GRAY) return true;
      if (state === WHITE && visit(next)) return true;
    }
    color.set(id, BLACK);
    return false;
  };
  return nodes.some((node) => color.get(node.id) === WHITE && visit(node.id));
}

/// `isValidConnection` 的实现：返回 {ok, reason}，reason 用于即时提示。
export function isConnectionAllowed(
  nodes: SandboxNode[],
  edges: SandboxEdge[],
  candidate: ConnectionCandidate,
): { ok: boolean; reason?: string } {
  const source = nodes.find((node) => node.id === candidate.source);
  const target = nodes.find((node) => node.id === candidate.target);
  if (!source || !target) return { ok: false, reason: '节点不存在' };
  if (source.id === target.id) return { ok: false, reason: '不能连接自身' };
  if (source.data.nodeType === 'action') return { ok: false, reason: 'Action 是终点，不能作为起点' };
  if (source.data.nodeType === 'subagent') return { ok: false, reason: 'Subagent 是装配终点，不能作为起点' };
  if (source.data.nodeType === 'squad') return { ok: false, reason: 'Squad 是装配终点，不能作为起点' };
  if (target.data.nodeType === 'trigger') return { ok: false, reason: 'Trigger 是起点，不能作为目标' };
  // 装配终点两类入边：extension pass = 授权；trigger out / 条件 pass = 派发义务。
  if ((target.data.nodeType === 'subagent' || target.data.nodeType === 'squad')
    && !['extension', 'trigger', 'requirement', 'restriction'].includes(source.data.nodeType)) {
    return { ok: false, reason: `${NODE_LABELS[target.data.nodeType]} 只接受 扩展(pass)/触发器(out)/条件(pass) 入边` };
  }
  if ((candidate.targetHandle ?? 'in') !== 'in') return { ok: false, reason: '目标仅接受入口 Handle' };
  // 缺省出口按 source 类型取默认 Handle（与 Rust None 语义一致）：
  // trigger→out，其余→pass。
  const handle = candidate.sourceHandle ?? (source.data.nodeType === 'trigger' ? 'out' : 'pass');
  const allowed = handlesFor(source);
  if (!allowed.includes(handle)) {
    return {
      ok: false,
      reason: source.data.nodeType === 'extension' && handle === 'fail'
        ? '可选扩展不允许 fail 出边'
        : '该 Handle 不支持从此节点出发',
    };
  }
  // 装配连线仅表达「授予/义务」：Subagent/Squad 入边不接受 fail 出口。
  if ((target.data.nodeType === 'subagent' || target.data.nodeType === 'squad') && handle === 'fail') {
    return { ok: false, reason: '→ 装配终点的连线不能使用 fail 出口' };
  }
  // 存储边的缺省出口同样按 source 类型归一化（trigger→out，其余→pass）再比较。
  const defaultHandle = source.data.nodeType === 'trigger' ? 'out' : 'pass';
  if (edges.some((edge) => edge.source === source.id && edge.target === target.id && (edge.sourceHandle ?? defaultHandle) === handle)) {
    return { ok: false, reason: '重复连线' };
  }
  if (wouldCreateCycle(nodes, edges, source.id, target.id)) {
    return { ok: false, reason: '会形成环（规则图必须是 DAG）' };
  }
  return { ok: true };
}

/// 边的语义色：fail → 红；目标为 warn Action → 黄；dispatch（指挥→成员）
/// 与派发义务（非扩展来源→装配终点）→ 紫；pass/out → 默认。
export function edgeTone(edge: SandboxEdge, nodes: SandboxNode[]): 'fail' | 'warn' | 'dispatch' | 'default' {
  const handle = edge.sourceHandle ?? 'out';
  if (handle === 'fail') return 'fail';
  if (handle === 'dispatch') return 'dispatch';
  const source = nodes.find((node) => node.id === edge.source);
  const target = nodes.find((node) => node.id === edge.target);
  if (source && target
    && (target.data.nodeType === 'subagent' || target.data.nodeType === 'squad')
    && source.data.nodeType !== 'extension') {
    return 'dispatch';
  }
  if (target?.data.nodeType === 'action') {
    return target.data.result === 'warn' ? 'warn' : 'fail';
  }
  return 'default';
}

// ---- 军团内部画布（subnet 子沙盘）：节点仅 subagent/extension；调度用 dispatch 出口 ----

/// 指挥节点 id：dispatch 出边源（全图至多一个，校验保证）。
export function squadCoordinatorId(nodes: SandboxNode[], edges: SandboxEdge[]): string | null {
  const edge = edges.find((item) => (item.sourceHandle ?? '') === 'dispatch');
  return edge?.source ?? null;
}

/// 军团内连线规则（镜像 Rust validate_squad）：
/// extension --pass--> 成员；成员 --dispatch--> 成员；指挥唯一且不可被 dispatch 指向；DAG。
export function isSquadConnectionAllowed(
  nodes: SandboxNode[],
  edges: SandboxEdge[],
  candidate: ConnectionCandidate,
): { ok: boolean; reason?: string } {
  const source = nodes.find((node) => node.id === candidate.source);
  const target = nodes.find((node) => node.id === candidate.target);
  if (!source || !target) return { ok: false, reason: '节点不存在' };
  if (source.id === target.id) return { ok: false, reason: '不能连接自身' };
  if (target.data.nodeType !== 'subagent') return { ok: false, reason: '军团内只能指向成员（子智能体节点）' };
  if ((candidate.targetHandle ?? 'in') !== 'in') return { ok: false, reason: '目标仅接受入口 Handle' };
  if (source.data.nodeType === 'extension') {
    const handle = candidate.sourceHandle ?? 'pass';
    if (handle !== 'pass') return { ok: false, reason: '扩展 → 成员只能使用 pass 出口' };
  } else if (source.data.nodeType === 'subagent') {
    const handle = candidate.sourceHandle ?? 'dispatch';
    if (handle !== 'dispatch') return { ok: false, reason: '成员的调度出边只能使用 dispatch 出口' };
    const coordinator = squadCoordinatorId(nodes, edges);
    if (coordinator && coordinator !== source.id) {
      return { ok: false, reason: '军团至多一个指挥（已有 dispatch 出边源）' };
    }
    if (target.id === coordinator || edges.some((edge) => (edge.sourceHandle ?? '') === 'dispatch' && edge.target === source.id)) {
      return { ok: false, reason: '指挥不能被 dispatch 指向' };
    }
  } else {
    return { ok: false, reason: '军团内只允许 子智能体/扩展 节点' };
  }
  const handle = candidate.sourceHandle ?? (source.data.nodeType === 'extension' ? 'pass' : 'dispatch');
  if (edges.some((edge) => edge.source === source.id && edge.target === target.id && (edge.sourceHandle ?? handle) === handle)) {
    return { ok: false, reason: '重复连线' };
  }
  if (wouldCreateCycle(nodes, edges, source.id, target.id)) {
    return { ok: false, reason: '会形成环（军团内部必须是 DAG）' };
  }
  return { ok: true };
}

/// 军团内容限制：只允许 subagent/extension 节点（供放置过滤）。
export const SQUAD_NODE_KINDS: ReadonlySet<NodeKind> = new Set(['subagent', 'extension']);

/// 军团内部校验（镜像 Rust validate_squad）：节点类型、成员数、边规则、指挥唯一、DAG。
export function validateSquadDraft(name: string, nodes: SandboxNode[], edges: SandboxEdge[]): string[] {
  const errors: string[] = [];
  if (nodes.length > 48) errors.push(`军团 ${name} 节点数超过 48 上限`);
  if (edges.length > 96) errors.push(`军团 ${name} 边数超过 96 上限`);
  let members = 0;
  const ids = new Set<string>();
  for (const node of nodes) {
    if (ids.has(node.id)) errors.push(`军团 ${name} 节点 id 重复：${node.id}`);
    ids.add(node.id);
    if (!SQUAD_NODE_KINDS.has(node.data.nodeType)) {
      errors.push(`军团 ${name} 只允许 子智能体/扩展 节点（${node.data.nodeType} 非法）`);
      continue;
    }
    if (node.data.nodeType === 'subagent') {
      members += 1;
      const agent = node.data.agent?.trim() ?? '';
      if (!agent) errors.push(`军团 ${name} 成员 ${node.id} 缺少 agent 类型名`);
      else if (agent.length > 64 || !/^[A-Za-z0-9_-]+$/.test(agent)) {
        errors.push(`军团 ${name} 成员 ${node.id} 的 agent 名必须为字母/数字/-/_`);
      }
    }
    if (node.data.nodeType === 'extension' && !node.data.extension?.target?.trim()) {
      errors.push(`军团 ${name} 扩展节点 ${node.id} 缺少 target`);
    }
  }
  if (members === 0) errors.push(`军团 ${name} 没有成员子智能体`);
  if (members > 12) errors.push(`军团 ${name} 成员数超过 12 上限`);
  for (const edge of edges) {
    const check = isSquadConnectionAllowed(nodes, edges.filter((item) => item.id !== edge.id), edge);
    if (!check.ok && check.reason !== '重复连线') errors.push(`军团 ${name} 边 ${edge.id}：${check.reason}`);
  }
  return errors;
}

/// 军团内部树状排列：指挥置顶行，dispatch 成员第二层，孤立成员同行排布，扩展挂到成员右侧。
/// BFS 层级 + 每行均匀横排——简单确定性布局，不引第三方布局库。
export function layoutSquadTree(nodes: SandboxNode[], edges: SandboxEdge[]): SandboxNode[] {
  const GAP_X = 240;
  const GAP_Y = 150;
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const depth = new Map<string, number>();
  const coordinator = squadCoordinatorId(nodes, edges);
  // dispatch 图 BFS 定层；扩展与孤儿节点后置。
  const roots = coordinator ? [coordinator] : nodes.filter((node) => node.data.nodeType === 'subagent').map((node) => node.id);
  const queue: [string, number][] = roots.map((id) => [id, 0]);
  while (queue.length) {
    const [id, level] = queue.shift()!;
    if (depth.has(id)) continue;
    depth.set(id, level);
    for (const edge of edges) {
      if (edge.source !== id || (edge.sourceHandle ?? '') !== 'dispatch') continue;
      queue.push([edge.target, level + 1]);
    }
  }
  // 未在 dispatch 树上的成员放最后一层；扩展挂在最深成员层之下。
  const memberDepths = nodes.filter((node) => node.data.nodeType === 'subagent').map((node) => depth.get(node.id) ?? -1);
  const orphanLevel = memberDepths.length ? Math.max(...memberDepths, 0) + 1 : 0;
  const extLevel = orphanLevel + 1;
  const rows = new Map<number, SandboxNode[]>();
  for (const node of nodes) {
    const level = node.data.nodeType === 'extension'
      ? extLevel
      : (depth.get(node.id) ?? orphanLevel);
    const row = rows.get(level) ?? [];
    row.push(node);
    rows.set(level, row);
  }
  const positioned = new Map<string, { x: number; y: number }>();
  for (const [level, row] of rows) {
    row.forEach((node, index) => {
      positioned.set(node.id, { x: index * GAP_X - ((row.length - 1) * GAP_X) / 2, y: level * GAP_Y });
    });
  }
  return nodes.map((node) => ({ ...node, position: positioned.get(node.id) ?? node.position }));
}

// ---- 校验与序列化 ---------------------------------------------------------------

export interface KnownExtensions {
  skills: SkillView[];
  servers: McpServerView[];
  plugins: PluginView[];
  /** 可装配的子智能体（mode=subagent 且未隐藏）；未加载时缺省——校验不误报未定义。 */
  agents?: AgentDefinition[];
}

export interface DraftValidation {
  errors: string[];
  /** 可保存但值得提示的问题（未安装扩展、空消息、孤立节点）。 */
  warnings: string[];
}

/// Action 提示语留空时 Rust 自动派生文案的 TS 镜像——属性面板预览「留空将自动生成」。
/// 与 default_gate_message 保持一致：关卡 → 「请先{call}。」、限制 → 违反限制、其它 → 通用句。
export function autoActionMessage(upstream: SandboxNode | undefined): string {
  if (!upstream) return '该操作被架构规则拦截。';
  const data = upstream.data;
  switch (data.nodeType) {
    case 'requirement': {
      if (data.updatePlan) return '请先调用 update_plan 提交计划。';
      const tool = data.toolName?.trim();
      const file = data.targetFile?.trim();
      if (tool && file) return `请先调用 ${tool}（${file}）。`;
      if (tool) return `请先调用 ${tool}。`;
      if (file) return `请先读取 ${file}。`;
      return '该操作被架构规则拦截。';
    }
    case 'extension': {
      const ext = data.extension;
      if (!ext) return '该操作被架构规则拦截。';
      if (ext.kind === 'skill') return `请先调用 load_skill ${ext.target}。`;
      if (ext.kind === 'mcp') {
        return ext.tool ? `请先调用 mcp__${ext.target}__${ext.tool}。` : `请先调用 MCP 服务 ${ext.target} 的任意工具。`;
      }
      return `请先完成插件 ${ext.target} 要求的调用。`;
    }
    case 'restriction':
      return `违反架构限制：${data.denyRule ?? ''}，请调整后再试。`;
    default:
      return '该操作被架构规则拦截。';
  }
}

/// 派发义务上游：沿「非扩展来源」入边回溯收集 Trigger（死链检测用；
/// extension→终点是授权边，其上游触发器不属于义务语境）。
const obligationTriggerAncestors = (nodes: SandboxNode[], edges: SandboxEdge[], nodeId: string): string[] => {
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const incoming = new Map<string, string[]>();
  for (const edge of edges) {
    if (byId.get(edge.source)?.data.nodeType === 'extension') continue;
    const list = incoming.get(edge.target) ?? [];
    list.push(edge.source);
    incoming.set(edge.target, list);
  }
  const out: string[] = [];
  const visited = new Set<string>();
  const queue = [nodeId];
  while (queue.length > 0) {
    const id = queue.pop()!;
    for (const parent of incoming.get(id) ?? []) {
      if (visited.has(parent)) continue;
      visited.add(parent);
      if (byId.get(parent)?.data.nodeType === 'trigger') out.push(parent);
      queue.push(parent);
    }
  }
  return out;
};

const nodeName = (node: SandboxNode): string => {
  const data = node.data;
  if (data.nodeType === 'extension') return data.extension?.name || data.extension?.target || node.id;
  return NODE_LABELS[data.nodeType] ?? node.id;
};

export function validateDraft(
  nodes: SandboxNode[],
  edges: SandboxEdge[],
  known?: KnownExtensions,
  squads?: Record<string, { nodes: SandboxNode[]; edges: SandboxEdge[] }>,
): DraftValidation {
  const errors: string[] = [];
  const warnings: string[] = [];
  const ids = new Set(nodes.map((node) => node.id));
  const hasTrigger = nodes.some((node) => node.data.nodeType === 'trigger');
  // mode/scope/继承提示都依赖有效时机（显式 after + pass 边继承）。
  const afterSet = effectiveAfterSet(nodes, edges);
  const hasAfterParent = (nodeId: string) =>
    edges.some((edge) => edge.target === nodeId && afterSet.has(edge.source) && (edge.sourceHandle ?? 'pass') === 'pass');
  if (nodes.length > 128) errors.push('节点数超过 128 上限');
  if (edges.length > 256) errors.push('边数超过 256 上限');
  if (!hasTrigger && nodes.length > 0) warnings.push('图中没有 Trigger：规则不会命中任何调用');

  for (const node of nodes) {
    const data = node.data;
    switch (data.nodeType) {
      case 'trigger':
        if (!data.action) errors.push(`触发器 ${node.id} 缺少动作类型`);
        if (data.scope === 'each') errors.push(`触发器 ${node.id} 的 scope=每次 仅对前置要求/扩展有效`);
        break;
      case 'requirement':
        if (!data.toolName && !data.targetFile && !data.updatePlan) {
          errors.push(`前置要求 ${node.id} 需要 toolName、targetFile 或 updatePlan 之一`);
        }
        if (data.mode && !afterSet.has(node.id)) errors.push(`前置要求 ${node.id} 的挂起策略仅在事后关卡上有效`);
        if (data.scope === 'each' && afterSet.has(node.id)) errors.push(`前置要求 ${node.id} 是事后关卡，scope=每次 无意义`);
        if (data.timing === 'before' && hasAfterParent(node.id)) {
          warnings.push(`前置要求「${nodeName(node)}」位于事后关卡下游但固定为事前——触发调用会先被它拦`);
        }
        break;
      case 'restriction':
        if (!data.denyRule?.trim()) errors.push(`架构限制 ${node.id} 需要非空 denyRule`);
        if (data.scope === 'each') errors.push(`架构限制 ${node.id} 的 scope=每次 仅对前置要求/扩展有效`);
        break;
      case 'action':
        if (!data.result) errors.push(`结果动作 ${node.id} 缺少 result`);
        if (!data.message?.trim()) warnings.push(`结果动作 ${node.id} 没有自定义提示语（留空将自动按上游关卡生成）`);
        else if (/load_skill|mcp__/.test(data.message) && !data.message.includes('{')) {
          warnings.push(`结果动作「${node.id}」提示语疑似写死调用名：改用 {call}/{target} 占位符可自动跟随关卡节点`);
        }
        if (data.scope === 'each') errors.push(`结果动作 ${node.id} 的 scope=每次 仅对前置要求/扩展有效`);
        break;
      case 'subagent': {
        const agent = data.agent?.trim() ?? '';
        if (!agent) errors.push(`子智能体 ${node.id} 缺少 agent 类型名`);
        else if (agent.length > 64 || !/^[A-Za-z0-9_-]+$/.test(agent)) {
          errors.push(`子智能体 ${node.id} 的 agent 名必须为字母/数字/-/_`);
        }
        if (data.scope === 'each') errors.push(`子智能体 ${node.id} 的 scope=每次 仅对前置要求/扩展有效`);
        if (agent && known?.agents && !known.agents.some((item) => item.name === agent && item.mode === 'subagent' && !item.hidden)) {
          warnings.push(`子智能体 ${agent} 未定义或未启用（hidden/非 subagent 模式）：spawn_subagent 时将报错`);
        }
        break;
      }
      case 'squad': {
        const name = data.squad?.trim() ?? '';
        if (!name) errors.push(`军团 ${node.id} 缺少 squad 军团名`);
        else if (name.length > 64 || !/^[A-Za-z0-9_-]+$/.test(name)) {
          errors.push(`军团 ${node.id} 的军团名必须为字母/数字/-/_`);
        }
        if (data.scope === 'each') errors.push(`军团 ${node.id} 的 scope=每次 仅对前置要求/扩展有效`);
        if (name && squads && !squads[name]) {
          errors.push(`军团 ${node.id} 引用的军团 ${name} 不存在于 squads（双击节点进入编辑后重试）`);
        }
        break;
      }
      case 'extension': {
        const ext = data.extension;
        if (!ext?.kind || !ext.target?.trim()) {
          errors.push(`扩展节点 ${node.id} 缺少 kind/target`);
          break;
        }
        if (ext.requirement === 'optional'
          && edges.some((edge) => edge.source === node.id && (edge.sourceHandle ?? 'pass') === 'fail')) {
          errors.push(`可选扩展「${nodeName(node)}」不允许 fail 出边`);
        }
        if (ext.mode && !afterSet.has(node.id)) errors.push(`扩展「${nodeName(node)}」的挂起策略仅在事后关卡上有效`);
        if (ext.scope === 'each' && afterSet.has(node.id)) errors.push(`扩展「${nodeName(node)}」是事后关卡，scope=每次 无意义`);
        if (ext.timing === 'before' && hasAfterParent(node.id)) {
          warnings.push(`扩展「${nodeName(node)}」位于事后关卡下游但固定为事前——触发调用会先被它拦`);
        }
        if (known) {
          if (ext.kind === 'skill' && !known.skills.some((skill) => skill.name === ext.target)) {
            warnings.push(`技能 ${ext.target} 未安装：运行时将被拦截`);
          }
          if (ext.kind === 'mcp') {
            const server = known.servers.find((item) => item.name === ext.target);
            if (!server) warnings.push(`MCP 服务 ${ext.target} 未配置：运行时将被拦截`);
            else if (!server.running) warnings.push(`MCP 服务 ${ext.target} 未运行：运行时将被拦截`);
            else if (ext.tool && !server.tools.some((tool) => tool.name === ext.tool)) {
              warnings.push(`MCP 服务 ${ext.target} 没有工具 ${ext.tool}`);
            }
          }
          if (ext.kind === 'plugin' && !known.plugins.some((plugin) => plugin.name === ext.target && !plugin.error)) {
            warnings.push(`插件 ${ext.target} 未安装或清单损坏：运行时将被拦截`);
          }
        }
        break;
      }
    }
  }
  for (const edge of edges) {
    if (!ids.has(edge.source) || !ids.has(edge.target)) {
      errors.push(`边 ${edge.id} 指向不存在的节点`);
      continue;
    }
    const check = isConnectionAllowed(nodes, edges.filter((item) => item.id !== edge.id), edge);
    if (!check.ok && check.reason !== '重复连线') errors.push(`边 ${edge.id}：${check.reason}`);
    // 派发义务边（非扩展来源 → 装配终点）：回溯不到 Trigger = 死链，运行时永不生效。
    const source = nodes.find((node) => node.id === edge.source);
    const target = nodes.find((node) => node.id === edge.target);
    if (source && target
      && (target.data.nodeType === 'subagent' || target.data.nodeType === 'squad')
      && source.data.nodeType !== 'extension'
      && obligationTriggerAncestors(nodes, edges, edge.target).length === 0) {
      warnings.push(`派发义务链（→ ${nodeName(target)}）回溯不到 Trigger：该义务不会生效`);
    }
  }
  // 环检测（整体）
  if (hasCycle(nodes, edges)) errors.push('规则图存在环（必须是 DAG）');
  // 军团引用唯一性 + 内部结构（镜像 Rust validate_squads）。
  if (squads) {
    const referenced = new Set<string>();
    for (const node of nodes) {
      if (node.data.nodeType !== 'squad') continue;
      const name = node.data.squad?.trim() ?? '';
      if (!name) continue;
      // Set.add 返回集合本身而非“是否新增”——必须先 has 再 add。
      if (referenced.has(name)) errors.push(`Squad 节点重名：${name}`);
      referenced.add(name);
    }
    for (const [name, draft] of Object.entries(squads)) {
      errors.push(...validateSquadDraft(name, draft.nodes, draft.edges));
      if (known?.agents) {
        for (const node of draft.nodes) {
          if (node.data.nodeType !== 'subagent') continue;
          const agent = node.data.agent?.trim() ?? '';
          if (agent && !known.agents.some((item) => item.name === agent && item.mode === 'subagent' && !item.hidden)) {
            warnings.push(`军团 ${name} 成员 ${agent} 未定义或未启用（hidden/非 subagent 模式）：指挥派发时将报错`);
          }
        }
      }
    }
  }
  return { errors, warnings };
}

const serializeNodes = (nodes: SandboxNode[]): GraphNode[] =>
  nodes.map((node) => {
    const { nodeType, ...data } = node.data;
    return {
      id: node.id,
      type: nodeType,
      position: { x: node.position.x, y: node.position.y },
      data: stripEmpty(data),
    } satisfies GraphNode;
  });

const serializeEdges = (edges: SandboxEdge[]): GraphEdge[] =>
  edges.map((edge) => ({
    id: edge.id,
    source: edge.source,
    target: edge.target,
    ...(edge.sourceHandle != null ? { sourceHandle: edge.sourceHandle } : {}),
    targetHandle: edge.targetHandle ?? 'in',
  }));

/// 军团子图序列化：与主图同套节点/边结构，无 version/squads 嵌套。
export function serializeSquad(nodes: SandboxNode[], edges: SandboxEdge[]): SquadGraph {
  return { nodes: serializeNodes(nodes), edges: serializeEdges(edges) };
}

/// RF 运行时结构 → 持久化 JSON：剥离 RF 字段，`data.nodeType` → `type`。
export function serializeGraph(
  nodes: SandboxNode[],
  edges: SandboxEdge[],
  squads?: Record<string, { nodes: SandboxNode[]; edges: SandboxEdge[] }>,
): ArchitectureGraph {
  const squadEntries = Object.entries(squads ?? {})
    .map(([name, draft]) => [name, serializeSquad(draft.nodes, draft.edges)] as const);
  return {
    version: 1,
    nodes: serializeNodes(nodes),
    edges: serializeEdges(edges),
    ...(squadEntries.length ? { squads: Object.fromEntries(squadEntries) } : {}),
  };
}

const stripEmpty = (data: GraphNodeData): GraphNodeData => {
  const out: GraphNodeData = {};
  if (data.targetDir?.trim()) out.targetDir = data.targetDir.trim();
  if (data.action) out.action = data.action;
  if (data.toolName?.trim()) out.toolName = data.toolName.trim();
  if (data.targetFile?.trim()) out.targetFile = data.targetFile.trim();
  if (data.updatePlan) out.updatePlan = true;
  if (data.scope) out.scope = data.scope;
  if (data.timing) out.timing = data.timing;
  // mode 仅 timing=after 有效；剥离非法组合保持 wire 合法（Rust 校验同款约束）。
  if (data.mode && data.timing === 'after') out.mode = data.mode;
  if (data.denyRule?.trim()) out.denyRule = data.denyRule.trim();
  if (data.result) out.result = data.result;
  if (data.message?.trim()) out.message = data.message.trim();
  if (data.agent?.trim()) out.agent = data.agent.trim();
  if (data.squad?.trim()) out.squad = data.squad.trim();
  if (data.requirement) out.requirement = data.requirement;
  if (data.extension?.kind && data.extension.target?.trim()) {
    const ext = data.extension;
    out.extension = {
      kind: ext.kind,
      name: ext.name?.trim() || ext.target.trim(),
      target: ext.target.trim(),
      requirement: ext.requirement ?? 'required',
      ...(ext.tool?.trim() ? { tool: ext.tool.trim() } : {}),
      ...(ext.scope && ext.scope !== 'session' ? { scope: ext.scope } : {}),
      ...(ext.timing ? { timing: ext.timing } : {}),
      ...(ext.mode && ext.timing === 'after' ? { mode: ext.mode } : {}),
      ...(ext.params && Object.keys(ext.params).length ? { params: ext.params } : {}),
    };
  }
  return out;
};

const deserializeNodes = (nodes: GraphNode[]): SandboxNode[] =>
  nodes.map((node) => ({
    id: node.id,
    type: 'maestro',
    position: node.position,
    data: { ...node.data, nodeType: node.type },
  }));

const deserializeEdges = (edges: GraphEdge[]): SandboxEdge[] =>
  edges.map((edge) => ({ ...edge, targetHandle: edge.targetHandle ?? 'in', type: 'maestro' }));

/// 持久化 JSON → 画布结构（含军团子图：squad 节点 data.squad 引用 squads 键）。
export function deserializeGraph(graph: ArchitectureGraph): {
  nodes: SandboxNode[];
  edges: SandboxEdge[];
  squads: Record<string, { nodes: SandboxNode[]; edges: SandboxEdge[] }>;
} {
  const squads = Object.fromEntries(
    Object.entries(graph.squads ?? {}).map(([name, squad]) => [
      name,
      { nodes: deserializeNodes(squad.nodes), edges: deserializeEdges(squad.edges) },
    ]),
  );
  return {
    nodes: deserializeNodes(graph.nodes),
    edges: deserializeEdges(graph.edges),
    squads,
  };
}

// ---- 预置模板 -----------------------------------------------------------------

export interface TemplateSeed {
  skills: SkillView[];
  servers: McpServerView[];
  plugins: PluginView[];
}

export interface SandboxTemplate {
  id: string;
  label: string;
  description: string;
  /// `squads` 为模板携带的军团子图：键是占位军团名，插入时防重名改名并
  /// 同步改写 squad 节点的 data.squad；子图节点/边有独立 id 空间。
  build: (seed: TemplateSeed) => {
    nodes: SandboxNode[];
    edges: SandboxEdge[];
    squads?: Record<string, { nodes: SandboxNode[]; edges: SandboxEdge[] }>;
  };
}

const edge = (source: SandboxNode, target: SandboxNode, handle: 'out' | 'pass' | 'fail'): SandboxEdge =>
  ({ id: nextId(), source: source.id, target: target.id, sourceHandle: handle, targetHandle: 'in', type: 'maestro' });

const at = (x: number, y: number) => ({ x, y });

// 先精确/包含匹配 fallback 语义名（'code-review' 命中 code-review-and-quality），
// 命中不了才退回列表首个——避免模板永远选中字母序第一的无关扩展。
const pickNamed = <T extends { name: string }>(list: T[], fallback: string): string | undefined =>
  list.find((item) => item.name === fallback)?.name
    ?? list.find((item) => item.name.includes(fallback))?.name
    ?? list[0]?.name;
const pickSkill = (seed: TemplateSeed, fallback: string) =>
  pickNamed(seed.skills, fallback) ?? fallback;
const pickServer = (seed: TemplateSeed, fallback: string) =>
  pickNamed(seed.servers, fallback) ?? fallback;

export const SANDBOX_TEMPLATES: SandboxTemplate[] = [
  {
    id: 'read-before-modify',
    label: '修改代码前必须先读文档',
    description: 'Trigger(modify) → Requirement(read_file 文档) → 拦截',
    build: () => {
      const trigger = createNode('trigger', at(60, 160), { action: 'modify', targetDir: 'src' });
      const requirement = createNode('requirement', at(340, 160), { toolName: 'read_file', targetFile: 'docs/architecture.md', scope: 'session' });
      const action = createNode('action', at(640, 160), { result: 'block', message: '修改代码前必须先阅读 docs/architecture.md。' });
      return { nodes: [trigger, requirement, action], edges: [edge(trigger, requirement, 'out'), edge(requirement, action, 'fail')] };
    },
  },
  {
    id: 'plan-before-write',
    label: '写代码前必须提交计划',
    description: 'Trigger(modify) → Requirement(update_plan) → 拦截',
    build: () => {
      const trigger = createNode('trigger', at(60, 160), { action: 'modify' });
      const requirement = createNode('requirement', at(340, 160), { updatePlan: true, scope: 'session' });
      const action = createNode('action', at(640, 160), { result: 'block', message: '写代码前请先调用 update_plan 提交任务计划。' });
      return { nodes: [trigger, requirement, action], edges: [edge(trigger, requirement, 'out'), edge(requirement, action, 'fail')] };
    },
  },
  {
    id: 'required-skill',
    label: '必须先加载指定 Skill',
    description: 'Trigger(modify src/ui) → 必选 Skill → 拦截',
    build: (seed) => {
      const skill = pickSkill(seed, 'ui-guidelines');
      const trigger = createNode('trigger', at(60, 160), { action: 'modify', targetDir: 'src/ui' });
      const ext = createExtensionNode(at(360, 160), { kind: 'skill', name: skill, target: skill, requirement: 'required', scope: 'session' });
      const action = createNode('action', at(660, 160), { result: 'block', message: '请先{call}再修改文件。' });
      return { nodes: [trigger, ext, action], edges: [edge(trigger, ext, 'out'), edge(ext, action, 'fail')] };
    },
  },
  {
    id: 'required-mcp',
    label: '必须先调用 MCP 工具',
    description: 'Trigger(modify) → 必选 MCP Server/Tool → 拦截',
    build: (seed) => {
      const server = pickServer(seed, 'postgres');
      const tool = seed.servers[0]?.tools[0]?.name;
      const trigger = createNode('trigger', at(60, 160), { action: 'modify', targetDir: 'src' });
      const ext = createExtensionNode(at(360, 160), { kind: 'mcp', name: server, target: server, tool, requirement: 'required', scope: 'run' });
      const action = createNode('action', at(660, 160), { result: 'block', message: '必须先{call}后才能写入。' });
      return { nodes: [trigger, ext, action], edges: [edge(trigger, ext, 'out'), edge(ext, action, 'fail')] };
    },
  },
  {
    id: 'plan-review',
    label: '生成计划前必须计划审查',
    description: 'Trigger(plan) → 必选 Skill（计划审查）→ 拦截',
    build: (seed) => {
      const skill = pickSkill(seed, 'plan-review');
      const trigger = createNode('trigger', at(60, 160), { action: 'plan' });
      const ext = createExtensionNode(at(360, 160), { kind: 'skill', name: skill, target: skill, requirement: 'required', scope: 'run' });
      const action = createNode('action', at(660, 160), { result: 'block', message: '提交计划前必须先完成计划审查流程。' });
      return { nodes: [trigger, ext, action], edges: [edge(trigger, ext, 'out'), edge(ext, action, 'fail')] };
    },
  },
  {
    id: 'decompose-before-code',
    label: '生成代码前必须拆分调研',
    description: 'Trigger(modify) → Requirement(spawn_subagent) → 拦截',
    build: () => {
      const trigger = createNode('trigger', at(60, 160), { action: 'modify', targetDir: 'src' });
      const requirement = createNode('requirement', at(340, 160), { toolName: 'spawn_subagent', scope: 'run' });
      const action = createNode('action', at(640, 160), { result: 'block', message: '修改代码前请先拆分任务：用 spawn_subagent 完成调研。' });
      return { nodes: [trigger, requirement, action], edges: [edge(trigger, requirement, 'out'), edge(requirement, action, 'fail')] };
    },
  },
  {
    id: 'dangerous-command',
    label: '危险命令直接拦截',
    description: 'Trigger(command 含 rm -rf) → 拦截',
    build: () => {
      const trigger = createNode('trigger', at(60, 160), { action: 'command', targetDir: 'rm -rf' });
      const action = createNode('action', at(360, 160), { result: 'block', message: '危险命令已被架构规则拦截。' });
      return { nodes: [trigger, action], edges: [edge(trigger, action, 'out')] };
    },
  },
  {
    id: 'after-skill-review',
    label: '改码后必须调用审查 Skill',
    description: 'Trigger(modify) → 事后 Skill 验收 → 未验收拦截后续操作',
    build: (seed) => {
      const skill = pickSkill(seed, 'code-review');
      const trigger = createNode('trigger', at(60, 160), { action: 'modify', targetDir: 'src' });
      const ext = createExtensionNode(at(360, 160), {
        kind: 'skill', name: skill, target: skill, requirement: 'required', scope: 'session', timing: 'after',
      });
      const action = createNode('action', at(660, 160), { result: 'block', message: '修改尚未验收：请先{call}完成审查。' });
      return { nodes: [trigger, ext, action], edges: [edge(trigger, ext, 'out'), edge(ext, action, 'fail')] };
    },
  },
  {
    id: 'after-run-tests',
    label: '改码后必须跑测试',
    description: 'Trigger(modify) → 事后 run_command 验收 → 未验收拦截后续操作',
    build: () => {
      const trigger = createNode('trigger', at(60, 160), { action: 'modify', targetDir: 'src' });
      const requirement = createNode('requirement', at(360, 160), { toolName: 'run_command', timing: 'after' });
      const action = createNode('action', at(660, 160), { result: 'block', message: '代码已修改但尚未运行测试：请先执行测试命令完成验收。' });
      return { nodes: [trigger, requirement, action], edges: [edge(trigger, requirement, 'out'), edge(requirement, action, 'fail')] };
    },
  },
  {
    id: 'remind-on-modify',
    label: '修改时持续提醒（防漂移）',
    description: 'Trigger(modify) → 提醒动作：每次写入放行但附提醒文本到工具结果',
    build: () => {
      const trigger = createNode('trigger', at(60, 160), { action: 'modify' });
      const action = createNode('action', at(380, 160), { result: 'remind', message: '改动须与已批准计划一致；发现漂移先停下回报。' });
      return { nodes: [trigger, action], edges: [edge(trigger, action, 'out')] };
    },
  },
  {
    id: 'plan-each-modify',
    label: '每次写码前重新提交计划',
    description: 'Trigger(modify) → Requirement(update_plan, scope=每次) → 拦截',
    build: () => {
      const trigger = createNode('trigger', at(60, 160), { action: 'modify' });
      const requirement = createNode('requirement', at(360, 160), { updatePlan: true, scope: 'each' });
      const action = createNode('action', at(660, 160), { result: 'block', message: '每次修改前请先 update_plan 提交当前计划。' });
      return { nodes: [trigger, requirement, action], edges: [edge(trigger, requirement, 'out'), edge(requirement, action, 'fail')] };
    },
  },
  {
    id: 'squad-team',
    label: '示例：调研军团',
    description: '军团节点（双击进子图）：general 指挥 派发→ explore 必选成员；可选 Skill 连军团 = 全员共享授权',
    build: (seed) => {
      const skill = pickSkill(seed, 'code-review');
      const squad = createNode('squad', at(200, 180), { squad: 'research-team' });
      const ext = createExtensionNode(at(200, 330), { kind: 'skill', name: skill, target: skill, requirement: 'optional', scope: 'run' });
      return {
        nodes: [squad, ext],
        edges: [edge(ext, squad, 'pass')],
        squads: { 'research-team': squadSkeleton() },
      };
    },
  },
];

/// 画布示例：Trigger(modify src/ui) → 必选 Skill + 必选 MCP（AND 分支）；
/// Skill pass → Restriction(src/db) → Action；两个 fail 各自接 Action。
/// 节点使用确定性 id，preview fixture 的 trace 才能按 nodeId 高亮。
export function sampleGraph(seed: TemplateSeed): { nodes: SandboxNode[]; edges: SandboxEdge[] } {
  const skill = pickSkill(seed, 'ui-guidelines');
  const server = pickServer(seed, 'postgres');
  const tool = seed.servers[0]?.tools[0]?.name;

  const trigger = { ...createNode('trigger', at(40, 200), { action: 'modify', targetDir: 'src/ui' }), id: 'sample-trigger' };
  const skillNode = { ...createExtensionNode(at(320, 80), { kind: 'skill', name: skill, target: skill, requirement: 'required', scope: 'session' }), id: 'sample-skill' };
  const mcpNode = { ...createExtensionNode(at(320, 330), { kind: 'mcp', name: server, target: server, tool, requirement: 'required', scope: 'session' }), id: 'sample-mcp' };
  const restriction = { ...createNode('restriction', at(620, 60), { denyRule: 'src/db' }), id: 'sample-restriction' };
  const noDb = { ...createNode('action', at(920, 60), { result: 'block', message: '不允许在 UI 层引入 DB 层，请重写。' }), id: 'sample-action-db' };
  const needSkill = { ...createNode('action', at(620, 190), { result: 'block', message: '请先{call}。' }), id: 'sample-action-skill' };
  const needMcp = { ...createNode('action', at(620, 330), { result: 'block', message: '必须先{call}后才能写入。' }), id: 'sample-action-mcp' };

  return {
    nodes: [trigger, skillNode, mcpNode, restriction, noDb, needSkill, needMcp],
    edges: [
      edge(trigger, skillNode, 'out'),
      edge(trigger, mcpNode, 'out'),
      edge(skillNode, restriction, 'pass'),
      edge(skillNode, needSkill, 'fail'),
      edge(restriction, noDb, 'fail'),
      edge(mcpNode, needMcp, 'fail'),
    ],
  };
}

/// 重映射节点 id（示例图二次插入画布时避免 id 冲突）。
export function remapIds(nodes: SandboxNode[], edges: SandboxEdge[]): { nodes: SandboxNode[]; edges: SandboxEdge[] } {
  const map = new Map(nodes.map((node) => [node.id, nextId()]));
  return {
    nodes: nodes.map((node) => ({ ...node, id: map.get(node.id)! })),
    edges: edges.map((edge) => ({ ...edge, id: nextId(), source: map.get(edge.source)!, target: map.get(edge.target)! })),
  };
}

// ---- Trace → 画布高亮 ----------------------------------------------------------

const STATUS_PRIORITY: Record<TraceStatus, number> = {
  matched: 1,
  passed: 2,
  optional: 3,
  pending: 4,
  reminded: 5,
  failed: 6,
  blocked: 7,
};

/// 最近一次 trace 的节点状态映射；多步合并时取更「决定性」的状态。
export function traceNodeStatus(steps: ArchitectureTraceStep[]): Record<string, TraceStatus> {
  const out: Record<string, TraceStatus> = {};
  for (const step of steps) {
    const existing = out[step.nodeId];
    if (!existing || STATUS_PRIORITY[step.status] >= STATUS_PRIORITY[existing]) {
      out[step.nodeId] = step.status;
    }
  }
  return out;
}
