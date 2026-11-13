import { Bot, ChevronDown, ChevronRight, Database, File, FileCode2, FileEdit, FileJson, FileText, Folder, ListTodo, Loader2, Puzzle, SquareTerminal, Wrench, type LucideIcon } from 'lucide-react';
import { useState } from 'react';
import type { ToolCallView } from '../api/types';
import { parseApprovalPreview } from '../lib/conversation';
import MiniDiff, { countChanges } from './MiniDiff';

/// `write_result` payload emitted in the Finished detail for write tools —
/// carries the before/after contents for the mini diff.
interface WriteResult { kind: 'write_result'; path: string; before: string; after: string }

function writeResultOf(detail: string): WriteResult | null {
  try {
    const value = JSON.parse(detail) as Partial<WriteResult>;
    return value.kind === 'write_result' && typeof value.before === 'string' && typeof value.after === 'string'
      ? (value as WriteResult)
      : null;
  } catch {
    return null;
  }
}

/// Per-tool presentation: lucide icon + Chinese action verb, ZCode-style.
const TOOL_META: Record<string, { icon: LucideIcon; verb: string }> = {
  write_file: { icon: FileEdit, verb: '写入' },
  apply_patch: { icon: FileEdit, verb: '修改' },
  read_file: { icon: FileText, verb: '读取' },
  list_dir: { icon: Folder, verb: '查阅' },
  run_command: { icon: SquareTerminal, verb: '执行' },
  load_skill: { icon: Puzzle, verb: '加载技能' },
  update_plan: { icon: ListTodo, verb: '更新计划' },
  spawn_subagent: { icon: Bot, verb: '子智能体' },
  read_session: { icon: Database, verb: '读取会话' },
  list_sessions: { icon: Database, verb: '列出会话' },
};

function meta(tool: string): { icon: LucideIcon; verb: string; short: string } {
  const known = TOOL_META[tool];
  if (known) return { ...known, short: tool };
  const short = tool.startsWith('mcp__') ? tool.slice(5).replace(/__/g, ' · ') : tool;
  return { icon: tool.startsWith('mcp__') ? Puzzle : Wrench, verb: '调用', short };
}

function FileTypeIcon({ name }: { name: string }) {
  const ext = name.split('.').pop()?.toLowerCase() ?? '';
  if (['ts','tsx','js','jsx','rs','py','cpp','cc','c','h','hpp','cs','go','java'].includes(ext)) return <FileCode2 size={12} />;
  if (['json','toml','yaml','yml'].includes(ext)) return <FileJson size={12} />;
  if (['md','txt','log','csv'].includes(ext)) return <FileText size={12} />;
  return <File size={12} />;
}

/// Row target: directory prefix gray, filename a separate clickable link.
function Target({ text, clickable, onOpen }: { text: string; clickable: boolean; onOpen: () => void }) {
  if (!text) return null;
  const slash = Math.max(text.lastIndexOf('/'), text.lastIndexOf('\\'));
  const dir = slash >= 0 ? text.slice(0, slash + 1) : '';
  const name = slash >= 0 ? text.slice(slash + 1) : text;
  return <span className="tool-call-target">{dir && <span className="tool-call-dir">{dir}</span>}
    <FileTypeIcon name={name} />
    {clickable
      ? <button type="button" className="tool-call-file-link" onClick={(event) => { event.stopPropagation(); onOpen(); }}>{name}</button>
      : <strong>{name}</strong>}
  </span>;
}

function targetOf(call: ToolCallView): string {
  if (call.target) return call.target;
  const segments = call.detail.split(' · ');
  return segments.length > 1 ? segments[segments.length - 1] : '';
}

