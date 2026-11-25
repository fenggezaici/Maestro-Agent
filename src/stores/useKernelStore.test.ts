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

it('bumps extensionsTick on extensions_changed', () => {
  const before = useKernelStore.getState().extensionsTick;
  useKernelStore.getState().applySideEffects({ type: 'extensions_changed', sequence: 99 } as never);
  expect(useKernelStore.getState().extensionsTick).toBe(before + 1);
});

it('marks a subagent cancelled when the finish event says so', () => {
  const trace = { id: 'sa-1', runId: 1, sessionId: 1, agent: 'explore', description: 'd', prompt: 'p',
    status: 'running', summary: '', error: null, content: '', reasoning: '', toolCount: 0,
    startedAt: 1, finishedAt: null, tools: [], timeline: [] } as never;
  useKernelStore.setState({ subagents: { 'sa-1': trace } });
  useKernelStore.getState().applySideEffects({
    type: 'subagent_finished', runId: 1, subagentId: 'sa-1', status: 'cancelled',
    summary: '', error: null, toolCount: 0, inputTokens: null, outputTokens: null, finishedAt: 2,
  } as never);
  expect((useKernelStore.getState().subagents['sa-1'] as { status: string }).status).toBe('cancelled');
});
