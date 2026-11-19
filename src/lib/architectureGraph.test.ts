import { describe, expect, it } from 'vitest';
import {
  createExtensionNode, createNode, deserializeGraph, isConnectionAllowed,
  autoActionMessage, nodeSummary, remapIds, sampleGraph, serializeGraph, traceNodeStatus, validateDraft,
  SANDBOX_TEMPLATES,
  type SandboxEdge, type SandboxNode,
} from './architectureGraph';
import type { PluginView } from '../api/extensions';

const at = { x: 0, y: 0 };
const SEED = {
  skills: [{ name: 'ui-guidelines', description: '', source: 'global' as const, path: '' }],
  servers: [{
    name: 'postgres', command: 'npx', args: [], env: {}, disabled: false, autoStart: false,
    trust: false, running: true, toolCount: 1, error: null,
    tools: [{ name: 'query', description: '', inputSchema: {}, readOnly: true }],
  }],
  plugins: [] as PluginView[],
};

describe('isConnectionAllowed', () => {
  const make = (): { nodes: SandboxNode[]; edges: SandboxEdge[] } => {
    const trigger = createNode('trigger', at, { action: 'modify' });
    const requirement = createNode('requirement', at, { toolName: 'read_file' });
    const action = createNode('action', at, { result: 'block' });
    return {
      nodes: [trigger, requirement, action],
      edges: [{ id: 'e1', source: trigger.id, target: requirement.id, sourceHandle: 'out', targetHandle: 'in' }],
    };
  };

  it('接受 trigger→requirement→action 的正常链路', () => {
    const { nodes, edges } = make();
    const requirement = nodes[1];
    const action = nodes[2];
    expect(isConnectionAllowed(nodes, edges, { source: requirement.id, target: action.id, sourceHandle: 'fail' }).ok).toBe(true);
    expect(isConnectionAllowed(nodes, edges, { source: requirement.id, target: action.id, sourceHandle: 'pass' }).ok).toBe(true);
  });

  it('拒绝环、自连、重复边与非法方向', () => {
    const { nodes, edges } = make();
    const [trigger, requirement, action] = nodes;
    expect(isConnectionAllowed(nodes, edges, { source: requirement.id, target: trigger.id, sourceHandle: 'fail' }).ok).toBe(false);
    expect(isConnectionAllowed(nodes, edges, { source: trigger.id, target: trigger.id, sourceHandle: 'out' }).ok).toBe(false);
    expect(isConnectionAllowed(nodes, edges, { source: trigger.id, target: requirement.id, sourceHandle: 'out' }).ok).toBe(false);
    expect(isConnectionAllowed(nodes, edges, { source: action.id, target: requirement.id, sourceHandle: 'out' }).ok).toBe(false);
    expect(isConnectionAllowed(nodes, edges, { source: requirement.id, target: action.id, sourceHandle: 'bogus' }).ok).toBe(false);
  });

  it('可选扩展不允许 fail 出边，必选扩展允许', () => {
    const required = createExtensionNode(at, { kind: 'skill', name: 's', target: 's', requirement: 'required' });
    const optional = createExtensionNode(at, { kind: 'skill', name: 'o', target: 'o', requirement: 'optional' });
    const action = createNode('action', at, { result: 'block' });
    const nodes = [required, optional, action];
    expect(isConnectionAllowed(nodes, [], { source: required.id, target: action.id, sourceHandle: 'fail' }).ok).toBe(true);
    expect(isConnectionAllowed(nodes, [], { source: optional.id, target: action.id, sourceHandle: 'fail' }).ok).toBe(false);
  });
});

describe('validateDraft', () => {
  it('空字段报错误；未安装扩展报警告', () => {
    const requirement = createNode('requirement', at, {});
    const restriction = createNode('restriction', at, { denyRule: '  ' });
    const ext = createExtensionNode(at, { kind: 'skill', name: 'ghost', target: 'ghost', requirement: 'required' });
    const { errors, warnings } = validateDraft([requirement, restriction, ext], [], SEED);
    expect(errors.some((e) => e.includes('前置要求'))).toBe(true);
    expect(errors.some((e) => e.includes('denyRule'))).toBe(true);
    expect(warnings.some((w) => w.includes('ghost'))).toBe(true);
    expect(warnings.some((w) => w.includes('Trigger'))).toBe(true);
  });

  it('检测环', () => {
    const a = createNode('requirement', at, { toolName: 'read_file' });
    const b = createNode('requirement', at, { toolName: 'write_file' });
    const edges: SandboxEdge[] = [
      { id: 'x', source: a.id, target: b.id, sourceHandle: 'fail', targetHandle: 'in' },
      { id: 'y', source: b.id, target: a.id, sourceHandle: 'fail', targetHandle: 'in' },
    ];
    const { errors } = validateDraft([a, b], edges, SEED);
    expect(errors.some((e) => e.includes('环'))).toBe(true);
  });
});

