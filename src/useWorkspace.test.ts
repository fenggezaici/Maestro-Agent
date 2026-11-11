import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { useWorkspace } from './hooks/useWorkspace';

const api = vi.hoisted(() => ({ info: vi.fn(), select: vi.fn(), tree: vi.fn(), text: vi.fn() }));
vi.mock('./api/workspace', () => ({ workspaceApi: api }));

beforeEach(() => {
  localStorage.clear();
  vi.clearAllMocks();
  api.info.mockResolvedValue({ name: 'A', path: '/project-a' });
  api.select.mockImplementation(async (path: string) => ({ name: path, path }));
  api.tree.mockResolvedValue([{ name: 'file.ts', path: 'file.ts', is_dir: false, children: [] }]);
  api.text.mockResolvedValue('new content');
});

it('does not invoke Rust until Explorer is activated, then reads metadata and text separately', async () => {
  const hook = renderHook(({ enabled }) => useWorkspace(enabled), { initialProps: { enabled: false } });
  expect(api.info).not.toHaveBeenCalled();
  hook.rerender({ enabled: true });
  await waitFor(() => expect(hook.result.current.tree).toHaveLength(1));
  expect(api.tree).toHaveBeenCalledWith('/project-a');
  expect(api.text).not.toHaveBeenCalled();
  await act(async () => { await hook.result.current.selectFile('file.ts'); });
  expect(hook.result.current.file).toMatchObject({ source: 'disk', content: 'new content' });
  expect(api.text).toHaveBeenCalledWith('/project-a', 'file.ts');
});

it('ignores a delayed file from an old workspace and never falls back to sample files', async () => {
  let resolveOld!: (value: string) => void;
  const delayed = new Promise<string>((resolve) => { resolveOld = resolve; });
  api.text.mockImplementationOnce(() => delayed);
  const hook = renderHook(() => useWorkspace(true));
  await waitFor(() => expect(hook.result.current.tree).toHaveLength(1));
  act(() => { void hook.result.current.selectFile('old.ts'); });
  await act(async () => { await hook.result.current.selectWorkspace('/project-b'); });
  await act(async () => { await hook.result.current.selectFile('file.ts'); });
  await act(async () => { resolveOld('stale secret content'); });
  expect(hook.result.current.file?.content).toBe('new content');
  expect(hook.result.current.file?.id).toContain('/project-b');
  api.tree.mockRejectedValueOnce(new Error('permission denied'));
  await act(async () => { await hook.result.current.refresh(); });
  expect(hook.result.current.tree).toHaveLength(1);
  expect(hook.result.current.file?.content).toBe('new content');
  expect(hook.result.current.error).toBe('permission denied');
});

it('reuses the cached tree instead of rescanning until a forced refresh', async () => {
  const hook = renderHook(() => useWorkspace(true));
  await waitFor(() => expect(hook.result.current.tree).toHaveLength(1));
  expect(api.tree).toHaveBeenCalledTimes(1);
  await act(async () => { await hook.result.current.load(); });
  expect(api.tree).toHaveBeenCalledTimes(1);
  await act(async () => { await hook.result.current.refresh(); });
  expect(api.tree).toHaveBeenCalledTimes(2);
});

it('ignores an old file error after a newer selection succeeds', async () => {
  let rejectOld!: (value: Error) => void;
  api.text.mockImplementationOnce(() => new Promise<string>((_resolve, reject) => { rejectOld = reject; }));
  const hook = renderHook(() => useWorkspace(true));
  await waitFor(() => expect(hook.result.current.tree).toHaveLength(1));
  act(() => { void hook.result.current.selectFile('old.ts'); });
  await act(async () => { await hook.result.current.selectFile('new.ts'); });
  await act(async () => { rejectOld(new Error('stale failure')); });
  expect(hook.result.current.file?.path).toBe('new.ts');
  expect(hook.result.current.fileError).toBeNull();
  expect(hook.result.current.fileLoading).toBe(false);
});
