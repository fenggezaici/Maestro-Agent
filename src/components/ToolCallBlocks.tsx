import { Ban, Check, ChevronDown, ChevronRight, Wrench, X } from 'lucide-react';
import { useState } from 'react';
import CollapsibleBlock from './CollapsibleBlock';
import type { ToolCallView } from '../api/types';

/// ZCode-style collapsed tool-call group: ONE summary row per run
/// (`🛠 工具调用 · N (name1、name2…)`) that expands into per-call rows —
/// each row collapses again to show its recorded detail. ✓/✗ status icons.
export default function ToolCallBlocks({ calls }: { calls: ToolCallView[] }) {
  const [openRows, setOpenRows] = useState<Set<number>>(new Set());
  if (!calls.length) return null;
  const blocked = calls.filter((call) => !call.allowed).length;
  const names = [...new Set(calls.map((call) => call.tool))].slice(0, 4).join('、');
  return <CollapsibleBlock
    title={`工具调用 · ${calls.length}`}
    icon={<Wrench size={12} />}
    subtitle={<span className="tool-call-names">{names}</span>}
    contentClass="tool-call-blocks"
    defaultOpen={false}>
    <div role="list" aria-label="工具调用明细">
      {blocked > 0 && <p className="tool-call-blocked-note"><Ban size={11} />{blocked} 次被阻止</p>}
      {calls.map((call, index) => {
        const expanded = openRows.has(index);
        return <div key={index} className={`tool-call-row ${call.allowed ? 'allowed' : 'blocked'}`} role="listitem">
          <button type="button" className="tool-call-summary" aria-expanded={expanded}
            onClick={() => setOpenRows((value) => { const next = new Set(value); next.has(index) ? next.delete(index) : next.add(index); return next; })}>
            {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
            {call.allowed ? <Check size={12} className="tool-call-ok-icon" /> : <X size={12} className="tool-call-fail-icon" />}
            <span className="tool-call-name">{call.tool}</span>
            <span className={`tool-call-status ${call.allowed ? 'ok' : 'blocked'}`}>{call.allowed ? '已执行' : '已阻止'}</span>
          </button>
          {expanded && <p className={`tool-call-detail ${call.allowed ? '' : 'failed'}`}>{call.allowed ? (call.detail || '（无记录）') : `失败：${call.detail || '（无记录）'}`}</p>}
        </div>;
      })}
    </div>
  </CollapsibleBlock>;
}