describe('serialize/deserialize 往返', () => {
  it('节点类型与数据字段保持 wire 契约', () => {
    const trigger = createNode('trigger', at, { action: 'modify', targetDir: ' src/ui ' });
    const skill = createExtensionNode(at, { kind: 'skill', name: 'ui-guidelines', target: 'ui-guidelines', requirement: 'required', scope: 'session', params: {} });
    const action = createNode('action', at, { result: 'block', message: '请先加载技能。' });
    const edges: SandboxEdge[] = [
      { id: 'e1', source: trigger.id, target: skill.id, sourceHandle: 'out', targetHandle: 'in' },
      { id: 'e2', source: skill.id, target: action.id, sourceHandle: 'fail', targetHandle: 'in' },
    ];
    const graph = serializeGraph([trigger, skill, action], edges);
    expect(graph.nodes[0].type).toBe('trigger');
    expect(graph.nodes[0].data.targetDir).toBe('src/ui');
    expect(graph.nodes[1].type).toBe('extension');
    expect(graph.nodes[1].data.extension?.target).toBe('ui-guidelines');
    expect(graph.edges[0].sourceHandle).toBe('out');
    const back = deserializeGraph(graph);
    expect(back.nodes[1].data.nodeType).toBe('extension');
    expect(back.edges[0].sourceHandle).toBe('out');
  });

  it('空字段被剥离，不会写入 undefined 键', () => {
    const node = createNode('trigger', at, { action: 'any', targetDir: '  ' });
    const graph = serializeGraph([node], []);
    expect('targetDir' in graph.nodes[0].data).toBe(false);
  });

  it('timing/mode 透传；mode 在 timing≠after 时被剥离保持 wire 合法', () => {
    const req = createNode('requirement', at, { toolName: 'run_command', timing: 'after', mode: 'strict' });
    const bad = createNode('requirement', at, { toolName: 'run_command', mode: 'strict' });
    const ext = createExtensionNode(at, { kind: 'skill', name: 'review', target: 'review', requirement: 'required', timing: 'after', mode: 'batch' });
    const graph = serializeGraph([req, bad, ext], []);
    expect(graph.nodes[0].data.timing).toBe('after');
    expect(graph.nodes[0].data.mode).toBe('strict');
    expect('mode' in graph.nodes[1].data).toBe(false);
    expect(graph.nodes[2].data.extension?.timing).toBe('after');
    expect(graph.nodes[2].data.extension?.mode).toBe('batch');
    const back = deserializeGraph(graph);
    expect(back.nodes[0].data.timing).toBe('after');
    expect(back.nodes[0].data.mode).toBe('strict');
  });

  it('nodeSummary 区分事前/事后', () => {
    const before = createNode('requirement', at, { toolName: 'run_command' });
    const after = createNode('requirement', at, { toolName: 'run_command', timing: 'after' });
    expect(nodeSummary(before)).toBe('先调用 run_command');
    expect(nodeSummary(after)).toBe('事后调用 run_command');
    const extAfter = createExtensionNode(at, { kind: 'skill', name: 'review', target: 'review', requirement: 'required', timing: 'after' });
    expect(nodeSummary(extAfter)).toContain('事后验收');
  });
});

describe('sampleGraph / remapIds / traceNodeStatus', () => {
  it('示例图可序列化、无错误，节点 id 确定', () => {
    const { nodes, edges } = sampleGraph(SEED);
    const { errors } = validateDraft(nodes, edges, SEED);
    expect(errors).toEqual([]);
    expect(nodes.map((n) => n.id)).toContain('sample-trigger');
    const remapped = remapIds(nodes, edges);
    expect(remapped.nodes.every((n) => !n.id.startsWith('sample-'))).toBe(true);
    expect(remapped.edges.every((e) => remapped.nodes.some((n) => n.id === e.source))).toBe(true);
  });

  it('trace 合并取更决定性的状态', () => {
    const map = traceNodeStatus([
      { nodeId: 'a', status: 'matched' },
      { nodeId: 'a', status: 'passed' },
      { nodeId: 'b', status: 'failed' },
      { nodeId: 'b', status: 'blocked' },
    ]);
    expect(map.a).toBe('passed');
    expect(map.b).toBe('blocked');
  });
});

