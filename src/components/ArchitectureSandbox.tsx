/// 架构沙盘：可视化规则编排（Node Graph）。
/// Trigger → Requirement / Restriction / Extension(Skill·MCP·Plugin) → Action
/// 组成 DAG，序列化为 `.maestro/graph_rules.json` 由 Rust 在工具执行前权威拦截。
import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent, type PointerEvent as ReactPointerEvent } from 'react';
import {
  Background, BackgroundVariant, BaseEdge, Controls, Handle, MiniMap, Panel, Position,
  ReactFlow, ReactFlowProvider, SelectionMode, addEdge, getBezierPath, useEdgesState,
  useNodesState, useReactFlow, type Connection, type Edge, type EdgeProps,
  type FinalConnectionState, type Node, type NodeProps, type XYPosition,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import {
  BookOpenCheck, Bot, ChevronDown, CircleAlert, FileCode2, Flag,
  Layers, ListChecks, Plus, Puzzle, Save, ShieldAlert, Sparkles, WandSparkles,
} from 'lucide-react';
import { architectureApi, type ArchitectureGraph, type ExtensionData, type ExtensionKind, type GateMode, type GateTiming, type GraphNodeData, type SatisfyScope, type TriggerAction } from '../api/architecture';
import { listAgentsApi, type AgentDefinition } from '../api/agents';
import type { ArchitectureTrace } from '../api/types';
import type { ExtensionsController } from '../hooks/useExtensions';
import {
  ACTION_LABELS, NODE_LABELS, SANDBOX_TEMPLATES, createExtensionNode, createNode,
  deserializeGraph, edgeTone, isConnectionAllowed, nodeSummary, remapIds, sampleGraph,
  serializeGraph, traceNodeStatus, effectiveAfterSet, validateDraft, autoActionMessage, type KnownExtensions, type NodeKind,
  type TraceStatus,
} from '../lib/architectureGraph';
import {
  buildLibraryItems, detailForExtension, filterLibrary, libraryTree, readRecentIds,
  recordRecentId, treeLeafCount, type LibraryItem, type LibraryTreeGroup, type PendingConnection,
} from '../lib/nodeLibrary';
import NodeLibrary, { ItemHint } from './NodeLibrary';
import { sandboxFixtureExtensions } from './workbenchMock';
import { Choice } from './ui';

type SandboxNodeData = { nodeType: NodeKind; traceStatus?: TraceStatus } & Record<string, unknown>;
type SandboxNode = Node<SandboxNodeData, 'maestro'>;
type SandboxEdge = Edge;

const KNOWN_TOOLS = [
  'read_file', 'read_files', 'list_files', 'grep_files', 'semantic_search',
  'write_file', 'apply_patch', 'run_command', 'update_plan', 'load_skill',
  'spawn_subagent', 'load_agent', 'web_search', 'web_fetch', 'use_skill', 'ask_user',
];

const KIND_ICON: Record<NodeKind, typeof Flag> = {
  trigger: Flag,
  requirement: BookOpenCheck,
  restriction: ShieldAlert,
  action: CircleAlert,
  extension: Puzzle,
  subagent: Bot,
};

const STATUS_LABEL: Record<TraceStatus, string> = {
  matched: '命中', passed: '已通过', optional: '可选', failed: '未通过', blocked: '已拦截',
  pending: '待验收', reminded: '已提醒',
};

// ---- 自定义节点 -----------------------------------------------------------------

function MaestroNode({ data, selected }: NodeProps<SandboxNode>) {
  const kind = data.nodeType as NodeKind;
  const summary = nodeSummary({ id: '', type: kind, position: { x: 0, y: 0 }, data } as never);
  const status = data.traceStatus as TraceStatus | undefined;
  const ext = data.extension as ExtensionData | undefined;
  // 有效时机：注入的 effectiveAfter（含 pass 边继承）优先于节点自身字段。
  const rawAfter = kind === 'requirement'
    ? data.timing === 'after'
    : kind === 'extension' && ext?.timing === 'after';
  const isAfter = (data.effectiveAfter as boolean | undefined) ?? rawAfter;
  // raw 未设时机但有效 after → 徽标标注「继承」，让用户看清明细。
  const inherited = isAfter && !(kind === 'requirement' ? data.timing : ext?.timing);
  const title = kind === 'extension'
    ? `${ext?.kind === 'skill' ? '📜' : ext?.kind === 'mcp' ? '🧩' : '🔌'} ${ext?.name || ext?.target || NODE_LABELS.extension}`
    : kind === 'trigger' ? `⚡ ${NODE_LABELS.trigger}`
    : kind === 'requirement' ? (isAfter ? '📋 事后要求' : `📋 ${NODE_LABELS.requirement}`)
    : kind === 'restriction' ? `🚧 ${NODE_LABELS.restriction}`
    : kind === 'subagent' ? `🤖 ${String(data.agent ?? '') || NODE_LABELS.subagent}` : `⛔ ${NODE_LABELS.action}`;
  const Icon = KIND_ICON[kind];
  const hasTarget = kind !== 'trigger';
  const handles = kind === 'trigger' ? ['out'] : kind === 'action' ? [] : ['pass', 'fail'];
  const extDetail = detailForExtension(ext);
  const node = (
    <div className={`maestro-node kind-${kind} ${isAfter ? 'timing-after' : ''} ${selected ? 'selected' : ''} ${status ? `trace-${status}` : ''}`}>
      {hasTarget && <Handle id="in" type="target" position={Position.Left} className="maestro-handle in" />}
      <div className="maestro-node-head">
        <Icon size={13} className="maestro-node-icon" />
        <span className="maestro-node-title">{title}</span>
      </div>
      {/* 徽标独立成行：head 右侧是 pass/fail 出口标签的绝对定位区，避免重叠。 */}
      {(isAfter || status) && (
        <div className="maestro-node-badges">
          {isAfter && <span className="maestro-node-timing">{inherited ? '事后·继承' : '事后'}</span>}
          {status && <span className={`maestro-node-status status-${status}`}>{STATUS_LABEL[status]}</span>}
        </div>
      )}
      <div className="maestro-node-body">{summary}</div>
      {kind === 'trigger' && <Handle id="out" type="source" position={Position.Right} className="maestro-handle out" />}
      {kind !== 'trigger' && kind !== 'action' && kind !== 'subagent' && <>
        <Handle id="pass" type="source" position={Position.Right} className="maestro-handle pass" style={{ top: '32%' }} />
        <span className="maestro-handle-tag tag-pass" style={{ top: '24%' }}>通过</span>
        {(kind !== 'extension' || ext?.requirement !== 'optional') && <>
          <Handle id="fail" type="source" position={Position.Right} className="maestro-handle fail" style={{ top: '76%' }} />
          <span className="maestro-handle-tag tag-fail" style={{ top: '68%' }}>未通过</span>
        </>}
      </>}
    </div>
  );
  // 扩展节点 hover 出详情卡；详情经 extensionHints 反查，不写入序列化数据。
  return kind === 'extension' ? <ItemHint title={title} detail={extDetail} side="top">{node}</ItemHint> : node;
}

// ---- 自定义边：贝塞尔 + 语义色 + 执行高亮动画 ---------------------------------------

function MaestroEdge({ id, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, data, selected }: EdgeProps) {
  const [path] = getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition });
  const tone = (data?.tone as 'fail' | 'warn' | 'default' | undefined) ?? 'default';
  const active = Boolean(data?.active);
  return <>
    <BaseEdge id={id} path={path} className={`maestro-edge edge-${tone} ${selected ? 'selected' : ''}`} />
    {active && <circle r="3.5" className={`maestro-edge-dot dot-${tone}`}><animateMotion dur="1.6s" path={path} repeatCount="indefinite" /></circle>}
  </>;
}

