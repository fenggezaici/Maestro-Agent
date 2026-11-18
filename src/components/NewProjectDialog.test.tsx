import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { EditProjectDialog } from './NewProjectDialog';
import type { WorkbenchProject } from './workspace';

const project = (over: Partial<WorkbenchProject> = {}): WorkbenchProject => ({
  id: '7',
  name: '我的项目',
  path: 'D:\\work\\my-project',
  sessions: [],
  ...over,
});

const base = {
  onOpenChange: vi.fn(),
  onBrowse: vi.fn(async () => null as string | null),
  onSave: vi.fn(async () => null as string | null),
};

it('prefills name and workspace path from the project', () => {
  render(<EditProjectDialog project={project()} {...base} />);
  expect(screen.getByLabelText('项目名称')).toHaveValue('我的项目');
  expect(screen.getByTitle('D:\\work\\my-project')).toHaveTextContent('my-project');
  expect(screen.getByRole('button', { name: '保存' })).toBeEnabled();
});

it('trims the name and rejects empty names before saving', async () => {
  const onSave = vi.fn(async () => null);
  render(<EditProjectDialog project={project()} {...base} onSave={onSave} />);
  const input = screen.getByLabelText('项目名称');
  fireEvent.change(input, { target: { value: '   ' } });
  fireEvent.click(screen.getByRole('button', { name: '保存' }));
  expect(onSave).not.toHaveBeenCalled();

  fireEvent.change(input, { target: { value: '  新项目名  ' } });
  fireEvent.click(screen.getByRole('button', { name: '保存' }));
  await waitFor(() => expect(onSave).toHaveBeenCalledWith('新项目名', 'D:\\work\\my-project'));
});

it('lets the user pick a new folder and surfaces save errors', async () => {
  const onBrowse = vi.fn(async () => 'D:\\elsewhere\\picked');
  const onSave = vi.fn(async () => '项目目录不存在或无权限');
  render(<EditProjectDialog project={project({ path: null })} {...base} onBrowse={onBrowse} onSave={onSave} />);

  fireEvent.click(screen.getByRole('button', { name: /添加文件夹/ }));
  await waitFor(() => expect(screen.getByTitle('D:\\elsewhere\\picked')).toBeInTheDocument());

  fireEvent.click(screen.getByRole('button', { name: '保存' }));
  await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('项目目录不存在或无权限'));
});

it('routes the delete action to the parent instead of deleting inline', () => {
  const onDelete = vi.fn();
  render(<EditProjectDialog project={project()} {...base} onDelete={onDelete} />);
  fireEvent.click(screen.getByRole('button', { name: '删除项目' }));
  expect(onDelete).toHaveBeenCalledWith('7');
});

it('stays empty when no project is selected', () => {
  const { container } = render(<EditProjectDialog project={null} {...base} />);
  expect(container.querySelector('.project-dialog')).not.toBeInTheDocument();
});
