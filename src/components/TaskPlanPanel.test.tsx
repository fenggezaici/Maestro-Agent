import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it } from 'vitest';

import type { PlanStep } from '../api/types';
import TaskPlanPanel from './TaskPlanPanel';

const steps: PlanStep[] = [
  { id: '1', description: '分析项目', status: 'completed' },
  { id: '2', description: '重构核心文件', status: 'in_progress' },
  { id: '3', description: '补充测试', status: 'pending' },
];

it('renders progress, completed strikethrough and the running spinner', () => {
  const view = render(<TaskPlanPanel steps={steps} running variant="list" />);
  expect(screen.getByText('任务计划')).toBeInTheDocument();
  expect(screen.getByText('1/3')).toBeInTheDocument();

  const completed = view.container.querySelector('.task-plan-step.completed');
  expect(completed).not.toBeNull();
  expect(completed!.querySelector('.task-plan-icon.completed')).not.toBeNull();
  expect(completed!.querySelector('.task-plan-text')!.className).toContain('task-plan-text');

  const active = view.container.querySelector('.task-plan-step.in-progress');
  expect(active!.querySelector('.task-plan-icon.spinning')).not.toBeNull();

  const pending = view.container.querySelector('.task-plan-step.pending');
  expect(pending!.querySelector('.task-plan-icon.pending')).not.toBeNull();
});

it('degrades the in-progress icon when the run is no longer streaming', () => {
  const view = render(<TaskPlanPanel steps={steps} running={false} variant="list" />);
  expect(view.container.querySelector('.task-plan-icon.spinning')).toBeNull();
  expect(view.container.querySelector('.task-plan-step.in-progress .task-plan-icon')).not.toBeNull();
});

it('shows the all-done summary once every step is completed', () => {
  const done = steps.map((step) => ({ ...step, status: 'completed' as const }));
  render(<TaskPlanPanel steps={done} running={false} variant="card" />);
  expect(screen.getByText('全部完成')).toBeInTheDocument();
  expect(screen.getByText('3/3')).toBeInTheDocument();
});

it('renders nothing without steps', () => {
  const { container } = render(<TaskPlanPanel steps={[]} running variant="card" />);
  expect(container).toBeEmptyDOMElement();
});

it('starts the floating card collapsed and expands on click', () => {
  const view = render(<TaskPlanPanel steps={steps} running variant="card" />);
  expect(screen.queryByText('分析项目')).not.toBeVisible();
  fireEvent.click(screen.getByRole('button', { name: /任务计划/ }));
  expect(screen.getByText('分析项目')).toBeVisible();
});