const nodeTypes = { maestro: MaestroNode };
const edgeTypes = { maestro: MaestroEdge };
const DND_MIME = 'application/maestro-node';

// ---- 物料区卡片（LibraryItem 驱动：指针拖拽 + 点击双通道） -------------------------
// 说明：不用 HTML5 DnD——`.workbench` 的 CSS zoom（fontScale）会让 Chromium/WebView2
// 的 dragstart 命中检测失效；改为 pointerdown/move/up 自实现拖拽，坐标系自洽。

interface CardDrag {
  x: number; y: number; pointerId: number; active: boolean;
}

function PaletteCard({ entry, compact, onPlace }: {
  entry: LibraryItem;
  compact?: boolean;
  onPlace: (entry: LibraryItem, client?: { x: number; y: number }) => void;
}) {
  const ext = entry.extension;
  const swatch = ext ? `ext-${ext.kind}` : entry.nodeKind;
  const drag = useRef<CardDrag | null>(null);
  const [ghost, setGhost] = useState<{ x: number; y: number } | null>(null);

  const onPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || entry.disabled) return;
    drag.current = { x: event.clientX, y: event.clientY, pointerId: event.pointerId, active: false };
    event.currentTarget.setPointerCapture(event.pointerId);
  };
  const onPointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const state = drag.current;
    if (!state || event.pointerId !== state.pointerId) return;
    if (!state.active && Math.hypot(event.clientX - state.x, event.clientY - state.y) > 6) state.active = true;
    if (state.active) setGhost({ x: event.clientX, y: event.clientY });
  };
  const onPointerUp = (event: ReactPointerEvent<HTMLDivElement>) => {
    const state = drag.current;
    if (!state || event.pointerId !== state.pointerId) return;
    drag.current = null;
    setGhost(null);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    if (!state.active) {
      onPlace(entry);
      return;
    }
    // ghost 是 pointer-events:none，elementFromPoint 返回真实落点元素。
    const overPane = document.elementFromPoint(event.clientX, event.clientY)?.closest('.react-flow');
    if (overPane) onPlace(entry, { x: event.clientX, y: event.clientY });
  };

  // 拖拽/点击提示并入详情卡 usage；禁用原因在 meta 之外单独置灰显示。
  const hintDetail = entry.disabled
    ? { ...entry.detail, usage: entry.disabled }
    : entry.detail;
  return (
    <>
      <ItemHint title={entry.title} detail={hintDetail} open={ghost ? false : undefined}>
        <div className={`palette-card palette-${swatch} ${compact ? 'compact' : ''} ${entry.disabled ? 'disabled' : ''} ${ghost ? 'dragging' : ''}`}
          onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp}
          onPointerCancel={() => { drag.current = null; setGhost(null); }}>
          {entry.badge && <span className="palette-badge">{entry.badge}</span>}
          <span className="palette-card-title">{entry.title}</span>
          {!compact && entry.subtitle && <span className="palette-card-sub">{entry.subtitle}</span>}
          <span className="palette-card-add" aria-hidden>＋</span>
        </div>
      </ItemHint>
      {ghost && (
        <div className={`palette-drag-ghost palette-${swatch}`} style={{ left: ghost.x + 10, top: ghost.y - 14 }} aria-hidden>
          {entry.title}
        </div>
      )}
    </>
  );
}

/// 左侧分类树：顶层组恒展开；子文件夹叶子 >8 时默认折叠；子文件夹内用紧凑单行卡。
function PaletteTree({ group, depth, onPlace }: {
  group: LibraryTreeGroup; depth: number; onPlace: (entry: LibraryItem, client?: { x: number; y: number }) => void;
}) {
  const count = treeLeafCount(group);
  if (count === 0) return null;
  return (
    <details className={`palette-tree depth-${depth}`} open={depth === 0 || count <= 8} data-cat={depth === 0 ? group.label : undefined}>
      <summary className="palette-heading">
        <span className="palette-heading-label">{group.label}</span><em className="palette-count">{count}</em>
      </summary>
      {group.children.map((child) => <PaletteTree key={child.label} group={child} depth={depth + 1} onPlace={onPlace} />)}
      {group.items.map((entry) => <PaletteCard key={entry.id} entry={entry} compact={depth > 0} onPlace={onPlace} />)}
    </details>
  );
}

// ---- 属性面板 -----------------------------------------------------------------

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return <label className="prop-field"><span className="prop-label">{label}</span>{children}</label>;
}

