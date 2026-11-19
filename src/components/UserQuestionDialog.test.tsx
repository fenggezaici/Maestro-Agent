import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import UserQuestionDialog from './UserQuestionDialog';
import type { PendingQuestionRequest } from './UserQuestionDialog';

const request: PendingQuestionRequest = {
  runId: 3,
  requestId: 'req-1',
  questions: [{
    id: 'q1',
    question: '用哪个模式继续？',
    options: [{ label: '计划' }, { label: '代码' }],
  }],
};

it('requires an answer before submitting and reports the structured selection', () => {
  const onSubmit = vi.fn();
  render(<UserQuestionDialog request={request} pending={false} onSubmit={onSubmit} />);

  expect(screen.getByRole('button', { name: '提交' })).toBeDisabled();
  fireEvent.click(screen.getByText('计划'));
  fireEvent.click(screen.getByRole('button', { name: '提交' }));

  expect(onSubmit).toHaveBeenCalledTimes(1);
  expect(onSubmit.mock.calls[0][0]).toEqual([{ questionId: 'q1', selected: ['计划'], custom: null }]);
});

it('can be skipped so the model decides on its own', () => {
  const onSubmit = vi.fn();
  render(<UserQuestionDialog request={request} pending={false} onSubmit={onSubmit} />);
  fireEvent.click(screen.getByRole('button', { name: '跳过' }));
  expect(onSubmit).toHaveBeenCalledWith([]);
});

it('disables both actions while a submission is pending', () => {
  render(<UserQuestionDialog request={request} pending onSubmit={vi.fn()} />);
  expect(screen.getByRole('button', { name: '跳过' })).toBeDisabled();
  expect(screen.getByRole('button', { name: '提交' })).toBeDisabled();
});

const twoQuestions: PendingQuestionRequest = {
  runId: 4,
  requestId: 'req-2',
  questions: [
    { id: 'q1', question: '目标是什么？', options: [{ label: '修 bug' }, { label: '写测试' }] },
    { id: 'q2', question: '怎么验证？', options: [{ label: '跑测试' }], multiSelect: true },
  ],
};

it('steps through questions one at a time with auto-advance and back', () => {
  const onSubmit = vi.fn();
  render(<UserQuestionDialog request={twoQuestions} pending={false} onSubmit={onSubmit} />);

  // Only the first question is visible; progress shows 1/2.
  expect(screen.getByText('目标是什么？')).toBeInTheDocument();
  expect(screen.queryByText('怎么验证？')).not.toBeInTheDocument();
  expect(screen.getByText('问题 1 / 2')).toBeInTheDocument();

  // Single-choice tap auto-advances to the next question.
  fireEvent.click(screen.getByText('修 bug'));
  expect(screen.getByText('怎么验证？')).toBeInTheDocument();
  expect(screen.getByText('问题 2 / 2')).toBeInTheDocument();

  // Multi-select stays put; 提交 waits for the answer.
  fireEvent.click(screen.getByText('跑测试'));
  fireEvent.click(screen.getByRole('button', { name: '上一题' }));
  expect(screen.getByText('目标是什么？')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: '下一题' }));
  fireEvent.click(screen.getByRole('button', { name: '提交' }));

  expect(onSubmit).toHaveBeenCalledWith([
    { questionId: 'q1', selected: ['修 bug'], custom: null },
    { questionId: 'q2', selected: ['跑测试'], custom: null },
  ]);
});
