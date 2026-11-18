import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import RevertChangesDialog from './RevertChangesDialog';

const base = {
  open: true,
  onOpenChange: vi.fn(),
  loading: false,
  applying: false,
  error: null as string | null,
  onApply: vi.fn(),
};

it('lists safe, conflicted and skipped files with readable reasons', () => {
  render(<RevertChangesDialog {...base}
    preview={{
      safeFiles: ['src/a.ts'],
      unsafeFiles: [{ path: 'src/b.ts', reason: 'external_modified' }],
      ignoredFiles: [{ path: 'src/c.ts', reason: 'checkpoint_missing' }],
    }} />);

  expect(screen.getByText(/个文件可安全撤销/).closest('p')).toHaveTextContent('1 个文件可安全撤销');
  expect(screen.getByText('src/a.ts')).toBeInTheDocument();
  expect(screen.getByText('文件在之后被修改')).toBeInTheDocument();
  expect(screen.getByText('旧版本没有可回退快照')).toBeInTheDocument();
});

it('cannot apply when nothing is safe to revert', () => {
  render(<RevertChangesDialog {...base}
    preview={{ safeFiles: [], unsafeFiles: [{ path: 'src/b.ts', reason: 'file_read_failed' }], ignoredFiles: [] }} />);
  expect(screen.getByRole('button', { name: '确认撤销' })).toBeDisabled();
});

it('applies the revert and surfaces backend errors', () => {
  const onApply = vi.fn();
  const { rerender } = render(<RevertChangesDialog {...base} onApply={onApply}
    preview={{ safeFiles: ['src/a.ts'], unsafeFiles: [], ignoredFiles: [] }} />);

  fireEvent.click(screen.getByRole('button', { name: '确认撤销' }));
  expect(onApply).toHaveBeenCalledTimes(1);

  rerender(<RevertChangesDialog {...base} onApply={onApply} error="撤销失败：文件被占用"
    preview={{ safeFiles: ['src/a.ts'], unsafeFiles: [], ignoredFiles: [] }} />);
  expect(screen.getByRole('alert')).toHaveTextContent('撤销失败：文件被占用');
});