function PropertyPanel({ node, known, effectiveAfter, upstream, onChange, onDelete }: {
  node: SandboxNode;
  known: KnownExtensions;
  /** 节点当前的有效时机是否为事后（含 pass 边继承，供提示行显示）。 */
  effectiveAfter: boolean;
  /** 全部入边来源节点（fail 优先；Action 预览 merge 派生文案用）。 */
  upstream?: SandboxNode[];
  onChange: (id: string, patch: Partial<SandboxNodeData>) => void;
  onDelete: (id: string) => void;
}) {
  const data = node.data;
  const set = (patch: Partial<SandboxNodeData>) => onChange(node.id, patch);
  const setExt = (patch: Partial<ExtensionData>) =>
    set({ extension: { ...(data.extension as ExtensionData), ...patch } });

  const ext = data.extension as ExtensionData | undefined;
  const mcpServer = ext?.kind === 'mcp' ? known.servers.find((item) => item.name === ext.target) : undefined;

  return (
    <div className="prop-panel">
      <header className="prop-head">
        <span className={`prop-kind kind-${data.nodeType}`}>{NODE_LABELS[data.nodeType as NodeKind]}</span>
        <span className="prop-id">{node.id}</span>
      </header>

      {data.nodeType === 'trigger' && <>
        <Field label="动作">
          <Choice label="触发动作" value={String(data.action ?? 'modify')} onChange={(v) => set({ action: v as TriggerAction })}
            options={(Object.keys(ACTION_LABELS) as TriggerAction[]).map((a) => ({ value: a, label: ACTION_LABELS[a] }))} />
        </Field>
        <Field label={data.action === 'prompt' ? '用户输入关键词' : '目标路径 / 命令子串'}>
          <input className="prop-input" value={String(data.targetDir ?? '')}
            placeholder={data.action === 'prompt' ? '如：重构 / UI / 数据库；留空 = 任意输入' : 'src/ui、docs；command 填子串；留空 = 任意'}
            onChange={(e) => set({ targetDir: e.target.value })} />
        </Field>
        {data.action === 'prompt' && (
          <p className="prop-note">命中后该链对本轮 Run 的全部工具调用生效。</p>
        )}
      </>}

      {data.nodeType === 'requirement' && <>
        <Field label="要求工具">
          <input className="prop-input" list="maestro-tool-names" value={String(data.toolName ?? '')} placeholder="read_file / mcp__server__tool"
            onChange={(e) => set({ toolName: e.target.value || undefined })} />
        </Field>
        <Field label="要求路径（可选）">
          <input className="prop-input" value={String(data.targetFile ?? '')} placeholder="docs/architecture.md"
            onChange={(e) => set({ targetFile: e.target.value || undefined })} />
        </Field>
        <label className="prop-check">
          <input type="checkbox" checked={Boolean(data.updatePlan)}
            onChange={(e) => set({ updatePlan: e.target.checked || undefined })} />
          要求先调用 update_plan
        </label>
        <Field label="满足范围">
          <Choice label="满足范围" value={String(data.scope ?? 'session')} onChange={(v) => set({ scope: v as SatisfyScope })}
            options={[
              { value: 'session', label: '会话级（一次满足）' },
              { value: 'run', label: '每轮运行重新满足' },
              { value: 'each', label: '每次触发重新满足' },
            ]} />
        </Field>
        <Field label="时机">
          <Choice label="时机" value={String(data.timing ?? 'follow')} onChange={(v) => {
            if (v === 'follow') {
              set({ timing: undefined, mode: undefined });
              return;
            }
            const timing = v as GateTiming;
            set({ timing, ...(timing !== 'after' ? { mode: undefined } : {}) });
          }} options={[
            { value: 'follow', label: '跟随上游（默认）' },
            { value: 'before', label: '事前（固定）' },
            { value: 'after', label: '事后验收' },
          ]} />
        </Field>
        {!data.timing && effectiveAfter && (
          <p className="prop-note">当前继承上游「事后」语义：本节点也是验收义务。选「事前」可固定为事前。</p>
        )}
        {data.timing === 'after' && <>
          <Field label="挂起策略">
            <Choice label="挂起策略" value={String(data.mode ?? 'batch')} onChange={(v) => set({ mode: v as GateMode })}
              options={[
                { value: 'batch', label: '批量累积（可连改多次再统一验收）' },
                { value: 'strict', label: '严格逐改（改一次必须审一次）' },
              ]} />
          </Field>
          <p className="prop-note">触发调用执行后欠下验收：读/计划/提问放行，其余实质操作被拦，Run 结束未验收会被追问。</p>
        </>}
      </>}

      {data.nodeType === 'restriction' && <>
        <Field label="禁止规则">
          <input className="prop-input" value={String(data.denyRule ?? '')} placeholder="src/db、.env、rm -rf"
            onChange={(e) => set({ denyRule: e.target.value })} />
        </Field>
        <Field label="匹配范围">
          <Choice label="匹配范围" value={String(data.scope ?? 'session')} onChange={(v) => set({ scope: v as SatisfyScope })}
            options={[{ value: 'session', label: '路径 + 内容' }, { value: 'run', label: '仅路径' }]} />
        </Field>
      </>}

      {data.nodeType === 'action' && <>
        <Field label="结果">
          <Choice label="拦截结果" value={String(data.result ?? 'block')} onChange={(v) => set({ result: v as 'block' | 'warn' | 'remind' })}
            options={[
              { value: 'block', label: 'Block 拦截（红）' },
              { value: 'warn', label: 'Warn 警告拦截（黄）' },
              { value: 'remind', label: 'Remind 提醒（放行+附言）' },
            ]} />
          {data.result === 'remind' && (
            <p className="prop-note">提醒不会拦截：调用照常执行，本消息拼进工具结果喂给模型。适合做「时刻提醒」。</p>
          )}
        </Field>
        <Field label="拦截提示语（返回给模型）">
          <textarea className="prop-input prop-textarea" rows={3} value={String(data.message ?? '')}
            placeholder="不允许在 UI 层引入 DB 层，请重写。" onChange={(e) => set({ message: e.target.value })} />
        </Field>
        <p className="prop-note">
          {'占位符自动填触发关卡信息：{call} 完整调用（如「调用 load_skill ui-review」）、{target} 目标、{tool} 工具、{name} 名称。'}
        </p>
        {!String(data.message ?? '').trim() && upstream && upstream.length > 0 && (
          <p className="prop-note">
            留空将自动生成（按实际触发关卡；多路同时失败时逐条列出）：
            <br />
            {upstream.slice(0, 3).map((src, i) => (
              <span key={src.id}>{i + 1}. {autoActionMessage(src)}<br /></span>
            ))}
            {upstream.length > 3 && `等 ${upstream.length} 项`}
          </p>
        )}
        {!String(data.message ?? '').trim() && (!upstream || upstream.length === 0) && (
          <p className="prop-note">留空将自动生成：{autoActionMessage(undefined)}</p>
        )}
      </>}

      {data.nodeType === 'extension' && ext && <>
        <Field label="扩展类型">
          <Choice label="扩展类型" value={ext.kind} onChange={(v) => {
            const kind = v as ExtensionKind;
            const target = kind === 'skill' ? known.skills[0]?.name ?? ''
              : kind === 'mcp' ? known.servers[0]?.name ?? ''
              : known.plugins[0]?.name ?? '';
            set({ extension: { ...ext, kind, name: target, target, tool: undefined } });
          }} options={[{ value: 'skill', label: '📜 Skill' }, { value: 'mcp', label: '🧩 MCP' }, { value: 'plugin', label: '🔌 插件' }]} />
        </Field>
        <Field label={ext.kind === 'mcp' ? 'MCP 服务' : ext.kind === 'skill' ? '技能' : '插件'}>
          {ext.kind === 'skill' && <select className="prop-input" value={ext.target} onChange={(e) => setExt({ target: e.target.value, name: e.target.value })}>
            <option value="">（手动输入）</option>
            {known.skills.map((skill) => <option key={`${skill.source}:${skill.name}`} value={skill.name}>{skill.name}（{skill.source === 'project' ? '项目' : '全局'}）</option>)}
          </select>}
          {ext.kind === 'mcp' && <select className="prop-input" value={ext.target} onChange={(e) => setExt({ target: e.target.value, name: e.target.value, tool: undefined })}>
            <option value="">（手动输入）</option>
            {known.servers.map((server) => <option key={server.name} value={server.name}>{server.name}{server.running ? '' : '（未运行）'}</option>)}
          </select>}
          {ext.kind === 'plugin' && <select className="prop-input" value={ext.target} onChange={(e) => setExt({ target: e.target.value, name: e.target.value })}>
            <option value="">（手动输入）</option>
            {known.plugins.filter((p) => !p.error).map((plugin) => <option key={plugin.name} value={plugin.name}>{plugin.name}</option>)}
          </select>}
          <input className="prop-input" value={ext.target} placeholder="或手动输入名称"
            onChange={(e) => setExt({ target: e.target.value, name: e.target.value })} />
        </Field>
        {ext.kind === 'mcp' && mcpServer && mcpServer.tools.length > 0 && (
          <Field label="限定工具（可选）">
            <select className="prop-input" value={ext.tool ?? ''} onChange={(e) => setExt({ tool: e.target.value || undefined })}>
              <option value="">服务内任意工具</option>
              {mcpServer.tools.map((tool) => <option key={tool.name} value={tool.name}>{tool.name}</option>)}
            </select>
          </Field>
        )}
        <Field label="要求级别">
          <Choice label="要求级别" value={ext.requirement ?? 'required'} onChange={(v) => setExt({ requirement: v as 'required' | 'optional' })}
            options={[{ value: 'required', label: '必选（fail 可连线拦截）' }, { value: 'optional', label: '可选（建议注入）' }]} />
        </Field>
        <Field label="满足范围">
          <Choice label="扩展满足范围" value={ext.scope ?? 'session'} onChange={(v) => setExt({ scope: v as SatisfyScope })}
            options={[
              { value: 'session', label: '会话级' },
              { value: 'run', label: '每轮运行' },
              { value: 'each', label: '每次触发重新满足' },
            ]} />
        </Field>
        <Field label="时机">
          <Choice label="时机" value={ext.timing ?? 'follow'} onChange={(v) => {
            if (v === 'follow') {
              set({ extension: { ...ext, timing: undefined, mode: undefined } });
              return;
            }
            const timing = v as GateTiming;
            set({ extension: { ...ext, timing, ...(timing !== 'after' ? { mode: undefined } : {}) } });
          }} options={[
            { value: 'follow', label: '跟随上游（默认）' },
            { value: 'before', label: '事前（固定）' },
            { value: 'after', label: '事后验收' },
          ]} />
        </Field>
        {!ext.timing && effectiveAfter && (
          <p className="prop-note">当前继承上游「事后」语义：本节点也是验收义务。选「事前」可固定为事前。</p>
        )}
        {ext.timing === 'after' && <>
          <Field label="挂起策略">
            <Choice label="挂起策略" value={ext.mode ?? 'batch'} onChange={(v) => setExt({ mode: v as GateMode })}
              options={[
                { value: 'batch', label: '批量累积（可连改多次再统一验收）' },
                { value: 'strict', label: '严格逐改（改一次必须审一次）' },
              ]} />
          </Field>
          <p className="prop-note">触发调用执行后欠下验收：模型须实际调用本扩展完成审查/测试；读/计划/提问放行，其余实质操作被拦。</p>
        </>}
        <ExtensionUsage ext={ext} />
      </>}

      {data.nodeType === 'subagent' && <>
        <Field label="子智能体类型">
          <select className="prop-input" value={String(data.agent ?? '')} onChange={(e) => set({ agent: e.target.value || undefined })}>
            <option value="">（手动输入）</option>
            {(known.agents ?? []).map((agent) => (
              <option key={agent.name} value={agent.name}>
                {agent.name}（{agent.source === 'project' ? '项目' : agent.source === 'global' ? '全局' : '内置'}）
              </option>
            ))}
          </select>
          <input className="prop-input" value={String(data.agent ?? '')} placeholder="或手动输入类型名（如 explore）"
            onChange={(e) => set({ agent: e.target.value })} />
        </Field>
        <p className="prop-note">
          装配终点：扩展节点用「通过」出口连入本节点——spawn_subagent(type={String(data.agent ?? '') || '…'}) 派发时，
          子循环获得这些扩展能力；必选条目强制真实调用后才能收尾，可选条目只注入建议。本节点不参与调用链求值。
        </p>
        {upstream && upstream.length > 0 && (
          <div className="prop-detail">
            <p className="prop-detail-title">已绑定扩展（{upstream.length}）</p>
            <dl className="prop-detail-meta">
              {upstream.map((src) => {
                const srcExt = src.data.extension as ExtensionData | undefined;
                return (
                  <div key={src.id} className="prop-detail-meta-row">
                    <dt>{srcExt?.kind === 'skill' ? 'Skill' : srcExt?.kind === 'mcp' ? 'MCP' : '插件'}</dt>
                    <dd title={srcExt?.tool ? `${srcExt.target}/${srcExt.tool}` : srcExt?.target}>
                      {srcExt?.name || srcExt?.target || src.id}{srcExt?.tool ? ` / ${srcExt.tool}` : ''} · {srcExt?.requirement === 'optional' ? '可选' : '必选'}
                    </dd>
                  </div>
                );
              })}
            </dl>
          </div>
        )}
      </>}

      <button type="button" className="prop-delete" onClick={() => onDelete(node.id)}>删除节点</button>
    </div>
  );
}