describe('模板与占位符', () => {
  it('模板按名称模糊匹配扩展：code-review 命中 code-review-and-quality', () => {
    const seed = {
      skills: [
        { name: 'algorithmic-art', description: '', source: 'global' as const, path: '' },
        { name: 'code-review-and-quality', description: '', source: 'global' as const, path: '' },
      ],
      servers: SEED.servers,
      plugins: [] as PluginView[],
    };
    const built = SANDBOX_TEMPLATES.find((t) => t.id === 'after-skill-review')!.build(seed);
    const ext = built.nodes.find((n) => n.data.nodeType === 'extension');
    expect(ext?.data.extension?.target).toBe('code-review-and-quality');
    const action = built.nodes.find((n) => n.data.nodeType === 'action');
    expect(action?.data.message).toContain('{call}');
  });

  it('含扩展名的模板文案一律用占位符，不写死调用名', () => {
    const built = SANDBOX_TEMPLATES.find((t) => t.id === 'required-skill')!.build(SEED);
    const action = built.nodes.find((n) => n.data.nodeType === 'action');
    expect(action?.data.message).toContain('{call}');
    expect(action?.data.message).not.toContain('ui-guidelines');
    const mcp = SANDBOX_TEMPLATES.find((t) => t.id === 'required-mcp')!.build(SEED);
    const mcpAction = mcp.nodes.find((n) => n.data.nodeType === 'action');
    expect(mcpAction?.data.message).toContain('{call}');
  });

  it('validateDraft 对写死调用名的 Action 文案报警告', () => {
    const trigger = createNode('trigger', at, { action: 'modify' });
    const action = createNode('action', at, {
      result: 'block',
      message: '请先调用 load_skill("algorithmic-art") 完成审查。',
    });
    const { warnings } = validateDraft([trigger, action], [
      { id: 'e1', source: trigger.id, target: action.id, sourceHandle: 'out', targetHandle: 'in' },
    ], SEED);
    expect(warnings.some((w) => w.includes('写死调用名'))).toBe(true);
    // 占位符文案不警告。
    const ok = createNode('action', at, { result: 'block', message: '请先{call}完成审查。' });
    const { warnings: okWarnings } = validateDraft([trigger, ok], [
      { id: 'e2', source: trigger.id, target: ok.id, sourceHandle: 'out', targetHandle: 'in' },
    ], SEED);
    expect(okWarnings.some((w) => w.includes('写死调用名'))).toBe(false);
  });
});

describe('autoActionMessage（空文案自动派生预览）', () => {
  it('按上游关卡生成与 Rust default_gate_message 一致的文案', () => {
    const skill = createExtensionNode(at, { kind: 'skill', name: 'ui-review', target: 'ui-review', requirement: 'required' });
    expect(autoActionMessage(skill)).toBe('请先调用 load_skill ui-review。');
    const mcpTool = createExtensionNode(at, { kind: 'mcp', name: 'pg/query', target: 'pg', tool: 'query', requirement: 'required' });
    expect(autoActionMessage(mcpTool)).toBe('请先调用 mcp__pg__query。');
    const mcpServer = createExtensionNode(at, { kind: 'mcp', name: 'pg', target: 'pg', requirement: 'required' });
    expect(autoActionMessage(mcpServer)).toBe('请先调用 MCP 服务 pg 的任意工具。');
    const plugin = createExtensionNode(at, { kind: 'plugin', name: 'p', target: 'p', requirement: 'required' });
    expect(autoActionMessage(plugin)).toBe('请先完成插件 p 要求的调用。');
    expect(autoActionMessage(createNode('requirement', at, { updatePlan: true }))).toBe('请先调用 update_plan 提交计划。');
    expect(autoActionMessage(createNode('requirement', at, { toolName: 'run_command', targetFile: 'cargo test' }))).toBe('请先调用 run_command（cargo test）。');
    expect(autoActionMessage(createNode('requirement', at, { targetFile: 'docs/a.md' }))).toBe('请先读取 docs/a.md。');
    expect(autoActionMessage(createNode('restriction', at, { denyRule: 'src/db' }))).toBe('违反架构限制：src/db，请调整后再试。');
    expect(autoActionMessage(createNode('trigger', at, { action: 'modify' }))).toBe('该操作被架构规则拦截。');
    expect(autoActionMessage(undefined)).toBe('该操作被架构规则拦截。');
  });
});
