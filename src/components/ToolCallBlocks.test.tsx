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

// Radix DropdownMenu opens on pointerdown/keydown — jsdom fireEvent has no
// pointer pipeline, so ArrowDown is the reliable open gesture.
async function openApprovalMenu() {
  fireEvent.keyDown(screen.getByLabelText('更多审批操作'), { key: 'ArrowDown' });
  return screen.findByRole('menu');
}

it('shows the exact rule and the wider prefix rule in the dropdown before granting', async () => {
  const onResolveApproval = vi.fn();
  render(<ToolCallRow call={awaiting({})} onResolveApproval={onResolveApproval} />);

  const menu = await openApprovalMenu();
  expect(within(menu).getByText('npm run test')).toBeInTheDocument();
  expect(within(menu).getByText('npm *')).toBeInTheDocument();

  fireEvent.click(within(menu).getByText('始终允许（仅此调用）'));
  expect(onResolveApproval).toHaveBeenCalledWith(7, 'allow_always', 'exact');

  const menu2 = await openApprovalMenu();
  fireEvent.click(within(menu2).getByText('始终允许（命令前缀）'));
  expect(onResolveApproval).toHaveBeenCalledWith(7, 'allow_always', 'prefix');
});

it('defaults non-command tools to a single exact rule in the dropdown', async () => {
  const onResolveApproval = vi.fn();
  render(<ToolCallRow
    call={awaiting({ tool: 'write_file', target: 'src/a.ts', detail: JSON.stringify({ kind: 'write', path: 'src/a.ts', exists: false, old: '', new: 'x' }) })}
    onResolveApproval={onResolveApproval} />);

  const menu = await openApprovalMenu();
  expect(within(menu).queryByText('始终允许（命令前缀）')).not.toBeInTheDocument();
  fireEvent.click(within(menu).getByText('始终允许（仅此调用）'));
  expect(onResolveApproval).toHaveBeenCalledWith(7, 'allow_always', 'exact');
});

it('offers batch and session decisions for a suspended command', async () => {
  const onResolveApproval = vi.fn();
  render(<ToolCallRow call={awaiting({})} onResolveApproval={onResolveApproval} />);

  const menu = await openApprovalMenu();
  fireEvent.click(within(menu).getByText('本批全部允许'));
  expect(onResolveApproval).toHaveBeenCalledWith(7, 'allow_batch');

  const menu2 = await openApprovalMenu();
  fireEvent.click(within(menu2).getByText('本会话全部放行'));
  expect(onResolveApproval).toHaveBeenCalledWith(7, 'allow_session');

  const menu3 = await openApprovalMenu();
  fireEvent.click(within(menu3).getByText('本批全部拒绝'));
  expect(onResolveApproval).toHaveBeenCalledWith(7, 'deny_batch');
});

it('lists session rules and revokes one', async () => {
  listApprovalRules.mockResolvedValue([
    { tool: 'run_command', pattern: 'npm', prefix: true },
    { tool: 'write_file', pattern: 'src/a.ts', prefix: false },
  ]);
  revokeApprovalRule.mockResolvedValue(true);
  render(<ToolCallRow call={awaiting({})} onResolveApproval={vi.fn()} />);

  const menu = await openApprovalMenu();
  fireEvent.click(within(menu).getByText('授权规则…'));
  expect(await screen.findByText('本会话授权规则')).toBeInTheDocument();
  expect(listApprovalRules).toHaveBeenCalledWith(7);
  expect(await screen.findByText('npm *')).toBeInTheDocument();
  expect(screen.getByText('前缀：同前缀的其他命令也会免审')).toBeInTheDocument();

  fireEvent.click(screen.getByLabelText('撤销规则 run_command npm'));
  await waitFor(() => expect(revokeApprovalRule).toHaveBeenCalledWith(7, { tool: 'run_command', pattern: 'npm', prefix: true }));
  await waitFor(() => expect(screen.queryByText('npm *')).not.toBeInTheDocument());
});
