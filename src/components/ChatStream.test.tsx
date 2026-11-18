import { createRef } from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';

import type { PersistedPart, RunStatus, RunView, ToolCallView } from '../api/types';
import type { TimelineItem } from '../lib/conversation';
import ChatStream from './ChatStream';

function run(status: RunStatus, output = ''): RunView {
  return { id: 1, sessionId: 1, prompt: '分析此项目', mode: 'plan', status, output, error: null };
}

function reasoning(text: string, startedAt = 0, endedAt?: number): TimelineItem {
  return { kind: 'reasoning', segment: { text, startedAt, ...(endedAt === undefined ? {} : { endedAt }) } };
}

const base = {
  historyRef: createRef<HTMLDivElement>(),
  onScroll: () => {},
  runs: [run('running')],
  loadingSession: false,
  detailTools: [],
  liveTools: {},
  plans: [],
  reasoningDisplay: 'collapsed' as const,
  theme: 'dark',
  preview: false,
  messageAttachments: {},
  pending: false,
  onCopyError: () => {},
  onReviewPlan: () => {},
  onDecidePlan: () => {},
};

it('pins the active reasoning pane while streaming, pauses on scroll up, and resumes at the bottom', () => {
  const view = render(<ChatStream {...base} liveTimeline={{ 1: [reasoning('first')] }} />);
  const pane = view.container.querySelector<HTMLParagraphElement>('.reasoning-content p');
  expect(pane).not.toBeNull();
  let top = 0;
  Object.defineProperty(pane!, 'scrollTop', { configurable: true, get: () => top, set: (value: number) => { top = value; } });
  Object.defineProperty(pane!, 'scrollHeight', { configurable: true, value: 400 });
  Object.defineProperty(pane!, 'clientHeight', { configurable: true, value: 200 });

  view.rerender(<ChatStream {...base} liveTimeline={{ 1: [reasoning('first second')] }} />);
  expect(top).toBe(400);

  top = 0;
  fireEvent.scroll(pane!);
  view.rerender(<ChatStream {...base} liveTimeline={{ 1: [reasoning('first second third')] }} />);
  expect(top).toBe(0);

  top = 200;
  fireEvent.scroll(pane!);
  view.rerender(<ChatStream {...base} liveTimeline={{ 1: [reasoning('first second third fourth')] }} />);
  expect(top).toBe(400);
});

it('interleaves sealed reasoning nodes with tool rows and keeps the active burst open', () => {
  const calls: ToolCallView[] = [{ runId: 1, tool: 'read_file', detail: 'src/index.ts', allowed: true, target: 'src/index.ts' }];
  const timeline: TimelineItem[] = [
    reasoning('planning', 0, 3000),
    { kind: 'tool', index: 0 },
    reasoning('follow-up', 4000),
  ];
  render(<ChatStream {...base} liveTools={{ 1: calls }} liveTimeline={{ 1: timeline }} />);
  fireEvent.click(screen.getByRole('button', { name: /思考 3 秒/ }));
  expect(screen.getByText('思考 · 3 秒')).toBeInTheDocument();
  expect(screen.getByText('思考 · 进行中…')).toBeInTheDocument();
  expect(screen.getByText('读取')).toBeInTheDocument();
  expect(screen.getByText('index.ts')).toBeInTheDocument();

  const tool = screen.getByText('读取').closest('.tool-call-summary') as HTMLElement;
  expect(tool).toHaveAttribute('aria-expanded', 'false');
  const target = tool.querySelector('.tool-call-target-link');
  expect(target?.nextElementSibling?.classList.contains('tool-call-chevron')).toBe(true);
  expect(target?.nextElementSibling?.nextElementSibling?.classList.contains('tool-call-spacer')).toBe(true);
  fireEvent.click(tool);
  expect(tool).toHaveAttribute('aria-expanded', 'true');
  expect(document.querySelector('.tool-call-panel')).not.toBeNull();

  const reasoningToggle = screen.getByText('思考 · 3 秒').closest('.reasoning-toggle') as HTMLElement;
  expect(reasoningToggle).toHaveAttribute('aria-expanded', 'false');
  fireEvent.click(reasoningToggle);
  expect(reasoningToggle).toHaveAttribute('aria-expanded', 'true');
});

it('no longer renders the assistant meta row or the message footnote', () => {
  render(<ChatStream {...base} runs={[run('completed', 'answer')]} />);
  expect(screen.queryByText('Maestro')).toBeNull();
  expect(screen.queryByText('正在生成')).toBeNull();
  expect(screen.queryByText('模型回复 · 不执行工具或应用修改')).toBeNull();
  expect(screen.getByRole('button', { name: '复制回复 1' })).toBeInTheDocument();
  expect(screen.getByLabelText('Agent 回复 1')).toBeInTheDocument();
  expect(screen.getByLabelText('回复内容 1').closest('[data-turn-id="1"]')).not.toBeNull();
  expect(screen.getByRole('button', { name: '复制用户消息 1' })).toBeInTheDocument();
});

