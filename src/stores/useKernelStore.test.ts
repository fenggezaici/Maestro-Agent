import { beforeEach, expect, it, vi } from 'vitest';

vi.mock('../api/sessions', () => ({
  sessionsApi: {
    session: vi.fn(async () => ({ session: { id: 1, title: 't', updatedAt: 1, runCount: 0, projectId: 1, archived: false }, runs: [], plans: [] })),
  },
}));

import type { PlanStep } from '../api/types';
import { useKernelStore } from './useKernelStore';

const step = (id: string, description: string, status: PlanStep['status']): PlanStep => ({ id, description, status });

beforeEach(() => {
  useKernelStore.getState().disconnect();
});

it('keeps task plans per session and projects the selected session plan', () => {
  const store = useKernelStore.getState();
  useKernelStore.setState({ selectedSessionId: 1 });
  store.applySideEffects({
    sequence: 2,
    type: 'task_plan_updated',
    runId: 7,
    sessionId: 1,
    steps: [step('1', '分析项目', 'in_progress')],
  });
  expect(useKernelStore.getState().currentPlan.map((item) => item.description)).toEqual(['分析项目']);

  store.applySideEffects({
    sequence: 3,
    type: 'task_plan_updated',
    runId: 8,
    sessionId: 2,
    steps: [step('1', '读取文件', 'completed'), step('2', '重构', 'in_progress')],
  });
  // A plan for another session must not replace the visible projection.
  expect(useKernelStore.getState().currentPlan).toHaveLength(1);
  expect(useKernelStore.getState().taskPlans[2]).toHaveLength(2);

  store.selectSession(2);
  expect(useKernelStore.getState().currentPlan).toHaveLength(2);
  store.selectSession(1);
  expect(useKernelStore.getState().currentPlan).toHaveLength(1);
});
