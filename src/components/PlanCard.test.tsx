import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import PlanCard from './PlanCard';
import type { PlanCard as Plan } from '../api/types';

const pending: Plan = {
  id: 7,
  sessionId: 1,
  runId: 3,
  status: 'pending',
  draft: {
    title: '重构登录流程',
    summary: '拆分会话与鉴权',
    steps: ['抽取会话服务', '迁移调用方'],
    diffs: [{ path: 'src/a.ts', before: 'a', after: 'b' }],
  },
};

const base = { canDecide: true, pending: false, onReview: vi.fn(), onDecision: vi.fn() };

it('shows a pending plan with its steps and decides approve / reject', () => {
  const onDecision = vi.fn();
  render(<PlanCard {...base} plan={pending} onDecision={onDecision} />);

  expect(screen.getByLabelText('计划审批 7')).toBeInTheDocument();
  expect(screen.getByText('待审批')).toBeInTheDocument();
  expect(screen.getByText('重构登录流程')).toBeInTheDocument();
  expect(screen.getByText('抽取会话服务')).toBeInTheDocument();

  fireEvent.click(screen.getByRole('button', { name: '批准计划' }));
  expect(onDecision).toHaveBeenCalledWith(7, 'approve');
  fireEvent.click(screen.getByRole('button', { name: '拒绝计划' }));
  expect(onDecision).toHaveBeenCalledWith(7, 'reject');
});

it('cannot decide before the run finishes, and says why', () => {
  render(<PlanCard {...base} plan={pending} canDecide={false} />);
  expect(screen.getByRole('button', { name: '批准计划' })).toBeDisabled();
  expect(screen.getByRole('button', { name: '拒绝计划' })).toBeDisabled();
  expect(screen.getByText('等待本轮 Agent 完成后才能审批。')).toBeInTheDocument();
});

it('hides the decision buttons once the plan is settled', () => {
  render(<PlanCard {...base} plan={{ ...pending, status: 'approved' }} />);
  expect(screen.queryByRole('button', { name: '批准计划' })).not.toBeInTheDocument();
  expect(screen.getByText('已批准')).toBeInTheDocument();
  // A settled plan starts collapsed; expand it to read the footnote.
  fireEvent.click(screen.getByRole('button', { name: /展开计划 重构登录流程/ }));
  expect(screen.getByText('计划已批准，尚未执行任何修改。')).toBeInTheDocument();
});

it('marks preview decisions as display-only and keeps review reachable', () => {
  const onReview = vi.fn();
  render(<PlanCard {...base} plan={pending} preview onReview={onReview} />);
  expect(screen.getByText('静态预览：审批仅供展示，不执行或应用修改。')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: /审查 Diff/ }));
  expect(onReview).toHaveBeenCalledWith(7);
});

it('collapses and expands from the header', () => {
  render(<PlanCard {...base} plan={pending} />);
  const header = screen.getByRole('button', { name: /折叠计划 重构登录流程/ });
  expect(header).toHaveAttribute('aria-expanded', 'true');
  fireEvent.click(header);
  expect(screen.getByRole('button', { name: /展开计划 重构登录流程/ })).toHaveAttribute('aria-expanded', 'false');
  expect(screen.queryByText('抽取会话服务')).not.toBeInTheDocument();
});
