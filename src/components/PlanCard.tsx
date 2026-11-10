import { useState } from 'react';
import type { PlanCard as Plan, PlanDecision } from '../ipc';
import { ArrowUpRight, Check, ChevronDown, FileCheck2, ListChecks, X } from 'lucide-react';

export const planLabels = {
  pending: '待审批', approved: '已批准', rejected: '已拒绝', invalidated: '已失效',
};

interface Props {
  plan: Plan;
  preview?: boolean;
  canDecide: boolean;
  pending: boolean;
  onReview: (id: number) => void;
  onDecision: (id: number, decision: PlanDecision) => void;
}

export default function PlanCard({ plan, canDecide, pending, preview = false, onReview, onDecision }: Props) {
  const [open, setOpen] = useState(plan.status === 'pending');
  return (
    <article aria-label={`计划审批 ${plan.id}`} className="approval-card" data-collapsed={!open}>
      <button type="button" className="approval-heading plan-summary-row" aria-expanded={open}
        aria-label={`${open ? '折叠' : '展开'}计划 ${plan.draft.title}`} onClick={() => setOpen((value) => !value)}>
        <span className="approval-icon"><ListChecks size={17} strokeWidth={1.6} /></span>
        <div className="flex-1"><p className="eyebrow">PROPOSED PLAN</p><h3>{plan.draft.title}</h3></div>
        <span className={`plan-status plan-${plan.status}`}>{planLabels[plan.status]}</span>
        <ChevronDown size={14} className="plan-collapse-icon" />
      </button>
      {open && <>
        <p className="plan-summary">{plan.draft.summary}</p>
        <ol className="plan-steps">
          {plan.draft.steps.map((step, index) => <li key={index}>
            <span className="step-number">{String(index + 1).padStart(2, '0')}</span>
            <div><span className="step-label">Step {index + 1}</span><p>{step}</p></div>
          </li>)}
        </ol>
        <div className="approval-actions">
          <button type="button" className="text-button" onClick={() => onReview(plan.id)}><FileCheck2 size={14} />审查 Diff<ArrowUpRight size={12} /></button>
          <div className="flex-1" />
          {plan.status === 'pending' && <>
            <button type="button" aria-label="拒绝计划" className="button-ghost" disabled={!canDecide || pending}
              onClick={() => onDecision(plan.id, 'reject')}><X size={14} />Reject</button>
            <button type="button" aria-label="批准计划" className="button-approve" disabled={!canDecide || pending}
              onClick={() => onDecision(plan.id, 'approve')}><Check size={14} />Approve</button>
          </>}
        </div>
        <p className="approval-footnote">{preview ? '静态预览：审批仅供展示，不执行或应用修改。' : '审批只记录计划决定，不切换模式或应用修改。'}</p>
        {plan.status === 'pending' && !canDecide && !preview && <p className="approval-footnote">等待本轮 Agent 完成后才能审批。</p>}
        {plan.status === 'approved' && <p className="approval-footnote">计划已批准，尚未执行任何修改。</p>}
        {plan.status === 'invalidated' && <p className="approval-footnote">来源运行已取消、失败或中断，计划不可审批。</p>}
      </>}
    </article>
  );
}
