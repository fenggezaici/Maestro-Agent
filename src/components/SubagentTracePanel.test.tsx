import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import SubagentTracePanel from './SubagentTracePanel';
import type { SubagentView } from '../api/types';

const trace: SubagentView = {
  id: 'sub-1',
  runId: 3,
  sessionId: 1,
  agent: 'explore',
  description: '调研登录流程',
  prompt: '查看登录相关文件',
  status: 'running',
  summary: '',
  error: null,
  content: '已读取 3 个文件',
  reasoning: '',
  toolCount: 2,
  inputTokens: 10,
  outputTokens: 5,
  startedAt: 1,
  finishedAt: null,
  tools: [{ runId: 3, tool: 'read_file', detail: 'src/a.ts', allowed: true }],
};

it('renders the child trace with its status and can stop it', () => {
  const onCancel = vi.fn();
  render(<SubagentTracePanel trace={trace} light={false} reasoningDisplay="summary" onCancel={onCancel} />);

  expect(screen.getByLabelText('子智能体 explore')).toBeInTheDocument();
  expect(screen.getByText('调研登录流程')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: /停止/ }));
  expect(onCancel).toHaveBeenCalledWith('sub-1');
});

it('offers no stop control once the child finished', () => {
  render(<SubagentTracePanel
    trace={{ ...trace, status: 'completed', finishedAt: 2, summary: '结论：会话逻辑在 auth.ts' }}
    light={false} reasoningDisplay="summary" onCancel={vi.fn()} />);
  expect(screen.queryByRole('button', { name: /停止/ })).not.toBeInTheDocument();
});

it('shows provider-reported speed and thinking tokens, never estimates', () => {
  render(<SubagentTracePanel
    trace={{ ...trace, status: 'completed', finishedAt: 2, inputTokens: 21500, outputTokens: 2100,
      reasoningTokens: 800, firstTokenMs: 400, elapsedMs: 4400, lastInputTokens: 21500 }}
    light={false} reasoningDisplay="summary" onCancel={vi.fn()} />);
  expect(screen.getByText('think 800')).toBeInTheDocument();
  // 2100 output tokens over (4400-400)ms decode window.
  expect(screen.getByText('525.0 tok/s')).toBeInTheDocument();
  expect(screen.getByText('TTFT 0.4s')).toBeInTheDocument();
});
