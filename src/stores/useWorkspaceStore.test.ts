/// Workspace store concurrency guards: duplicate file reads share one IPC call
/// and directory refreshes coalesce instead of stacking overlapping scans.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Mock } from 'vitest';

import type { FileNode, WorkspaceInfo } from '../api/workspace';

vi.mock('../api/workspace', () => ({
  workspaceApi: {
    info: vi.fn(async (): Promise<WorkspaceInfo> => ({ name: 'root', path: 'E:/root' })),
    select: vi.fn(async (path: string): Promise<WorkspaceInfo> => ({ name: 'root', path })),
    tree: vi.fn(async (): Promise<FileNode[]> => []),
    text: vi.fn(async () => 'file content'),
  },
}));

import { workspaceApi } from '../api/workspace';
import { setWorkspaceMounted, useWorkspaceStore } from './useWorkspaceStore';

const treeMock = workspaceApi.tree as Mock;
const textMock = workspaceApi.text as Mock;

beforeEach(() => {
  vi.clearAllMocks();
  useWorkspaceStore.getState().disconnect();
  setWorkspaceMounted(true);
});

async function flushTicks(times = 10) {
  for (let index = 0; index < times; index++) await new Promise((resolve) => setTimeout(resolve, 0));
}

describe('useWorkspaceStore concurrency guards', () => {
  it('shares one IPC read for concurrent selectFile calls on the same path', async () => {
    let resolveText!: (value: string) => void;
    textMock.mockImplementationOnce(() => new Promise<string>((resolve) => { resolveText = resolve; }));
    await useWorkspaceStore.getState().load();
    const first = useWorkspaceStore.getState().selectFile('a.py');
    const second = useWorkspaceStore.getState().selectFile('a.py');
    resolveText('shared content');
    await Promise.all([first, second]);
    expect(textMock).toHaveBeenCalledTimes(1);
    expect(useWorkspaceStore.getState().file?.content).toBe('shared content');
    expect(useWorkspaceStore.getState().fileLoading).toBe(false);
  });

  it('coalesces refreshes while a directory scan is in flight', async () => {
    const pending: Array<(nodes: FileNode[]) => void> = [];
    treeMock.mockImplementation(() => new Promise<FileNode[]>((resolve) => { pending.push(resolve); }));
    const first = useWorkspaceStore.getState().load();
    await flushTicks();
    expect(treeMock).toHaveBeenCalledTimes(1);
    // Two refreshes during the scan queue once instead of stacking scans.
    void useWorkspaceStore.getState().refresh();
    void useWorkspaceStore.getState().refresh();
    await flushTicks();
    expect(treeMock).toHaveBeenCalledTimes(1);
    pending[0]([]);
    await first;
    await flushTicks();
    expect(treeMock).toHaveBeenCalledTimes(2);
    pending[1]([]);
    await flushTicks();
    expect(treeMock).toHaveBeenCalledTimes(2);
  });
});