/// 属性面板「用途」块：按当前 ext 反查扩展详情（与节点库同一份 detail），
/// 手动输入的 target 不在清单中时如实标注，不编造描述。
function ExtensionUsage({ ext }: { ext: ExtensionData }) {
  const detail = detailForExtension(ext);
  return (
    <div className="prop-detail">
      <p className="prop-detail-title">用途</p>
      {detail?.description && <p className="prop-detail-desc">{detail.description}</p>}
      {!detail && <p className="prop-detail-desc muted">「{ext.target || '未指定'}」不在当前扩展清单中（手动输入）。</p>}
      {detail?.meta && detail.meta.length > 0 && (
        <dl className="prop-detail-meta">
          {detail.meta.map(([label, value]) => (
            <div key={label} className="prop-detail-meta-row"><dt>{label}</dt><dd title={value}>{value}</dd></div>
          ))}
        </dl>
      )}
      <p className="prop-detail-usage">{detail?.usage ?? '必选：触发器命中后，模型须实际调用该扩展，否则关联工具调用被拦截。'}</p>
    </div>
  );
}

// ---- 主页面 ---------------------------------------------------------------------

interface SandboxProps {
  preview: boolean;
  extensions: ExtensionsController;
  trace: ArchitectureTrace | null;
}

function SandboxInner({ preview, extensions, trace }: SandboxProps) {
  const [nodes, setNodes, onNodesChange] = useNodesState<SandboxNode>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<SandboxEdge>([]);
  const [scope, setScope] = useState<'project' | 'global'>(extensions.hasWorkspace ? 'project' : 'global');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [status, setStatus] = useState<{ kind: 'info' | 'ok' | 'error' | 'warn'; text: string } | null>(null);
  const [reloadKey, setReloadKey] = useState(0);
  const [issues, setIssues] = useState<{ errors: string[]; warnings: string[] } | null>(null);
  const [loaded, setLoaded] = useState<{ path: string | null; source: string } | null>(null);
  const [dirty, setDirty] = useState(false);
  const [showJson, setShowJson] = useState(false);
  const [templateOpen, setTemplateOpen] = useState(false);
  const [paletteQuery, setPaletteQuery] = useState('');
  const [library, setLibrary] = useState<{
    x: number; y: number; flow: XYPosition;
    pending?: PendingConnection; pendingLabel?: string;
  } | null>(null);
  const dirtyRef = useRef(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const pointerRef = useRef<{ x: number; y: number } | null>(null);
  const clickOffsetRef = useRef(0);
  const markDirty = useCallback(() => { dirtyRef.current = true; setDirty(true); }, []);
  const { screenToFlowPosition, fitView } = useReactFlow();

  // 可装配子智能体清单：subagent 节点下拉与「未定义」告警共用；preview 走 fixture。
  const [agents, setAgents] = useState<AgentDefinition[]>([]);
  useEffect(() => {
    if (preview) return;
    let cancelled = false;
    listAgentsApi('all')
      .then((list) => { if (!cancelled) setAgents(list); })
      .catch(() => { /* 清单不可达时下拉留空，仍可手动输入 */ });
    return () => { cancelled = true; };
  }, [preview]);

  const known: KnownExtensions = useMemo(() => preview ? sandboxFixtureExtensions() : {
    skills: extensions.skills, servers: extensions.servers, plugins: extensions.plugins,
    agents,
  }, [preview, extensions.skills, extensions.servers, extensions.plugins, agents]);

  // 节点库数据源：左侧分类树与 Tab 浮层共用。
  const libraryItems = useMemo(() => buildLibraryItems(known), [known]);
  const sidebarTree = useMemo(() => libraryTree(libraryItems), [libraryItems]);
  const sidebarResults = useMemo(
    () => (paletteQuery.trim() ? filterLibrary(libraryItems, paletteQuery, null, readRecentIds()) : null),
    [libraryItems, paletteQuery],
  );

  // 加载规则
  useEffect(() => {
    let cancelled = false;
    if (preview) {
      const sample = sampleGraph(known);
      setNodes(sample.nodes as SandboxNode[]);
      setEdges(sample.edges as SandboxEdge[]);
      setLoaded({ path: null, source: 'preview' });
      setDirty(false); dirtyRef.current = false;
      return;
    }
    setStatus({ kind: 'info', text: '加载规则…' });
    // 超时兜底：agent run 期间 IPC/DB 可能排队——loading 永不假死。
    const timeout = new Promise<never>((_, reject) =>
      setTimeout(() => reject(new Error('加载超时（运行中系统繁忙）')), 10_000));
    void Promise.race([architectureApi.load(scope), timeout])
      .then((result) => {
        if (cancelled) return;
        if (result.error) {
          setStatus({ kind: 'error', text: `规则文件损坏：${result.error}` });
        }
        if (result.graph) {
          const draft = deserializeGraph(result.graph);
          setNodes(draft.nodes as SandboxNode[]);
          setEdges(draft.edges as SandboxEdge[]);
        } else {
          setNodes([]); setEdges([]);
        }
        setLoaded({ path: result.path, source: result.resolvedScope ?? scope });
        setDirty(false); dirtyRef.current = false;
        if (!result.error) setStatus(result.resolvedScope && result.resolvedScope !== scope
          ? { kind: 'info', text: `当前作用域无规则，已回退加载${result.resolvedScope === 'global' ? '全局' : '项目'}规则` }
          : null);
      })
      .catch((reason: unknown) => { if (!cancelled) setStatus({ kind: 'error', text: `加载失败：${reason instanceof Error ? reason.message : String(reason)}` }); });
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope, preview, reloadKey]);

  // 校验提示
  const validation = useMemo(() => validateDraft(nodes, edges, known), [nodes, edges, known]);

  // Trace 高亮 + 有效时机注入（事后沿 pass 边继承，与 Rust effective_after_set 同款）
  const statusByNode = useMemo(() => traceNodeStatus(trace?.steps ?? []), [trace]);
  const afterByNode = useMemo(() => effectiveAfterSet(nodes, edges), [nodes, edges]);
  const displayNodes = useMemo(() => nodes.map((node) => ({
    ...node,
    data: { ...node.data, traceStatus: statusByNode[node.id], effectiveAfter: afterByNode.has(node.id) },
  })), [nodes, statusByNode, afterByNode]);
  const displayEdges = useMemo(() => edges.map((edge) => ({
    ...edge,
    data: {
      tone: edgeTone(edge as never, displayNodes as never),
      active: Boolean(statusByNode[edge.source]),
    },
  })), [edges, displayNodes, statusByNode]);

  const selected = nodes.find((node) => node.id === selectedId) ?? null;
  // Action 预览自动派生文案：全部入边来源（fail 优先——merge 时逐条列出）。
  const upstreamsOf = useCallback((nodeId: string): SandboxNode[] => {
    const incoming = edges.filter((edge) => edge.target === nodeId);
    incoming.sort((a, b) => (a.sourceHandle === 'fail' ? 0 : 1) - (b.sourceHandle === 'fail' ? 0 : 1));
    return incoming
      .map((edge) => nodes.find((node) => node.id === edge.source))
      .filter((node): node is SandboxNode => Boolean(node));
  }, [edges, nodes]);

  const onConnect = useCallback((connection: Connection) => {
    const check = isConnectionAllowed(nodes as never, edges as never, {
      source: connection.source, target: connection.target,
      sourceHandle: connection.sourceHandle, targetHandle: connection.targetHandle,
    });
    if (!check.ok) {
      setStatus({ kind: 'warn', text: `不能连线：${check.reason}` });
      return;
    }
    markDirty();
    setEdges((eds) => addEdge({ ...connection, type: 'maestro', targetHandle: 'in' }, eds));
  }, [nodes, edges, setEdges, markDirty]);

  const isValidConnection = useCallback((connection: Connection | Edge) =>
    isConnectionAllowed(nodes as never, edges as never, {
      source: connection.source ?? '', target: connection.target ?? '',
      sourceHandle: 'sourceHandle' in connection ? connection.sourceHandle : undefined,
      targetHandle: 'in',
    }).ok, [nodes, edges]);

  // 模板/示例链整体平移到落点。
  const insertTemplateAt = useCallback((templateId: string, flow: XYPosition) => {
    const built = templateId === 'sample'
      ? sampleGraph(known)
      : SANDBOX_TEMPLATES.find((item) => item.id === templateId)?.build(known);
    if (!built) return;
    const keyed = remapIds(built.nodes, built.edges);
    const minX = Math.min(...keyed.nodes.map((node) => node.position.x));
    const minY = Math.min(...keyed.nodes.map((node) => node.position.y));
    markDirty();
    setNodes((items) => [...items, ...keyed.nodes.map((node) => ({
      ...node, position: { x: node.position.x - minX + flow.x, y: node.position.y - minY + flow.y },
    })) as SandboxNode[]]);
    setEdges((items) => [...items, ...keyed.edges as SandboxEdge[]]);
    void fitView({ duration: 200 });
  }, [known, markDirty, setNodes, setEdges, fitView]);

  // 统一放置入口：节点库浮层、左侧树点击、DnD 落点、pendingConnection 自动连线。
  const placeItem = useCallback((entry: LibraryItem, flow: XYPosition, pending?: PendingConnection | null) => {
    if (entry.disabled) return;
    if (entry.nodeKind === 'template') {
      if (!entry.templateId) return;
      insertTemplateAt(entry.templateId, flow);
      recordRecentId(entry.id);
      return;
    }
    const node = entry.extension
      ? createExtensionNode(flow, entry.extension)
      : createNode(entry.nodeKind as NodeKind, flow, entry.seed as Partial<GraphNodeData> | undefined);
    markDirty();
    setNodes((items) => [...items, node as SandboxNode]);
    setSelectedId(node.id);
    recordRecentId(entry.id);
    if (pending) {
      // 新节点尚未入列，校验时手动并入，否则 target 查找必失败（"节点不存在"）。
      const check = isConnectionAllowed([...nodes, node] as never, edges as never, {
        source: pending.source, target: node.id, sourceHandle: pending.sourceHandle, targetHandle: 'in',
      });
      if (check.ok) {
        setEdges((eds) => addEdge({
          source: pending.source, sourceHandle: pending.sourceHandle, target: node.id, targetHandle: 'in', type: 'maestro',
        }, eds));
      } else {
        setStatus({ kind: 'warn', text: `不能连线：${check.reason}` });
      }
    }
  }, [nodes, edges, markDirty, setNodes, setEdges, insertTemplateAt]);

  // 浮层打开：client 坐标 → 面板锚点（相对 wrap，夹取边界）+ flow 落点。
  const openLibraryAt = useCallback((clientX: number, clientY: number, pending?: PendingConnection | null) => {
    const wrap = wrapRef.current;
    if (!wrap) return;
    const rect = wrap.getBoundingClientRect();
    const x = Math.min(Math.max(0, clientX - rect.left), Math.max(0, rect.width - 352));
    const y = Math.min(Math.max(0, clientY - rect.top), Math.max(0, rect.height - 400));
    let pendingLabel: string | undefined;
    if (pending) {
      const source = nodes.find((node) => node.id === pending.source);
      pendingLabel = `⚡ ${source ? nodeSummary(source as never) : pending.source} → 选择要连接的节点`;
    }
    setLibrary({ x, y, flow: screenToFlowPosition({ x: clientX, y: clientY }), pending: pending ?? undefined, pendingLabel });
  }, [nodes, screenToFlowPosition]);

  // 左侧树/工具栏：点击放视口中心（级联偏移）；指针拖拽带落点坐标放指定位置。
  const handlePlace = useCallback((entry: LibraryItem, client?: { x: number; y: number }) => {
    if (client) {
      placeItem(entry, screenToFlowPosition(client), null);
      return;
    }
    const rect = wrapRef.current?.getBoundingClientRect();
    const offset = (clickOffsetRef.current = (clickOffsetRef.current + 24) % 120);
    const center = rect
      ? { x: rect.left + rect.width / 2 + offset, y: rect.top + rect.height / 2 + offset }
      : { x: 0, y: 0 };
    placeItem(entry, screenToFlowPosition(center), null);
  }, [placeItem, screenToFlowPosition]);

  // 连线拖到空白松手（ComfyUI link-release）：唤起兼容目标过滤的节点库。
  const onConnectEnd = useCallback((event: MouseEvent | TouchEvent, state: FinalConnectionState) => {
    // 连线拖出在空白处松手（toNode 为空、未命中任何 Handle）：唤起节点库并自动连线。
    if (state.toNode != null || state.isValid != null || !state.fromNode) return;
    const point = 'changedTouches' in event ? event.changedTouches[0] : event;
    openLibraryAt(point.clientX, point.clientY, {
      source: state.fromNode.id,
      sourceHandle: state.fromHandle?.id ?? 'out',
    });
  }, [openLibraryAt]);

  // Tab 唤起（Houdini）：仅当指针悬停在沙盘画布区且不在输入控件中。
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Tab') return;
      const target = event.target as HTMLElement | null;
      if (target?.closest('input, textarea, select, [contenteditable="true"]')) return;
      const wrap = wrapRef.current;
      if (!wrap?.matches(':hover')) return;
      event.preventDefault();
      const rect = wrap.getBoundingClientRect();
      const point = pointerRef.current ?? { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
      openLibraryAt(point.x, point.y, null);
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [openLibraryAt]);

  const onDrop = useCallback((event: DragEvent) => {
    event.preventDefault();
    const flow = screenToFlowPosition({ x: event.clientX, y: event.clientY });
    const raw = event.dataTransfer.getData(DND_MIME) || event.dataTransfer.getData('text/plain');
    if (!raw) {
      setStatus({ kind: 'warn', text: '拖拽数据为空，请改用点击或 Tab 放置' });
      return;
    }
    let entry: LibraryItem | undefined;
    try {
      const payload = JSON.parse(raw) as {
        id?: string; kind?: NodeKind; extension?: ExtensionData; seed?: Partial<GraphNodeData>; templateId?: string;
      };
      if (payload.extension || payload.kind || payload.templateId) {
        entry = {
          id: payload.id ?? `dnd:${payload.kind ?? payload.templateId ?? 'ext'}`,
          title: '', keywords: '', category: [],
          nodeKind: payload.templateId ? 'template' : payload.extension ? 'extension' : payload.kind!,
          extension: payload.extension, seed: payload.seed, templateId: payload.templateId,
        };
      } else if (payload.id) {
        entry = libraryItems.find((item) => item.id === payload.id);
      }
    } catch {
      entry = libraryItems.find((item) => item.id === raw);
    }
    if (entry) placeItem(entry, flow, null);
    else setStatus({ kind: 'warn', text: '无法识别的拖拽内容，请改用点击或 Tab 放置' });
  }, [screenToFlowPosition, libraryItems, placeItem]);

  const patchNode = useCallback((id: string, patch: Partial<SandboxNodeData>) => {
    markDirty();
    setNodes((items) => items.map((node) => node.id === id ? { ...node, data: { ...node.data, ...patch } } : node));
  }, [setNodes, markDirty]);

  const deleteNode = useCallback((id: string) => {
    markDirty();
    setNodes((items) => items.filter((node) => node.id !== id));
    setEdges((items) => items.filter((edge) => edge.source !== id && edge.target !== id));
    setSelectedId(null);
  }, [setNodes, setEdges, markDirty]);

  const insertTemplate = (id: string) => {
    const template = SANDBOX_TEMPLATES.find((item) => item.id === id);
    if (!template) return;
    const built = template.build({ skills: known.skills, servers: known.servers, plugins: known.plugins });
    markDirty();
    setNodes((items) => [...items, ...built.nodes as SandboxNode[]]);
    setEdges((items) => [...items, ...built.edges as SandboxEdge[]]);
    setTemplateOpen(false);
  };

  const runValidation = () => {
    setIssues(validation);
    setStatus(validation.errors.length
      ? { kind: 'error', text: `校验失败：${validation.errors.length} 个错误` }
      : validation.warnings.length
        ? { kind: 'warn', text: `校验通过，${validation.warnings.length} 条提示` }
        : { kind: 'ok', text: '校验通过' });
  };

  const save = async () => {
    if (validation.errors.length) {
      setIssues(validation);
      setStatus({ kind: 'error', text: '存在错误，无法保存' });
      return;
    }
    if (preview) {
      setStatus({ kind: 'warn', text: '预览模式不会写入 Rust 规则文件' });
      return;
    }
    const graph: ArchitectureGraph = serializeGraph(nodes as never, edges as never);
    setStatus({ kind: 'info', text: '保存中…' });
    try {
      const result = await architectureApi.save(graph, scope);
      setLoaded({ path: result.removed ? null : result.path, source: result.scope });
      setDirty(false); dirtyRef.current = false;
      setStatus({ kind: 'ok', text: result.removed ? `空画布已清空该作用域规则（回退生效）` : `已保存到 ${result.path}` });
    } catch (reason) {
      setStatus({ kind: 'error', text: `保存失败：${reason instanceof Error ? reason.message : String(reason)}` });
    }
  };

  const graphJson = useMemo(() => JSON.stringify(serializeGraph(nodes as never, edges as never), null, 2), [nodes, edges]);

  return (
    <div className="sandbox-root">
      {/* 左侧节点库：与 Tab 浮层同源的分类树 */}
      <aside className="sandbox-palette">
        <div className="palette-search">
          <input value={paletteQuery} onChange={(event) => setPaletteQuery(event.target.value)}
            placeholder="筛选节点…" aria-label="筛选节点" />
          {paletteQuery && <button type="button" onClick={() => setPaletteQuery('')} aria-label="清空筛选">×</button>}
        </div>
        {sidebarResults ? (
          <div className="palette-section">
            {sidebarResults.map((section) => (
              <div key={section.label}>
                <p className="palette-heading">{section.label}</p>
                {section.items.map((entry) => <PaletteCard key={entry.id} entry={entry} onPlace={handlePlace} />)}
              </div>
            ))}
            {sidebarResults.every((section) => section.items.length === 0) && (
              <p className="palette-empty">没有匹配的节点</p>
            )}
          </div>
        ) : (
          sidebarTree.map((group) => <PaletteTree key={group.label} group={group} depth={0} onPlace={handlePlace} />)
        )}
        <p className="palette-tip">双击画布或按 Tab 打开节点库；把连线拖到空白处可搜索接入节点。</p>
      </aside>

      {/* 中央画布 */}
      <div className="sandbox-canvas-wrap" ref={wrapRef}
        onPointerMove={(event) => { pointerRef.current = { x: event.clientX, y: event.clientY }; }}
        onPointerDown={(event) => {
          if (library && !(event.target as HTMLElement).closest('.node-library')) setLibrary(null);
        }}>
        <header className="sandbox-toolbar">
          <div className="toolbar-group">
            <Layers size={14} />
            <Choice label="规则作用域" value={scope} onChange={(v) => { if (dirtyRef.current && !window.confirm('未保存的修改将丢失，仍切换作用域？')) return; setScope(v as 'project' | 'global'); }}
              options={[
                { value: 'project', label: `项目规则${extensions.hasWorkspace || preview ? '' : '（需工作区）'}` },
                { value: 'global', label: '全局规则' },
              ]} />
          </div>
          <div className="toolbar-group">
            <button type="button" className="toolbar-btn" title="打开节点库（Tab / 双击画布）"
              onClick={() => {
                const rect = wrapRef.current?.getBoundingClientRect();
                if (rect) openLibraryAt(rect.left + rect.width / 2, rect.top + rect.height / 2, null);
              }}>
              <Plus size={13} /> 节点 <kbd className="toolbar-kbd">Tab</kbd>
            </button>
            <div className="template-picker">
              <button type="button" className="toolbar-btn" onClick={() => setTemplateOpen((v) => !v)}>
                <WandSparkles size={13} /> 预置模板 <ChevronDown size={11} />
              </button>
              {templateOpen && (
                <div className="template-menu">
                  {SANDBOX_TEMPLATES.map((template) => (
                    <button key={template.id} type="button" onClick={() => insertTemplate(template.id)}>
                      <strong>{template.label}</strong><span>{template.description}</span>
                    </button>
                  ))}
                  <button type="button" onClick={() => {
                    const sample = sampleGraph({ skills: known.skills, servers: known.servers, plugins: known.plugins });
                    const keyed = remapIds(sample.nodes, sample.edges);
                    markDirty();
                    setNodes((items) => [...items, ...keyed.nodes as SandboxNode[]]);
                    setEdges((items) => [...items, ...keyed.edges as SandboxEdge[]]);
                    setTemplateOpen(false);
                  }}>
                    <strong>验收示例图</strong><span>Trigger → Skill + MCP → Restriction → Action</span>
                  </button>
                </div>
              )}
            </div>
            <button type="button" className="toolbar-btn" onClick={runValidation}><ListChecks size={13} /> 校验</button>
            <button type="button" className="toolbar-btn" onClick={() => setShowJson((v) => !v)}><FileCode2 size={13} /> JSON</button>
          </div>
          <div className="toolbar-group">
            {loaded && <span className="toolbar-meta">{loaded.source === 'project' ? '项目' : loaded.source === 'global' ? '全局' : '示例'}{dirty && ' · 未保存'}</span>}
            <button type="button" className="toolbar-btn primary" onClick={() => void save()} disabled={preview}>
              <Save size={13} /> 保存沙盘
            </button>
          </div>
        </header>
        {status && <div className={`sandbox-status status-${status.kind}`} role="status">
          {status.text}
          {status.kind === 'error' && (
            <button type="button" className="sandbox-status-retry" onClick={() => setReloadKey((k) => k + 1)}>重试</button>
          )}
        </div>}
        {issues && (issues.errors.length > 0 || issues.warnings.length > 0) && (
          <div className="sandbox-issues">
            {issues.errors.map((text) => <p key={text} className="issue-error">✕ {text}</p>)}
            {issues.warnings.map((text) => <p key={text} className="issue-warn">⚠ {text}</p>)}
          </div>
        )}
        <ReactFlow
          nodes={displayNodes}
          edges={displayEdges}
          nodeTypes={nodeTypes}
          edgeTypes={edgeTypes}
          onNodesChange={(changes) => { onNodesChange(changes); if (changes.some((c) => c.type !== 'select')) markDirty(); }}
          onEdgesChange={(changes) => { onEdgesChange(changes); if (changes.some((c) => c.type !== 'select')) markDirty(); }}
          onConnect={onConnect}
          onConnectEnd={onConnectEnd}
          isValidConnection={isValidConnection}
          onDrop={onDrop}
          onDragOver={(event: DragEvent) => { event.preventDefault(); event.dataTransfer.dropEffect = 'move'; }}
          onDoubleClick={(event) => {
            if ((event.target as HTMLElement).classList.contains('react-flow__pane')) {
              openLibraryAt(event.clientX, event.clientY, null);
            }
          }}
          onSelectionChange={(params) => setSelectedId(params.nodes[0]?.id ?? null)}
          selectionOnDrag
          selectionMode={SelectionMode.Partial}
          panOnDrag={[1, 2]}
          defaultEdgeOptions={{ type: 'maestro' }}
          connectionRadius={32}
          fitView
          proOptions={{ hideAttribution: true }}
          colorMode="dark"
          deleteKeyCode={['Backspace', 'Delete']}
        >
          <Background variant={BackgroundVariant.Dots} gap={22} size={1.4} color="rgba(148,163,184,0.22)" />
          <Controls showInteractive={false} />
          <MiniMap pannable zoomable className="sandbox-minimap" />
          {showJson && (
            <Panel position="bottom-left" className="sandbox-json-panel">
              <pre>{graphJson}</pre>
            </Panel>
          )}
          {trace && (
            <Panel position="top-center" className="sandbox-trace-banner">
              <span className={trace.blocked ? 'trace-blocked' : 'trace-live'}>
                {trace.blocked ? `⛔ ${trace.message ?? '调用被拦截'}` : `▶ 工具 ${trace.tool}（${trace.target || '任意目标'}）按规则放行`}
              </span>
            </Panel>
          )}
        </ReactFlow>
        {nodes.length === 0 && !library && (
          <div className="sandbox-empty-hint" aria-hidden>
            <p>双击空白或按 <kbd>Tab</kbd> 打开节点库</p>
            <p>也可以点击左侧节点卡片直接放置</p>
          </div>
        )}
        {library && (
          <NodeLibrary
            anchor={{ x: library.x, y: library.y }}
            pending={library.pending}
            pendingLabel={library.pendingLabel}
            items={libraryItems}
            onPick={(entry, keepOpen) => {
              placeItem(entry, library.flow, library.pending);
              if (!keepOpen) setLibrary(null);
              else setLibrary((lib) => lib ? {
                ...lib,
                flow: { x: lib.flow.x + 28, y: lib.flow.y + 28 },
                pending: undefined, pendingLabel: undefined,
              } : lib);
            }}
            onClose={() => setLibrary(null)}
          />
        )}
      </div>

      {/* 右侧属性面板 */}
      <aside className="sandbox-props">
        {selected ? (
          <PropertyPanel node={selected} known={known} effectiveAfter={afterByNode.has(selected.id)} upstream={upstreamsOf(selected.id)} onChange={patchNode} onDelete={deleteNode} />
        ) : (
          <div className="prop-empty">
            <Sparkles size={20} strokeWidth={1.5} />
            <p>从左侧拖入节点，或选择画布中的节点编辑属性。</p>
            <p className="prop-hint">连线规则：Trigger 从 ⚡ 出发；Requirement / Restriction / 必选扩展有「通过 / 未通过」两个出口；Action 是终点。</p>
          </div>
        )}
        <datalist id="maestro-tool-names">
          {KNOWN_TOOLS.map((tool) => <option key={tool} value={tool} />)}
          {known.servers.flatMap((server) => server.tools.map((tool) => `mcp__${server.name}__${tool.name}`)).map((tool) => <option key={tool} value={tool} />)}
        </datalist>
      </aside>
    </div>
  );
}

export default function ArchitectureSandbox(props: SandboxProps) {
  return (
    <ReactFlowProvider>
      <SandboxInner {...props} />
    </ReactFlowProvider>
  );
}
