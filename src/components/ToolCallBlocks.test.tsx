import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { ToolCallRow } from './ToolCallBlocks';
import type { ToolCallView } from '../api/types';

const listApprovalRules = vi.fn();
const revokeApprovalRule = vi.fn();
vi.mock('../api/kernel', () => ({
  kernelApi: {
    listApprovalRules: (...args: unknown[]) => listApprovalRules(...args),
    revokeApprovalRule: (...args: unknown[]) => revokeApprovalRule(...args),
  },
}));

const awaiting = (overrides: Partial<ToolCallView>): ToolCallView => ({
  runId: 7,
  tool: 'run_command',
  detail: JSON.stringify({ kind: 'command', command: 'npm run test' }),
  allowed: true,
  awaitingApproval: true,
  ...overrides,
});

beforeEach(() => {
  listApprovalRules.mockReset();
  revokeApprovalRule.mockReset();
});

it('shows the exact rule and the wider prefix rule before granting', () => {
  const onResolveApproval = vi.fn();
  render(<ToolCallRow call={awaiting({})} onResolveApproval={onResolveApproval} />);

  fireEvent.click(screen.getByLabelText('始终允许工具调用'));
  const menu = screen.getByRole('menu');
  expect(within(menu).getByText('仅此命令（推荐）')).toBeInTheDocument();
  expect(within(menu).getByText('npm run test')).toBeInTheDocument();
  expect(within(menu).getByText('npm *')).toBeInTheDocument();

  fireEvent.click(screen.getByText('仅此命令（推荐）'));
  expect(onResolveApproval).toHaveBeenCalledWith(7, 'allow_always', 'exact');

  fireEvent.click(screen.getByLabelText('始终允许工具调用'));
  fireEvent.click(screen.getByText('命令前缀（更宽）'));
  expect(onResolveApproval).toHaveBeenCalledWith(7, 'allow_always', 'prefix');
});

it('defaults non-command tools to an exact rule without a scope menu', () => {
  const onResolveApproval = vi.fn();
  render(<ToolCallRow
    call={awaiting({ tool: 'write_file', target: 'src/a.ts', detail: JSON.stringify({ kind: 'write', path: 'src/a.ts', exists: false, old: '', new: 'x' }) })}
    onResolveApproval={onResolveApproval} />);

  fireEvent.click(screen.getByLabelText('始终允许工具调用'));
  expect(onResolveApproval).toHaveBeenCalledWith(7, 'allow_always', 'exact');
  expect(screen.queryByText('命令前缀（更宽）')).not.toBeInTheDocument();
});

it('lists session rules and revokes one', async () => {
  listApprovalRules.mockResolvedValue([
    { tool: 'run_command', pattern: 'npm', prefix: true },
    { tool: 'write_file', pattern: 'src/a.ts', prefix: false },
  ]);
  revokeApprovalRule.mockResolvedValue(true);
  render(<ToolCallRow call={awaiting({})} onResolveApproval={vi.fn()} />);

  fireEvent.click(screen.getByLabelText('查看本会话授权规则'));
  expect(await screen.findByText('本会话授权规则')).toBeInTheDocument();
  expect(listApprovalRules).toHaveBeenCalledWith(7);
  expect(await screen.findByText('npm *')).toBeInTheDocument();
  expect(screen.getByText('前缀：同前缀的其他命令也会免审')).toBeInTheDocument();

  fireEvent.click(screen.getByLabelText('撤销规则 run_command npm'));
  await waitFor(() => expect(revokeApprovalRule).toHaveBeenCalledWith(7, { tool: 'run_command', pattern: 'npm', prefix: true }));
  await waitFor(() => expect(screen.queryByText('npm *')).not.toBeInTheDocument());
});
