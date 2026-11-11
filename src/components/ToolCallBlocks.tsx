import { Ban, Check, ChevronDown, ChevronRight, Clock, ShieldAlert, SquareTerminal, Wrench, X } from 'lucide-react';
import { useState } from 'react';
import CollapsibleBlock from './CollapsibleBlock';
import type { ToolCallView } from '../api/types';
import { parseApprovalPreview } from '../lib/conversation';

/// Suspended call surfaced as an approval card (ZCode confirmation-style):
/// tool name + what it would do (write preview or the exact command line),
/// then Reject / Approve actions resolved by the Rust kernel.
function ApprovalCard({ call, onResolve }: { call: ToolCallView; onResolve: (runId: number, approved: boolean) => void }) {
  const preview = parseApprovalPreview(call.detail);
  return <div className="tool-approval" role="alert">
    <div className="tool-approval-head"><ShieldAlert size={13} /><strong>{call.tool}</strong><span className="tool-approval-badge">等待批准</span></div>
    {preview?.kind === 'write' ? <>
      <p className="tool-approval-path"><code>{preview.path}</code>{preview.exists ? ' · 覆盖现有文件' : ' · 新建文件'}</p>
      {preview.exists && !!preview.old && <details className="tool-approval-old"><summary>原内容</summary><pre>{preview.old}</pre></details>}
      <pre className="tool-approval-preview">{preview.new || '（空内容）'}</pre>
    </> : preview?.kind === 'command' ? <div className="tool-approval-command"><SquareTerminal size={13} /><code>$ {preview.command}</code></div>
      : preview?.kind === 'args' ? <pre className="tool-approval-preview">{preview.arguments || '（无参数）'}</pre>
      : <p className="tool-approval-path">{call.detail || '（无预览）'}</p>}
    <div className="approval-actions">
      <button type="button" aria-label="拒绝工具调用" className="button-ghost" onClick={() => onResolve(call.runId, false)}><X size={13} />Reject</button>
      <button type="button" aria-label="批准工具调用" className="button-approve" onClick={() => onResolve(call.runId, true)}><Check size={13} />Approve</button>
    </div>
  </div>;
}

/// ZCode-style collapsed tool-call group: ONE summary row per run
/// (`🛠 工具调用 · N (name1、name2…)`) that expands into per-call rows —
/// each row collapses again to show its recorded detail. ✓/✗ status icons.
export default function ToolCallBlocks({ calls, onResolveApproval }: { calls: ToolCallView[]; onResolveApproval?: (runId: number, approved: boolean) => void }) {
  const [openRows, setOpenRows] = useState<Set<number>>(new Set());
  if (!calls.length) return null;
  const blocked = calls.filter((call) => !call.allowed).length;
  const names = [...new Set(calls.map((call) => call.tool))].slice(0, 4).join('、');
  return <CollapsibleBlock
    title={`工具调用 · ${calls.length}`}
    icon={<Wrench size={12} />}
    subtitle={<span className="tool-call-names">{names}</span>}
    contentClass="tool-call-blocks"
    open={calls.some((call) => call.awaitingApproval) || undefined}>
    <div role="list" aria-label="工具调用明细">
      {blocked > 0 && <p className="tool-call-blocked-note"><Ban size={11} />{blocked} 次被阻止</p>}
      {calls.map((call, index) => {
        const expanded = openRows.has(index);
        const status = call.awaitingApproval ? '待批准' : call.pending ? '执行中' : call.allowed ? '已执行' : '已阻止';
        const statusClass = call.awaitingApproval || call.pending ? 'pending' : call.allowed ? 'ok' : 'blocked';
        return <div key={index} className={`tool-call-row ${call.allowed ? 'allowed' : 'blocked'}`} role="listitem">
          {call.awaitingApproval && onResolveApproval ? <ApprovalCard call={call} onResolve={onResolveApproval} /> : <>
            <button type="button" className="tool-call-summary" aria-expanded={expanded}
              onClick={() => setOpenRows((value) => { const next = new Set(value); next.has(index) ? next.delete(index) : next.add(index); return next; })}>
              {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
              {call.awaitingApproval ? <ShieldAlert size={12} className="tool-call-pending-icon" /> : call.pending ? <Clock size={12} className="tool-call-pending-icon" /> : call.allowed ? <Check size={12} className="tool-call-ok-icon" /> : <X size={12} className="tool-call-fail-icon" />}
              <span className="tool-call-name">{call.tool}</span>
              <span className={`tool-call-status ${statusClass}`}>{status}</span>
            </button>
            {expanded && !call.awaitingApproval && <p className={`tool-call-detail ${call.allowed ? '' : 'failed'}`}>{call.allowed ? (call.detail || '（无记录）') : `失败：${call.detail || '（无记录）'}`}</p>}
          </>}
        </div>;
      })}
    </div>
  </CollapsibleBlock>;
}
