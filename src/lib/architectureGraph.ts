/// 架构沙盘纯函数层：节点/边模型、连接规则、DAG 校验、序列化与预置模板。
/// 与 Rust `domain::architecture` 的校验语义保持一致（结构规则双端各查一次：
/// 画布给即时反馈，Rust 保存/执行时做权威判定）。
import type {
  ArchitectureGraph, ExtensionData, ExtensionKind, GraphEdge, GraphNode,
  GraphNodeData, SatisfyScope, TriggerAction,
} from '../api/architecture';
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
};

export const ACTION_LABELS: Record<TriggerAction, string> = {
  modify: '修改文件',
  read: '读取文件',
  command: '执行命令',
  plan: '提交计划',
  any: '任意工具',
  prompt: '用户输入匹配',
};

export function createNode(kind: NodeKind, position: { x: number; y: number }, seed: Partial<GraphNodeData> = {}): SandboxNode {
  const data: SandboxNode['data'] = { nodeType: kind, ...seed };
  if (kind === 'trigger' && !data.action) data.action = 'modify';
  if (kind === 'action' && !data.result) data.result = 'block';
  if (kind === 'extension' && !data.extension) {
    data.extension = { kind: 'skill', name: '', target: '', requirement: 'required', scope: 'session', params: {} };
  }
  return { id: nextId(), type: 'maestro', position, data };
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
    case 'action': return [];
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
  if (target.data.nodeType === 'trigger') return { ok: false, reason: 'Trigger 是起点，不能作为目标' };
  if ((candidate.targetHandle ?? 'in') !== 'in') return { ok: false, reason: '目标仅接受入口 Handle' };
  const handle = candidate.sourceHandle ?? 'out';
  const allowed = handlesFor(source);
  if (!allowed.includes(handle)) {
    return {
      ok: false,
      reason: source.data.nodeType === 'extension' && handle === 'fail'
        ? '可选扩展不允许 fail 出边'
        : '该 Handle 不支持从此节点出发',
    };
  }
  if (edges.some((edge) => edge.source === source.id && edge.target === target.id && (edge.sourceHandle ?? 'out') === handle)) {
    return { ok: false, reason: '重复连线' };
  }
  if (wouldCreateCycle(nodes, edges, source.id, target.id)) {
    return { ok: false, reason: '会形成环（规则图必须是 DAG）' };
  }
  return { ok: true };
}

/// 边的语义色：fail → 红；目标为 warn Action → 黄；pass/out → 默认。
export function edgeTone(edge: SandboxEdge, nodes: SandboxNode[]): 'fail' | 'warn' | 'default' {
  if ((edge.sourceHandle ?? 'out') === 'fail') return 'fail';
  const target = nodes.find((node) => node.id === edge.target);
  if (target?.data.nodeType === 'action') {
    return target.data.result === 'warn' ? 'warn' : 'fail';
  }
  return 'default';
}

// ---- 校验与序列化 ---------------------------------------------------------------

export interface KnownExtensions {
  skills: SkillView[];
  servers: McpServerView[];
  plugins: PluginView[];
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

const nodeName = (node: SandboxNode): string => {
  const data = node.data;
  if (data.nodeType === 'extension') return data.extension?.name || data.extension?.target || node.id;
  return NODE_LABELS[data.nodeType] ?? node.id;
};

export function validateDraft(
  nodes: SandboxNode[],
  edges: SandboxEdge[],
  known?: KnownExtensions,
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
  }
  // 环检测（整体）
  if (hasCycle(nodes, edges)) errors.push('规则图存在环（必须是 DAG）');
  return { errors, warnings };
}

/// RF 运行时结构 → 持久化 JSON：剥离 RF 字段，`data.nodeType` → `type`。
export function serializeGraph(nodes: SandboxNode[], edges: SandboxEdge[]): ArchitectureGraph {
  return {
    version: 1,
    nodes: nodes.map((node) => {
      const { nodeType, ...data } = node.data;
      return {
        id: node.id,
        type: nodeType,
        position: { x: node.position.x, y: node.position.y },
        data: stripEmpty(data),
      } satisfies GraphNode;
    }),
    edges: edges.map((edge) => ({
      id: edge.id,
      source: edge.source,
      target: edge.target,
      ...(edge.sourceHandle != null ? { sourceHandle: edge.sourceHandle } : {}),
      targetHandle: edge.targetHandle ?? 'in',
    })),
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

/// 持久化 JSON → 画布结构。
export function deserializeGraph(graph: ArchitectureGraph): { nodes: SandboxNode[]; edges: SandboxEdge[] } {
  return {
    nodes: graph.nodes.map((node) => ({
      id: node.id,
      type: 'maestro',
      position: node.position,
      data: { ...node.data, nodeType: node.type },
    })),
    edges: graph.edges.map((edge) => ({ ...edge, targetHandle: edge.targetHandle ?? 'in', type: 'maestro' })),
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
  build: (seed: TemplateSeed) => { nodes: SandboxNode[]; edges: SandboxEdge[] };
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
