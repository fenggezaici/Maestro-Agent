import { useEffect, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
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

/// Inline card for `ask_user`, docked directly above the composer (ZCode
/// ElicitationDialog parity — not a modal). Questions are presented one at a
/// time; single-choice taps auto-advance, and the run stays suspended until
/// Submit/Skip (or external cancellation).
export default function UserQuestionDialog({ request, pending, onSubmit }: {
  request: PendingQuestionRequest | null;
  pending: boolean;
  onSubmit: (answers: QuestionAnswer[]) => void;
}) {
  const [questionIndex, setQuestionIndex] = useState(0);
  const [selected, setSelected] = useState<Record<string, string[]>>({});
  const [customOn, setCustomOn] = useState<Record<string, boolean>>({});
  const [custom, setCustom] = useState<Record<string, string>>({});

  useEffect(() => {
    setQuestionIndex(0);
    setSelected({});
    setCustomOn({});
    setCustom({});
  }, [request?.requestId]);

  if (!request) return null;
  const total = request.questions.length;
  const index = Math.min(questionIndex, total - 1);
  const question = request.questions[index];
  const isLast = index >= total - 1;
  const answered = (item: UserQuestion) =>
    (selected[item.id]?.length ?? 0) > 0 ||
    (customOn[item.id] && (custom[item.id] ?? '').trim().length > 0);
  const complete = request.questions.every(answered);

  const submit = () => onSubmit(request.questions.map((item) => ({
    questionId: item.id,
    selected: customOn[item.id] ? [] : (selected[item.id] ?? []),
    custom: customOn[item.id] ? ((custom[item.id] ?? '').trim() || null) : null,
  })));

  const advance = () => {
    if (isLast) {
      if (complete) submit();
      return;
    }
    setQuestionIndex((value) => Math.min(value + 1, total - 1));
  };

  const pick = (optionLabel: string) => {
    setCustomOn((value) => ({ ...value, [question.id]: false }));
    setSelected((value) => ({
      ...value,
      [question.id]: question.multiSelect ? toggle(value[question.id], optionLabel) : [optionLabel],
    }));
    // ZCode: a single-choice tap is the answer — move on without a second click.
    if (!question.multiSelect && !isLast) setQuestionIndex((value) => Math.min(value + 1, total - 1));
  };

  const onCustomKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter' && (answered(question) || complete)) { event.preventDefault(); advance(); }
  };

  return <div className="user-question-card" role="dialog" aria-label="需要你的选择">
    <div className="user-question-card-head">
      {question.header && <span className="user-question-header-badge">{question.header}</span>}
      {total > 1 && <span className="user-question-progress">问题 {index + 1} / {total}</span>}
    </div>
    <p className="user-question-text">{question.question}</p>
    <div className="user-question-options">
      {question.options.map((option) => {
        const active = !customOn[question.id] && (selected[question.id] ?? []).includes(option.label);
        return <button type="button" key={option.label} aria-pressed={active}
          className={`user-question-option${active ? ' active' : ''}`}
          onClick={() => pick(option.label)}>
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
        placeholder="输入你的答案" onKeyDown={onCustomKeyDown}
        onChange={(event) => setCustom((value) => ({ ...value, [question.id]: event.target.value }))} />}
    </div>
    <div className="user-question-card-foot">
      <button type="button" className="button-ghost" disabled={pending} onClick={() => onSubmit([])}>跳过</button>
      <span className="user-question-spacer" />
      {index > 0 && <button type="button" className="button-ghost" disabled={pending}
        onClick={() => setQuestionIndex((value) => Math.max(value - 1, 0))}>上一题</button>}
      {isLast
        ? <button type="button" className="button-approve" disabled={pending || !complete} onClick={submit}>提交</button>
        : <button type="button" className="button-approve" disabled={pending} onClick={advance}>下一题</button>}
    </div>
  </div>;
}