it('drops the preview footnote as well', () => {
  render(<ChatStream {...base} preview runs={[run('completed', 'answer')]} />);
  expect(screen.queryByText('示例对话 · 不执行任何操作')).toBeNull();
});

it('moves user actions outside the bubble and refills the prompt from edit', () => {
  const onEditPrompt = vi.fn();
  render(<ChatStream {...base} onEditPrompt={onEditPrompt} />);
  const copy = screen.getByRole('button', { name: '复制用户消息 1' });
  expect(copy.closest('.user-message')).toBeNull();
  expect(copy.closest('.user-message-group')).not.toBeNull();
  fireEvent.click(screen.getByRole('button', { name: '编辑用户消息 1' }));
  expect(onEditPrompt).toHaveBeenCalledWith('分析此项目');
});

it('renders a clickable turn rail once there are multiple turns', () => {
  const onJumpToTurn = vi.fn();
  const runs = [run('completed', 'first'), { ...run('completed', 'second'), id: 2 }];
  const view = render(<ChatStream {...base} runs={runs} onJumpToTurn={onJumpToTurn} />);
  const ticks = view.container.querySelectorAll<HTMLButtonElement>('.chat-turn-tick');
  expect(view.container.querySelector('.chat-turn-rail')).not.toBeNull();
  expect(ticks).toHaveLength(2);
  expect(ticks[1].getAttribute('aria-current')).toBe('location');

  fireEvent.mouseEnter(ticks[0]);
  const card = screen.getByRole('tooltip');
  expect(card).toHaveTextContent('You');
  expect(card).toHaveTextContent('Maestro');
  expect(card).toHaveTextContent('first');
  fireEvent.mouseLeave(ticks[0]);
  expect(screen.queryByRole('tooltip')).toBeNull();

  fireEvent.click(ticks[0]);
  expect(onJumpToTurn).toHaveBeenCalledWith(1);
  expect(view.container.querySelector('.conversation-turn')?.className).toContain('turn-flash');
});

it('collapses a reasoning burst when it finishes unless manually reopened', () => {
  const view = render(<ChatStream {...base} liveTimeline={{ 1: [reasoning('streaming', 0)] }} />);
  expect(view.container.querySelector('.reasoning-content')).not.toBeNull();
  view.rerender(<ChatStream {...base} liveTimeline={{ 1: [reasoning('streaming', 0, 3000)] }} />);
  expect(view.container.querySelector('.reasoning-content')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: /思考 · 3 秒/ }));
  expect(view.container.querySelector('.reasoning-content')).not.toBeNull();
});

it('stays memoized so a keystroke cannot re-render the whole history', () => {
  // Workbench stabilizes every prop (useCallback/useMemo); without this wrapper
  // the memo contract silently disappears and typing re-parses all messages.
  expect((ChatStream as unknown as { $$typeof: symbol }).$$typeof).toBe(Symbol.for('react.memo'));
});

it('folds historical tool calls into a collapsed execution group', () => {
  const calls: ToolCallView[] = [
    { runId: 1, tool: 'read_file', detail: 'a.ts', allowed: true },
    { runId: 1, tool: 'write_file', detail: 'b.ts', allowed: true },
  ];
  const { container } = render(<ChatStream {...base} runs={[run('completed')]} detailTools={calls} />);
  // No liveTimeline after a restart: replay must still fold, not dump every row.
  expect(screen.getByRole('button', { name: /已执行 2 个工具/ })).toHaveAttribute('aria-expanded', 'false');
  expect(container.querySelector('.execution-body')).toHaveAttribute('aria-hidden', 'true');
});

it('uses persisted reasoning so a replayed run keeps its thinking time', () => {
  const calls: ToolCallView[] = [
    { runId: 1, tool: 'read_file', detail: 'a.ts', allowed: true },
    { runId: 1, tool: 'write_file', detail: 'b.ts', allowed: true },
  ];
  render(<ChatStream {...base} runs={[run('completed')]} detailTools={calls}
    detailReasoning={{ 1: [{ text: '先分析', startedAt: 0, endedAt: 3000 }] }} />);
  const summary = screen.getByRole('button', { name: /思考 3 秒 · 2 个工具/ });
  expect(summary).toHaveAttribute('aria-expanded', 'false');
});

it('replays persisted parts in their original interleaved order', () => {
  const calls: ToolCallView[] = [
    { runId: 1, tool: 'read_file', detail: 'a.ts', allowed: true },
    { runId: 1, tool: 'write_file', detail: 'b.ts', allowed: true },
  ];
  const parts: PersistedPart[] = [
    { kind: 'reasoning', text: '先想', startedAt: 0, endedAt: 2000 },
    { kind: 'tool', index: 0, startedAt: 2000 },
    { kind: 'content', text: '中间说明', startedAt: 3000 },
    { kind: 'tool', index: 1, startedAt: 4000 },
  ];
  render(<ChatStream {...base} runs={[run('completed')]} detailTools={calls} detailParts={{ 1: parts }} />);
  // The content part splits the execution into two groups, exactly like a live run.
  expect(screen.getAllByRole('button', { name: /个工具/ })).toHaveLength(2);
  expect(screen.getByText('中间说明')).toBeInTheDocument();
});
