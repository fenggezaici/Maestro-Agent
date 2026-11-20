import { invoke } from '@tauri-apps/api/core';

/// 架构沙盘 wire 契约（与 Rust `domain::architecture` 对应）。
export type GraphScope = 'project' | 'global' | 'effective';
export type TriggerAction = 'modify' | 'read' | 'command' | 'plan' | 'any' | 'prompt';
/** block/warn 都是拦截（warn 仅语气不同）；remind 放行并把消息拼进工具结果。 */
export type ActionResult = 'block' | 'warn' | 'remind';
export type ExtensionKind = 'skill' | 'mcp' | 'plugin';
export type ExtensionRequirement = 'required' | 'optional';
/** session 满足一次持续；run 每轮重新满足；each 每次同链触发后都要重新满足（仅前置关卡）。 */
export type SatisfyScope = 'session' | 'run' | 'each';
/** 关卡时机：before=调用前必须满足；after=触发后必须完成验收。 */
export type GateTiming = 'before' | 'after';
/** 事后挂起策略：batch=欠账期间同类触发放行累积；strict=同类触发也拦截。 */
export type GateMode = 'batch' | 'strict';

export interface GraphPosition { x: number; y: number }

export interface ExtensionData {
  kind: ExtensionKind;
  /** 显示名（skill 名 / server 名 / 插件名）。 */
  name: string;
  /** 实际目标：skill 名、MCP server 名或插件名。 */
  target: string;
  /** 仅 mcp：限定具体工具名；缺省为该 server 任意工具。 */
  tool?: string;
  requirement: ExtensionRequirement;
  /** session：满足一次持续有效；run：每轮 Run 需重新满足。 */
  scope?: SatisfyScope;
  /** 事后模式：触发调用后必须实际调用本扩展完成验收。 */
  timing?: GateTiming;
  /** 仅 timing=after 有效：batch 累积放行 / strict 同类也拦。 */
  mode?: GateMode;
  params?: Record<string, string>;
}

export interface GraphNodeData {
  targetDir?: string;
  action?: TriggerAction;
  toolName?: string;
  targetFile?: string;
  updatePlan?: boolean;
  scope?: SatisfyScope;
  /** Requirement 时机：before 前置满足 / after 事后验收。 */
  timing?: GateTiming;
  /** 仅 timing=after 有效。 */
  mode?: GateMode;
  denyRule?: string;
  result?: ActionResult;
  message?: string;
  extension?: ExtensionData;
  /** 仅 subagent 节点：绑定的子智能体类型名（AgentDefinition.name）。 */
  agent?: string;
}

export interface GraphNode {
  id: string;
  type: 'trigger' | 'requirement' | 'restriction' | 'action' | 'extension' | 'subagent';
  position: GraphPosition;
  data: GraphNodeData;
}

export interface GraphEdge {
  id: string;
  source: string;
  target: string;
  sourceHandle?: string;
  targetHandle?: string;
}

export interface ArchitectureGraph {
  version: number;
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface ArchitectureGraphView {
  /** 本次请求的作用域。 */
  scope: 'project' | 'global';
  /** graph 实际来自哪个作用域（文件不存在时为 null）。 */
  resolvedScope: 'project' | 'global' | null;
  /** 运行期真正生效的作用域。 */
  effectiveScope: 'project' | 'global' | null;
  path: string | null;
  graph: ArchitectureGraph | null;
  error: string | null;
}

export interface GraphSaveResult {
  path: string;
  scope: 'project' | 'global';
  /** true = 空画布保存导致该作用域规则被删除（回退到另一作用域）。 */
  removed: boolean;
  effectiveScope: 'project' | 'global' | null;
}

export const architectureApi = {
  load: (scope: GraphScope = 'effective') =>
    invoke<ArchitectureGraphView>('get_architecture_graph', { scope }),
  save: (graph: ArchitectureGraph, scope: 'project' | 'global') =>
    invoke<GraphSaveResult>('save_architecture_graph', { scope, graph }),
};
