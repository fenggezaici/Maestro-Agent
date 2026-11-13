import { useEffect, useState } from 'react';
import { ChevronDown, ChevronRight, File, FileCode2, FileJson, FileText, Loader2, RotateCcw } from 'lucide-react';
import type { RunChangeSummary, RunFileChange } from '../api/changes';

function Icon({ path }: { path: string }) {
  const ext = path.split('.').pop()?.toLowerCase() ?? '';
  if (['ts','tsx','js','jsx','py','rs','cpp','cc','c','h','hpp','cs','go','java'].includes(ext)) return <FileCode2 size={14} />;
  if (['json','yaml','yml','toml'].includes(ext)) return <FileJson size={14} />;
  if (['md','txt','log'].includes(ext)) return <FileText size={14} />;
  return <File size={14} />;
}

export default function ChangeSummaryCard({ summary, changes, loading, error, running, onExpand, onReview, onOpen, onRevert }: {
  summary: RunChangeSummary; changes: RunFileChange[]; loading: boolean; error: string | null; running: boolean;
  onExpand: () => void; onReview: (path: string) => void; onOpen: (path: string) => void; onRevert: () => void;
}) {
  const [open, setOpen] = useState(false);
  useEffect(() => { if (open) onExpand(); }, [open, summary.files, summary.additions, summary.deletions, onExpand]);
  const reverted = summary.state === 'reverted';
  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (next) onExpand();
  };
  return <section className="change-summary-card" aria-label="本轮变更文件">
    <div className="change-summary-head">
      <button type="button" className="change-summary-toggle" aria-expanded={open} onClick={toggle}>
        {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        <strong>{summary.files} 个文件已更改</strong>
        <span className="change-add">+{summary.additions}</span><span className="change-del">−{summary.deletions}</span>
        {reverted && <span className="change-reverted">已撤销</span>}
      </button>
      <button type="button" className="change-revert" onClick={onRevert}
        disabled={running || reverted || summary.files === 0}
        title={reverted ? '本轮修改已撤销' : running ? '运行中不可撤销' : '撤销本轮文件修改'}>
        <RotateCcw size={12} />撤销
      </button>
    </div>
    {open && <div className="change-summary-list">
      {loading ? <div className="change-summary-empty"><Loader2 size={13} className="spin" />正在读取变更明细…</div>
        : error ? <div className="change-summary-empty" role="alert">{error}</div>
        : changes.length === 0 ? <div className="change-summary-empty">暂无变更明细</div>
        : changes.map((change) => {
          const slash = Math.max(change.path.lastIndexOf('/'), change.path.lastIndexOf('\\'));
          const dir = slash >= 0 ? change.path.slice(0, slash + 1) : '';
          const name = slash >= 0 ? change.path.slice(slash + 1) : change.path;
          return <div className="change-summary-row" key={change.path}>
            <Icon path={change.path} /><strong>{name}</strong>{dir && <span className="change-dir">{dir}</span>}
            {change.added > 0 && <span className="change-add">+{change.added}</span>}
            {change.removed > 0 && <span className="change-del">−{change.removed}</span>}
            <span className="change-actions"><button type="button" className="button-ghost" onClick={() => onReview(change.path)}>审查</button><button type="button" className="button-ghost" onClick={() => onOpen(change.path)}>打开</button></span>
          </div>;
        })}
    </div>}
  </section>;
}
