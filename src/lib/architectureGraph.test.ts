import { describe, expect, it } from 'vitest';
import {
  createExtensionNode, createNode, deserializeGraph, edgeTone, isConnectionAllowed,
  autoActionMessage, isSquadConnectionAllowed, layoutSquadTree, nodeSummary, remapIds, sampleGraph,
  serializeGraph, squadSkeleton, traceNodeStatus, validateDraft, validateSquadDraft,
  SANDBOX_TEMPLATES,
  type SandboxEdge, type SandboxNode,
} from './architectureGraph';
import type { PluginView } from '../api/extensions';

const at = { x: 0, y: 0 };
const SEED = {
  skills: [{ name: 'ui-guidelines', description: '', source: 'global' as const, path: '' }],
  servers: [{
    name: 'postgres', command: 'npx', args: [], transport: 'stdio' as const, url: null, headerKeys: [], timeoutMs: null, env: {}, disabled: false, autoStart: false,
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

describe('subagent 装配节点', () => {
  const agent = () => createNode('subagent', at, { agent: 'explore' });
  const ext = (requirement: 'required' | 'optional' = 'required') =>
    createExtensionNode(at, { kind: 'skill', name: 'ui-guidelines', target: 'ui-guidelines', requirement });

  it('Extension --pass--> Subagent 合法；多个扩展可 merge 进同一节点', () => {
    const s = agent();
    const a = ext(), b = ext();
    const nodes = [s, a, b];
    expect(isConnectionAllowed(nodes, [], { source: a.id, target: s.id, sourceHandle: 'pass' }).ok).toBe(true);
    const edges: SandboxEdge[] = [{ id: 'e1', source: a.id, target: s.id, sourceHandle: 'pass', targetHandle: 'in' }];
    expect(isConnectionAllowed(nodes, edges, { source: b.id, target: s.id, sourceHandle: 'pass' }).ok).toBe(true);
    // 缺省出口（无 sourceHandle）按 pass 计。
    expect(isConnectionAllowed(nodes, edges, { source: b.id, target: s.id }).ok).toBe(true);
  });

  it('义务入边：trigger out / 条件 pass 可连；fail 与 action 入边仍拒', () => {
    const s = agent();
    const trigger = createNode('trigger', at, { action: 'any' });
    const req = createNode('requirement', at, { toolName: 'read_file' });
    const deny = createNode('restriction', at, { denyRule: 'secret' });
    const skill = ext();
    const action = createNode('action', at, { result: 'block' });
    const nodes = [s, trigger, req, deny, skill, action];
    // 义务连线（连线即声明「必须先派发」）
    expect(isConnectionAllowed(nodes, [], { source: trigger.id, target: s.id, sourceHandle: 'out' }).ok).toBe(true);
    expect(isConnectionAllowed(nodes, [], { source: req.id, target: s.id, sourceHandle: 'pass' }).ok).toBe(true);
    expect(isConnectionAllowed(nodes, [], { source: deny.id, target: s.id, sourceHandle: 'pass' }).ok).toBe(true);
    // fail 出口与 Action 源仍拒
    expect(isConnectionAllowed(nodes, [], { source: skill.id, target: s.id, sourceHandle: 'fail' }).ok).toBe(false);
    expect(isConnectionAllowed(nodes, [], { source: req.id, target: s.id, sourceHandle: 'fail' }).ok).toBe(false);
    expect(isConnectionAllowed(nodes, [], { source: action.id, target: s.id, sourceHandle: 'pass' }).ok).toBe(false);
    // subagent 自身仍无出边
    expect(isConnectionAllowed([s, action], [], { source: s.id, target: action.id, sourceHandle: 'pass' }).ok).toBe(false);
  });

  it('义务边语义色为 dispatch 紫；授权边保持默认', () => {
    const s = agent();
    const trigger = createNode('trigger', at, { action: 'any' });
    const req = createNode('requirement', at, { toolName: 'read_file' });
    const skill = ext();
    const nodes = [s, trigger, req, skill];
    const duty: SandboxEdge = { id: 'e1', source: trigger.id, target: s.id, sourceHandle: 'out', targetHandle: 'in' };
    const gated: SandboxEdge = { id: 'e2', source: req.id, target: s.id, sourceHandle: 'pass', targetHandle: 'in' };
    const grant: SandboxEdge = { id: 'e3', source: skill.id, target: s.id, sourceHandle: 'pass', targetHandle: 'in' };
    expect(edgeTone(duty, nodes)).toBe('dispatch');
    expect(edgeTone(gated, nodes)).toBe('dispatch');
    expect(edgeTone(grant, nodes)).toBe('default');
  });

  it('validateDraft：义务链回溯不到 Trigger 报警告', () => {
    const s = agent();
    const req = createNode('requirement', at, { toolName: 'read_file' });
    const trigger = createNode('trigger', at, { action: 'any' });
    // req→s 死链（req 无 trigger 上游）
    const dead: SandboxEdge = { id: 'e1', source: req.id, target: s.id, sourceHandle: 'pass', targetHandle: 'in' };
    const { warnings } = validateDraft([s, req], [dead], { ...SEED, agents: [] });
    expect(warnings.some((w) => w.includes('回溯不到 Trigger'))).toBe(true);
    // trigger→req→s 活链无警告
    const live: SandboxEdge[] = [
      { id: 'e2', source: trigger.id, target: req.id, sourceHandle: 'out', targetHandle: 'in' },
      dead,
    ];
    const ok = validateDraft([s, req, trigger], live, { ...SEED, agents: [] });
    expect(ok.warnings.some((w) => w.includes('回溯不到 Trigger'))).toBe(false);
  });

  it('validateDraft：缺 agent/非法字符报错，未定义 agent 报警告', () => {
    const missing = createNode('subagent', at, { agent: '  ' });
    const badName = createNode('subagent', at, { agent: '不是合法名!' });
    const ghost = agent();
    const { errors, warnings } = validateDraft([missing, badName, ghost], [], {
      ...SEED, agents: [{ name: 'general', mode: 'subagent', description: '', prompt: '', tools: [], hidden: false, source: 'builtin' }],
    });
    expect(errors.some((e) => e.includes('缺少 agent'))).toBe(true);
    expect(errors.some((e) => e.includes('字母/数字/-/_'))).toBe(true);
    expect(warnings.some((w) => w.includes('explore 未定义'))).toBe(true);
  });

  it('serialize 保留 agent 字段；nodeSummary 显示装配终点', () => {
    const s = agent();
    const graph = serializeGraph([s], []);
    expect(graph.nodes[0].type).toBe('subagent');
    expect(graph.nodes[0].data.agent).toBe('explore');
    expect(nodeSummary(s)).toContain('type=explore');
    const back = deserializeGraph(graph);
    expect(back.nodes[0].data.nodeType).toBe('subagent');
    expect(back.nodes[0].data.agent).toBe('explore');
  });
});

describe('squad 军团（subnet 子沙盘）', () => {
  const member = (agent: string, optional = false) =>
    createNode('subagent', at, optional ? { agent, requirement: 'optional' } : { agent });
  const squadOf = (name: string) => createNode('squad', at, { squad: name });

  it('主图：squad 接受扩展 pass + 义务入边，自身无出边', () => {
    const sq = squadOf('team-1');
    const ext = createExtensionNode(at, { kind: 'skill', name: 's', target: 's', requirement: 'required' });
    const trigger = createNode('trigger', at, { action: 'any' });
    const req = createNode('requirement', at, { toolName: 'read_file' });
    const action = createNode('action', at, { result: 'block' });
    const nodes = [sq, ext, trigger, req, action];
    expect(isConnectionAllowed(nodes, [], { source: ext.id, target: sq.id, sourceHandle: 'pass' }).ok).toBe(true);
    expect(isConnectionAllowed(nodes, [], { source: ext.id, target: sq.id, sourceHandle: 'fail' }).ok).toBe(false);
    // 义务入边：trigger out / 条件 pass
    expect(isConnectionAllowed(nodes, [], { source: trigger.id, target: sq.id, sourceHandle: 'out' }).ok).toBe(true);
    expect(isConnectionAllowed(nodes, [], { source: req.id, target: sq.id, sourceHandle: 'pass' }).ok).toBe(true);
    expect(isConnectionAllowed(nodes, [], { source: req.id, target: sq.id, sourceHandle: 'fail' }).ok).toBe(false);
    expect(isConnectionAllowed(nodes, [], { source: sq.id, target: action.id, sourceHandle: 'pass' }).ok).toBe(false);
  });

  it('军团内：成员 dispatch 出口连成员，指挥唯一且不可被指向', () => {
    const c = member('cto');
    const m1 = member('explore');
    const m2 = member('reviewer');
    const nodes = [c, m1, m2];
    expect(isSquadConnectionAllowed(nodes, [], { source: c.id, target: m1.id, sourceHandle: 'dispatch' }).ok).toBe(true);
    // 成员 pass/普通出口被拒
    expect(isSquadConnectionAllowed(nodes, [], { source: c.id, target: m1.id, sourceHandle: 'pass' }).ok).toBe(false);
    const edges: SandboxEdge[] = [{ id: 'd1', source: c.id, target: m1.id, sourceHandle: 'dispatch', targetHandle: 'in' }];
    // 第二指挥被拒
    expect(isSquadConnectionAllowed(nodes, edges, { source: m1.id, target: m2.id, sourceHandle: 'dispatch' }).ok).toBe(false);
    // 指挥不可被 dispatch 指向
    expect(isSquadConnectionAllowed(nodes, edges, { source: c.id, target: c.id, sourceHandle: 'dispatch' }).ok).toBe(false);
  });

  it('军团内：扩展只能 pass→成员；非成员目标与其它节点类型被拒', () => {
    const m = member('explore');
    const ext = createExtensionNode(at, { kind: 'skill', name: 's', target: 's', requirement: 'required' });
    const ext2 = createExtensionNode(at, { kind: 'skill', name: 's2', target: 's2', requirement: 'required' });
    const nodes = [m, ext, ext2];
    expect(isSquadConnectionAllowed(nodes, [], { source: ext.id, target: m.id, sourceHandle: 'pass' }).ok).toBe(true);
    expect(isSquadConnectionAllowed(nodes, [], { source: ext.id, target: m.id, sourceHandle: 'fail' }).ok).toBe(false);
    expect(isSquadConnectionAllowed(nodes, [], { source: ext.id, target: ext2.id, sourceHandle: 'pass' }).ok).toBe(false);
  });

  it('validateSquadDraft：成员数/节点类型/孤儿检测', () => {
    // 无成员
    expect(validateSquadDraft('t', [createExtensionNode(at, { kind: 'skill', name: 's', target: 's', requirement: 'required' })], []).some((e) => e.includes('没有成员'))).toBe(true);
    // 非法节点类型（trigger 不能进军团）
    expect(validateSquadDraft('t', [member('a'), createNode('trigger', at, { action: 'any' })], []).some((e) => e.includes('只允许'))).toBe(true);
    // 缺 agent（空串会被 createNode 默认值兜住，用空格触发空校验）
    expect(validateSquadDraft('t', [member(' ')], []).some((e) => e.includes('缺少 agent'))).toBe(true);
    // 合法：指挥 + 成员 + 装配
    const c = member('cto');
    const m = member('explore');
    const ext = createExtensionNode(at, { kind: 'skill', name: 's', target: 's', requirement: 'required' });
    const edges: SandboxEdge[] = [
      { id: 'd', source: c.id, target: m.id, sourceHandle: 'dispatch', targetHandle: 'in' },
      { id: 'b', source: ext.id, target: m.id, sourceHandle: 'pass', targetHandle: 'in' },
    ];
    expect(validateSquadDraft('t', [c, m, ext], edges)).toEqual([]);
  });

  it('serialize/deserialize 往返保留 squads 与成员 requirement', () => {
    const sq = squadOf('team-1');
    const ext = createExtensionNode(at, { kind: 'skill', name: 's', target: 's', requirement: 'required' });
    const mainEdges: SandboxEdge[] = [{ id: 'ge', source: ext.id, target: sq.id, sourceHandle: 'pass', targetHandle: 'in' }];
    const c = member('cto');
    const m = member('explore', true);
    const squadEdges: SandboxEdge[] = [{ id: 'd', source: c.id, target: m.id, sourceHandle: 'dispatch', targetHandle: 'in' }];
    const graph = serializeGraph([sq, ext], mainEdges, { 'team-1': { nodes: [c, m], edges: squadEdges } });
    expect(graph.squads?.['team-1'].nodes).toHaveLength(2);
    expect(graph.squads?.['team-1'].edges[0].sourceHandle).toBe('dispatch');
    expect(graph.squads?.['team-1'].nodes[1].data.requirement).toBe('optional');
    const back = deserializeGraph(graph);
    expect(back.squads['team-1'].nodes[1].data.requirement).toBe('optional');
    expect(back.squads['team-1'].edges[0].sourceHandle).toBe('dispatch');
  });

  it('validateDraft：squad 节点缺名/引用缺失/重名', () => {
    const missing = squadOf('  ');
    expect(validateDraft([missing], [], SEED, {}).errors.some((e) => e.includes('缺少 squad'))).toBe(true);
    const ghost = squadOf('nope');
    expect(validateDraft([ghost], [], SEED, {}).errors.some((e) => e.includes('不存在于 squads'))).toBe(true);
    const dup = [squadOf('team-1'), squadOf('team-1')];
    expect(validateDraft(dup, [], SEED, { 'team-1': { nodes: [member('a')], edges: [] } }).errors.some((e) => e.includes('重名'))).toBe(true);
  });

  it('layoutSquadTree：指挥置顶、成员居中、扩展底层', () => {
    const c = member('cto');
    const m1 = member('explore');
    const m2 = member('reviewer');
    const ext = createExtensionNode(at, { kind: 'skill', name: 's', target: 's', requirement: 'required' });
    const edges: SandboxEdge[] = [
      { id: 'd1', source: c.id, target: m1.id, sourceHandle: 'dispatch', targetHandle: 'in' },
      { id: 'd2', source: c.id, target: m2.id, sourceHandle: 'dispatch', targetHandle: 'in' },
      { id: 'b1', source: ext.id, target: m1.id, sourceHandle: 'pass', targetHandle: 'in' },
    ];
    const laid = layoutSquadTree([ext, m2, c, m1], edges);
    const y = (id: string) => laid.find((n) => n.id === id)!.position.y;
    expect(y(c.id)).toBeLessThan(y(m1.id));
    expect(y(m1.id)).toBe(y(m2.id));
    expect(y(m1.id)).toBeLessThan(y(ext.id));
  });

  it('squad-team 预置模板：主图 squad + 可选共享扩展，子图指挥 dispatch 必选成员', () => {
    const built = SANDBOX_TEMPLATES.find((t) => t.id === 'squad-team')!.build(SEED);
    const squad = built.nodes.find((n) => n.data.nodeType === 'squad');
    expect(squad?.data.squad).toBe('research-team');
    // 主图：可选 skill 扩展 pass → squad（全员共享授权演示）。
    const ext = built.nodes.find((n) => n.data.nodeType === 'extension');
    expect(ext?.data.extension?.requirement).toBe('optional');
    const shared = built.edges.find((e) => e.target === squad!.id);
    expect(shared?.source).toBe(ext!.id);
    expect(shared?.sourceHandle).toBe('pass');
    // 子图：general 指挥 + explore 必选成员，dispatch 连线。
    const sub = built.squads?.['research-team'];
    expect(sub).toBeDefined();
    const agents = sub!.nodes.filter((n) => n.data.nodeType === 'subagent').map((n) => n.data.agent).sort();
    expect(agents).toEqual(['explore', 'general']);
    expect(sub!.edges).toHaveLength(1);
    expect(sub!.edges[0].sourceHandle).toBe('dispatch');
    const coordinator = sub!.nodes.find((n) => n.id === sub!.edges[0].source);
    expect(coordinator?.data.agent).toBe('general');
    // 生成的子图本身能通过军团校验（零配置可跑）。
    expect(validateSquadDraft('research-team', sub!.nodes, sub!.edges)).toEqual([]);
  });

  it('squadSkeleton：新军团预置 CTO 骨架，可直接序列化往返', () => {
    const skeleton = squadSkeleton();
    expect(skeleton.nodes).toHaveLength(2);
    expect(skeleton.edges).toHaveLength(1);
    const dispatch = skeleton.edges[0];
    expect(dispatch.sourceHandle).toBe('dispatch');
    const coordinator = skeleton.nodes.find((n) => n.id === dispatch.source)!;
    const member = skeleton.nodes.find((n) => n.id === dispatch.target)!;
    expect(coordinator.data.agent).toBe('general');
    expect(member.data.agent).toBe('explore');
    expect(member.data.requirement).toBe('required');
    // 骨架自身通过军团校验 + 序列化往返不丢结构。
    expect(validateSquadDraft('team-x', skeleton.nodes, skeleton.edges)).toEqual([]);
    const graph = serializeGraph([], [], { 'team-x': { nodes: skeleton.nodes, edges: skeleton.edges } });
    const back = deserializeGraph(graph);
    expect(back.squads['team-x'].nodes).toHaveLength(2);
    expect(back.squads['team-x'].edges[0].sourceHandle).toBe('dispatch');
  });
});
