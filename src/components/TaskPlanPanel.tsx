/// ZCode-style task tracker: progress summary + compact step list. `card` is
/// the collapsible floating panel above the chat; `list` is the full list in
/// the right panel's Tasks view. Pure props — no IPC, no stores.
import { useEffect, useRef, useState } from 'react';
import { CheckCircle2, ChevronDown, ChevronRight, Circle, CircleDot, ListTodo, Loader2 } from 'lucide-react';

import type { PlanStep } from '../api/types';

const STATUS_CLASS: Record<PlanStep['status'], string> = {
  pending: 'pending',
  in_progress: 'in-progress',
  completed: 'completed',
};

function StepIcon({ status, running }: { status: PlanStep['status']; running: boolean }) {
  if (status === 'completed') return <CheckCircle2 size={14} className="task-plan-icon completed" />;
  if (status === 'in_progress') {
    return running
      ? <Loader2 size={14} className="task-plan-icon in-progress spinning" />
      : <CircleDot size={14} className="task-plan-icon in-progress" />;
  }
  return <Circle size={14} className="task-plan-icon pending" />;
}

export default function TaskPlanPanel({ steps, running, variant }: {
  steps: PlanStep[];
  running: boolean;
  variant: 'card' | 'list';
}) {
  const completed = steps.filter((step) => step.status === 'completed').length;
  const allDone = steps.length > 0 && completed === steps.length;
  const [open, setOpen] = useState(!allDone);
  const manual = useRef(false);
  const active = steps.find((step) => step.status === 'in_progress')
    ?? steps.find((step) => step.status === 'pending')
    ?? steps[steps.length - 1];

  // Follow the run state until the user takes over the disclosure.
  useEffect(() => {
    if (manual.current) return;
    setOpen(!allDone);
  }, [allDone]);

  if (!steps.length) return null;

  const progress = Math.round((completed / steps.length) * 100);
  const header = <div className="task-plan-header">
    {allDone ? <CheckCircle2 size={14} className="task-plan-icon completed" /> : <ListTodo size={14} className="task-plan-title-icon" />}
    <span className="task-plan-title">{allDone ? '全部完成' : '任务计划'}</span>
    <span className="task-plan-count">{completed}/{steps.length}</span>
    {variant === 'card' && (open ? <ChevronDown size={13} className="task-plan-chevron" /> : <ChevronRight size={13} className="task-plan-chevron" />)}
  </div>;

  const body = <div className="task-plan-list" role="list" aria-label="任务步骤">
    {steps.map((step) => <div key={step.id} className={`task-plan-step ${STATUS_CLASS[step.status]}`} role="listitem">
      <StepIcon status={step.status} running={running} />
      <span className="task-plan-text">{step.description}</span>
    </div>)}
  </div>;

  return <section className={`task-plan-panel is-${variant}`} aria-label="任务计划">
    <div className="task-plan-progress" aria-hidden="true">
      <span className="task-plan-progress-fill" style={{ width: `${progress}%` }} />
    </div>
    {variant === 'list' ? <>
      {header}
      {body}
    </> : <>
      <button type="button" className="task-plan-toggle" aria-expanded={open}
        onClick={() => { manual.current = true; setOpen((value) => !value); }}>
        {header}
      </button>
      {!open && active && !allDone && <p className="task-plan-current">{active.description}</p>}
      <div hidden={!open}>{body}</div>
    </>}
  </section>;
}
