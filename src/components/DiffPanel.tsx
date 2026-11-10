import * as Tabs from '@radix-ui/react-tabs';
import { Braces, ChevronRight, FileCode2, GitBranch, GitCompareArrows, LockKeyhole, PanelsTopLeft, Rows3 } from 'lucide-react';
import type { PlanCard } from '../api/types';
import { planLabels } from './PlanCard';
import { Choice } from './ui';
import MonacoDiff from './MonacoDiff';

export interface DiffViewState { file: { planId: number; path: string } | null; split: boolean }
interface Props {
  plans: PlanCard[]; selectedId: number | null; onSelect: (id: number) => void;
  view: DiffViewState; onViewChange: (view: DiffViewState) => void; workspaceName?: string;
  fontSize?: number; fontFamily?: string;
}

export default function DiffPanel({ plans, selectedId, onSelect, view, onViewChange, workspaceName = 'Maestro-Agent', fontSize = 12, fontFamily = 'Cascadia Code' }: Props) {
  const { file, split } = view;
  const plan = plans.find((item) => item.id === selectedId) ?? plans.at(-1);
  const diff = plan?.draft.diffs.find((item) => file?.planId === plan.id && item.path === file.path) ?? plan?.draft.diffs[0];
  const fileName = diff?.path.split('/').at(-1);
  return (
    <aside aria-label="Diff 审查面板" className="review-pane">
      <header className="review-header"><div className="flex items-center gap-2"><GitCompareArrows size={16} /><h2>Review changes</h2></div>
        <span className="readonly-label"><LockKeyhole size={11} />Read only</span>
      </header>
      {!plan && <div className="editor-empty"><Braces size={34} strokeWidth={1} /><h3>你的下一次变更，从这里开始</h3><p>Agent 提交计划后，文件差异会出现在这里。</p></div>}
      {plan && <>
        <div className="review-context"><span><GitBranch size={12} />proposed / plan-{plan.id}</span>
          {plans.length > 1 ? <Choice label="选择计划" value={String(plan.id)} onChange={(value) => onSelect(Number(value))}
            options={plans.map((item) => ({ value: String(item.id), label: `#${item.id} ${item.draft.title}` }))} />
            : <span className="muted-text">{planLabels[plan.status]}</span>}
        </div>
        {diff && <Tabs.Root value={diff.path} onValueChange={(path) => onViewChange({ ...view, file: { planId: plan.id, path } })} className="file-tabs-root">
          <Tabs.List aria-label="文件标签" className="file-tabs">
            {plan.draft.diffs.map((item) => <Tabs.Trigger key={item.path} value={item.path} className="file-tab" aria-label={item.path}>
              <FileCode2 size={14} strokeWidth={1.5} />{item.path.split('/').at(-1)}<span className="file-change-dot" />
            </Tabs.Trigger>)}
          </Tabs.List>
          <div className="editor-breadcrumb"><span>{workspaceName}</span><ChevronRight size={11} /><span>{diff.path.split('/').slice(0, -1).join(' / ') || 'src'}</span><ChevronRight size={11} /><span>{fileName}</span></div>
          <div className="diff-toolbar"><span>Original <span className="mx-2 opacity-30">→</span> Proposed</span>
            <Tabs.Root value={split ? 'split' : 'inline'} onValueChange={(value) => onViewChange({ ...view, split: value === 'split' })}>
              <Tabs.List aria-label="Diff 布局" className="segmented-tabs">
                <Tabs.Trigger value="split" aria-label="并排差异"><PanelsTopLeft size={13} /></Tabs.Trigger>
                <Tabs.Trigger value="inline" aria-label="行内差异"><Rows3 size={13} /></Tabs.Trigger>
              </Tabs.List>
            </Tabs.Root>
          </div>
          <MonacoDiff diff={diff} split={split} fontSize={fontSize} fontFamily={fontFamily} />
        </Tabs.Root>}
        {!diff && <div className="editor-empty"><FileCode2 size={28} /><p>此计划没有文件变更提议。</p></div>}
        <footer className="editor-status"><span>提议内容 · 未应用</span><span>{fileName?.match(/\.(h|cpp|hpp)$/) ? 'C++' : 'Text'}<span className="mx-3">UTF-8</span>LF</span></footer>
      </>}
      <div className="review-safety-note">占位提议，不代表工作区真实差异。批准不会执行修改。</div>
    </aside>
  );
}
