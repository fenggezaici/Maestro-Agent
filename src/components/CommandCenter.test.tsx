import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import CommandCenter, { type PaletteCommand } from './CommandCenter';
import type { WorkbenchProject } from './workspace';

const projects: WorkbenchProject[] = [{
  id: 'p1', name: 'Demo', path: null,
  sessions: [
    { id: 1, title: 'Refactor login flow', updatedAt: 1, runCount: 3, projectId: 1, archived: false },
    { id: 2, title: 'Fix diff panel', updatedAt: 1, runCount: 1, projectId: 1, archived: false },
  ],
}];

function setup(overrides: Partial<Parameters<typeof CommandCenter>[0]> = {}) {
  const props = {
    open: true, onOpenChange: vi.fn(), source: 'preview' as const, projects,
    commands: [{ id: 'settings', label: 'Open Settings', run: vi.fn() } satisfies PaletteCommand] as PaletteCommand[],
    files: [{ path: 'src/bridge/BridgeSession.h', name: 'BridgeSession.h' }],
    onSelectSession: vi.fn(), onSelectFile: vi.fn(),
    ...overrides,
  };
  const view = render(<CommandCenter {...props} />);
  return { props, unmount: view.unmount };
}

beforeEach(() => localStorage.clear());

it('lists commands when the query is empty', () => {
  setup();
  expect(screen.getByText('Commands')).toBeInTheDocument();
  expect(screen.getByText('Open Settings')).toBeInTheDocument();
});

it('runs the highlighted command on Enter and closes', () => {
  const { props } = setup();
  const input = screen.getByLabelText('搜索命令、会话或文件');
  fireEvent.change(input, { target: { value: 'sett' } });
  fireEvent.keyDown(input, { key: 'Enter' });
  expect(props.commands[0].run).toHaveBeenCalled();
  expect(props.onOpenChange).toHaveBeenCalledWith(false);
});

it('navigates to sessions and files with arrow keys', () => {
  const { props } = setup();
  const input = screen.getByLabelText('搜索命令、会话或文件');
  fireEvent.change(input, { target: { value: 'login' } });
  fireEvent.keyDown(input, { key: 'Enter' });
  expect(props.onSelectSession).toHaveBeenCalledWith(1);
  fireEvent.change(input, { target: { value: 'bridge' } });
  fireEvent.keyDown(input, { key: 'ArrowDown' });
  fireEvent.keyDown(input, { key: 'Enter' });
  expect(props.onSelectFile).toHaveBeenCalledWith('src/bridge/BridgeSession.h');
});

it('re-applies a recent query without closing the palette', () => {
  const first = setup();
  const input = screen.getByLabelText('搜索命令、会话或文件');
  fireEvent.change(input, { target: { value: 'login' } });
  fireEvent.keyDown(input, { key: 'Enter' });
  expect(first.props.onOpenChange).toHaveBeenCalledWith(false);
  first.unmount();
  setup();
  expect(screen.getByText('Recent')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('option', { name: /Refactor login flow/ }));
  expect(screen.getByLabelText('搜索命令、会话或文件')).toHaveValue('Refactor login flow');
});
