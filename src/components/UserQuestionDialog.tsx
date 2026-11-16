import { useEffect, useState } from 'react';
import { Dialog, DialogContent, DialogTitle } from './ui/dialog';
import type { QuestionAnswer, UserQuestion } from '../api/types';

export interface PendingQuestionRequest {
  runId: number;
  requestId: string;
  questions: UserQuestion[];
}

function toggle(list: string[] | undefined, value: string) {
  const current = list ?? [];
  return current.includes(value) ? current.filter((item) => item !== value) : [...current, value];
}

/// Modal for `ask_user`: options plus an explicit custom-answer field. The run
/// stays suspended until Submit/Skip (or cancellation from outside).
export default function UserQuestionDialog({ request, pending, onSubmit }: {
  request: PendingQuestionRequest | null;
  pending: boolean;
  onSubmit: (answers: QuestionAnswer[]) => void;
}) {
  const [selected, setSelected] = useState<Record<string, string[]>>({});
  const [customOn, setCustomOn] = useState<Record<string, boolean>>({});
  const [custom, setCustom] = useState<Record<string, string>>({});

  useEffect(() => {
    setSelected({});
    setCustomOn({});
    setCustom({});
  }, [request?.requestId]);

  if (!request) return null;
  const complete = request.questions.every((question) =>
    (selected[question.id]?.length ?? 0) > 0 ||
    (customOn[question.id] && (custom[question.id] ?? '').trim().length > 0));

  const submit = () => onSubmit(request.questions.map((question) => ({
    questionId: question.id,
    selected: customOn[question.id] ? [] : (selected[question.id] ?? []),
    custom: customOn[question.id] ? ((custom[question.id] ?? '').trim() || null) : null,
  })));

  return <Dialog open>
    <DialogContent variant="form" className="user-question-dialog" showClose={false}
      onInteractOutside={(event) => event.preventDefault()}
      onEscapeKeyDown={(event) => event.preventDefault()}>
      <DialogTitle asChild><h3>需要你的选择</h3></DialogTitle>
      <p className="dialog-hint">回答后任务会继续执行；也可以跳过，让模型自行决定。</p>
      <div className="user-question-list">
        {request.questions.map((question) => <section key={question.id} className="user-question-item">
          {question.header && <p className="user-question-header">{question.header}</p>}
          <p className="user-question-text">{question.question}</p>
          <div className="user-question-options">
            {question.options.map((option) => {
              const active = !customOn[question.id] && (selected[question.id] ?? []).includes(option.label);
              return <button type="button" key={option.label} aria-pressed={active}
                className={`user-question-option${active ? ' active' : ''}`}
                onClick={() => {
                  setCustomOn((value) => ({ ...value, [question.id]: false }));
                  setSelected((value) => ({
                    ...value,
                    [question.id]: question.multiSelect ? toggle(value[question.id], option.label) : [option.label],
                  }));
                }}>
                <strong>{option.label}</strong>
                {option.description && <span>{option.description}</span>}
              </button>;
            })}
            <button type="button" aria-pressed={Boolean(customOn[question.id])}
              className={`user-question-option custom${customOn[question.id] ? ' active' : ''}`}
              onClick={() => {
                setCustomOn((value) => ({ ...value, [question.id]: !value[question.id] }));
                setSelected((value) => ({ ...value, [question.id]: [] }));
              }}><strong>其他（自定义）</strong></button>
            {customOn[question.id] && <input aria-label={`${question.question} 自定义回答`} value={custom[question.id] ?? ''}
              placeholder="输入你的答案" onChange={(event) => setCustom((value) => ({ ...value, [question.id]: event.target.value }))} />}
          </div>
        </section>)}
      </div>
      <div className="dialog-actions">
        <button type="button" className="button-ghost" disabled={pending} onClick={() => onSubmit([])}>跳过</button>
        <button type="button" className="button-approve" disabled={pending || !complete} onClick={submit}>提交</button>
      </div>
    </DialogContent>
  </Dialog>;
}
