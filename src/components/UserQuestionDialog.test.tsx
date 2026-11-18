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