/// One collapsed tool-call row with icon + verb + target + status; expandable
/// detail panel for results/output/failures. Exported so the reasoning timeline
/// can interleave rows with thinking nodes in event order.
export function ToolCallRow({ call, onSelectFile, onOpenFile, onResolveApproval }: { call: ToolCallView; onSelectFile?: (path: string) => void; onOpenFile?: (path: string) => void; onResolveApproval?: (runId: number, approved: boolean) => void }) {
  const [expanded, setExpanded] = useState(false);
  const pending = Boolean(call.pending || call.awaitingApproval);
  const status = call.awaitingApproval ? '待批准' : call.pending ? '执行中' : call.allowed ? '已执行' : '已阻止';
  const statusClass = call.awaitingApproval || call.pending ? 'pending' : call.allowed ? 'ok' : 'blocked';
  const { icon: Icon, verb, short } = meta(call.tool);
  const target = targetOf(call);
  const writeResult = call.allowed ? writeResultOf(call.detail) : null;
  const delta = writeResult ? countChanges(writeResult.before, writeResult.after) : null;
  const panel = call.allowed ? (writeResult ? writeResult.path : (call.detail || '读取成功')) : `失败：${call.detail || '未知原因'}`;
  const openFile = () => {
    if ((call.tool === 'read_file' || call.tool === 'list_dir') && target) onOpenFile?.(target);
    const writePath = writeResult?.path ?? (call.tool === 'write_file' || call.tool === 'apply_patch' ? target : undefined);
    if (writePath) onSelectFile?.(writePath);
  };
  return <div className={`tool-call-row ${call.allowed ? 'allowed' : 'blocked'}${pending ? ' pending' : ''}`} role="listitem">
    <div role="button" tabIndex={0} className="tool-call-summary" aria-expanded={expanded}
      onClick={() => setExpanded((value) => !value)}
      onKeyDown={(event) => { if (event.target !== event.currentTarget) return; if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); setExpanded((value) => !value); } }}>
      {pending ? <Loader2 size={12} className="tool-call-spinner" /> : <Icon size={12} className={call.allowed ? 'tool-call-ok-icon' : 'tool-call-fail-icon'} />}
      <span className="tool-call-verb">{verb}</span>
      <span className="tool-call-target-link"><Target text={target || short} clickable={Boolean(target) && call.tool !== 'run_command'} onOpen={openFile} /></span>
      {delta && <span className="tool-call-delta"><em className="add">+{delta.added}</em><em className="del">−{delta.removed}</em></span>}
      {expanded ? <ChevronDown size={12} className="tool-call-chevron" /> : <ChevronRight size={12} className="tool-call-chevron" />}
      <span className="tool-call-spacer" />
      {(!call.allowed || pending) && <span className={`tool-call-status ${statusClass}`}>{status}</span>}
    </div>
    {call.awaitingApproval && onResolveApproval && (() => {
      const preview = parseApprovalPreview(call.detail);
      return <div className="tool-approval-inline" role="alert">
        {preview?.kind === 'command' ? <pre className="tool-approval-command">{preview.command}</pre>
          : preview?.kind === 'write' ? <MiniDiff before={preview.old} after={preview.new} />
          : <pre className="tool-approval-args">{preview?.kind === 'args' ? preview.arguments : call.detail}</pre>}
        <div className="tool-approval-actions">
          <button type="button" className="button-ghost" aria-label="拒绝工具调用" onClick={() => onResolveApproval(call.runId, false)}>Reject</button>
          <button type="button" className="button-approve" aria-label="批准工具调用" onClick={() => onResolveApproval(call.runId, true)}>Approve</button>
        </div>
      </div>;
    })()}
    {expanded && !call.awaitingApproval && (writeResult
      ? <MiniDiff before={writeResult.before} after={writeResult.after} />
      : <pre className="tool-call-panel">{panel}</pre>)}
  </div>;
}

/// ZCode-style collapsed tool-call group: one row per call.
export default function ToolCallBlocks({ calls, onSelectFile, onOpenFile }: { calls: ToolCallView[]; onSelectFile?: (path: string) => void; onOpenFile?: (path: string) => void }) {
  if (!calls.length) return null;
  return <div className="tool-events" role="list" aria-label="工具调用明细">
    {calls.map((call, index) => <ToolCallRow key={index} call={call} onSelectFile={onSelectFile} onOpenFile={onOpenFile} />)}
  </div>;
}
