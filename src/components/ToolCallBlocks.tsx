import { ChevronDown, ChevronRight, Database, FileEdit, FileText, Folder, Puzzle, SquareTerminal, Wrench, type LucideIcon } from 'lucide-react';
import { useState } from 'react';
import type { ToolCallView } from '../api/types';
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
  read_session: { icon: Database, verb: '读取会话' },
  list_sessions: { icon: Database, verb: '列出会话' },
};

function meta(tool: string): { icon: LucideIcon; verb: string; short: string } {
  const known = TOOL_META[tool];
  if (known) return { ...known, short: tool };
  const short = tool.startsWith('mcp__') ? tool.slice(5).replace(/__/g, ' · ') : tool;
  return { icon: tool.startsWith('mcp__') ? Puzzle : Wrench, verb: '调用', short };
}

/// Row target: path basename bold, directory prefix gray; plain text otherwise.
function Target({ text }: { text: string }) {
  if (!text) return null;
  const slash = Math.max(text.lastIndexOf('/'), text.lastIndexOf('\\'));
  const dir = slash >= 0 ? text.slice(0, slash + 1) : '';
  const name = slash >= 0 ? text.slice(slash + 1) : text;
  return <span className="tool-call-target">{dir && <span className="tool-call-dir">{dir}</span>}<strong>{name}</strong></span>;
}

function targetOf(call: ToolCallView): string {
  if (call.target) return call.target;
  const segments = call.detail.split(' · ');
  return segments.length > 1 ? segments[segments.length - 1] : '';
}

/// One collapsed tool-call row with icon + verb + target + status; expandable
/// detail panel for results/output/failures. Exported so the reasoning timeline
/// can interleave rows with thinking nodes in event order.
export function ToolCallRow({ call, onSelectFile }: { call: ToolCallView; onSelectFile?: (path: string) => void }) {
  const [expanded, setExpanded] = useState(false);
  const status = call.awaitingApproval ? '待批准' : call.pending ? '执行中' : call.allowed ? '已执行' : '已阻止';
  const statusClass = call.awaitingApproval || call.pending ? 'pending' : call.allowed ? 'ok' : 'blocked';
  const { icon: Icon, verb, short } = meta(call.tool);
  const target = targetOf(call);
  const writeResult = call.allowed ? writeResultOf(call.detail) : null;
  const delta = writeResult ? countChanges(writeResult.before, writeResult.after) : null;
  const panel = call.allowed ? (writeResult ? writeResult.path : (call.detail || '读取成功')) : `失败：${call.detail || '未知原因'}`;
  return <div className={`tool-call-row ${call.allowed ? 'allowed' : 'blocked'}`} role="listitem">
    <button type="button" className="tool-call-summary" aria-expanded={expanded}
      onClick={() => {
        setExpanded((value) => !value);
        const writePath = writeResult?.path ?? (call.tool === 'write_file' || call.tool === 'apply_patch' ? target : undefined);
        if (writePath) onSelectFile?.(writePath);
      }}>
      {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
      <Icon size={12} className={call.pending || call.awaitingApproval ? 'tool-call-pending-icon' : call.allowed ? 'tool-call-ok-icon' : 'tool-call-fail-icon'} />
      <span className="tool-call-verb">{verb}</span>
      <Target text={target || short} />
      {delta && <span className="tool-call-delta"><em className="add">+{delta.added}</em><em className="del">−{delta.removed}</em></span>}
      <span className={`tool-call-status ${statusClass}`}>{status}</span>
    </button>
    {expanded && !call.awaitingApproval && (writeResult
      ? <MiniDiff before={writeResult.before} after={writeResult.after} />
      : <pre className="tool-call-panel">{panel}</pre>)}
  </div>;
}

/// ZCode-style collapsed tool-call group: one row per call.
export default function ToolCallBlocks({ calls, onSelectFile }: { calls: ToolCallView[]; onSelectFile?: (path: string) => void }) {
  if (!calls.length) return null;
  return <div className="tool-events" role="list" aria-label="工具调用明细">
    {calls.map((call, index) => <ToolCallRow key={index} call={call} onSelectFile={onSelectFile} />)}
  </div>;
}
