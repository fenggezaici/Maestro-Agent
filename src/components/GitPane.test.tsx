import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { GitStatus, GitSummary } from '../api/git';
import GitPane from './GitPane';
import type { GitRepoState } from '../hooks/useGitRepo';

vi.mock('../api/git', () => ({
  gitApi: {
    summary: vi.fn(),
    status: vi.fn(),
    diff: vi.fn(),
    filesOf: vi.fn(),
    log: vi.fn(),
  },
}));

import { gitApi } from '../api/git';

afterEach(cleanup);

function summaryOf(partial: Partial<GitSummary> = {}): GitSummary {
  return {
    repo: true, gitMissing: false, root: '/repo', branch: 'main', upstream: 'origin/main',
    ahead: 2, behind: 1, staged: 1, unstaged: 1, untracked: 1, conflicted: 0,
    userName: 'u', userEmail: 'e', ...partial,
  };
}

function statusOf(partial: Partial<GitStatus> = {}): GitStatus {
  return {
    staged: [{ path: 'staged.txt', xy: 'A.', origPath: null, added: 3, removed: 0 }],
    unstaged: [{ path: 'src/tracked.txt', xy: '.M', origPath: null, added: 1, removed: 2 }],
    untracked: [{ path: 'fresh.txt', xy: '?', origPath: null, added: null, removed: null }],
    conflicted: [],
    ...partial,
  };
}

function repoOf(partial: Partial<GitRepoState> = {}): GitRepoState {
  return { summary: summaryOf(), status: statusOf(), loading: false, refresh: vi.fn(), ...partial };
}

describe('GitPane', () => {
  it('非 git 仓库显示占位而非错误', () => {
    render(<GitPane git={repoOf({ summary: summaryOf({ repo: false }), status: null })} fontSize={12} />);
    expect(screen.getByText('非 git 仓库')).toBeTruthy();
  });

  it('git 缺失与仓库缺失可区分', () => {
    render(<GitPane git={repoOf({ summary: summaryOf({ repo: false, gitMissing: true }), status: null })} fontSize={12} />);
    expect(screen.getByText('未检测到 git')).toBeTruthy();
  });

  it('头部渲染分支徽章与 ahead/behind', () => {
    render(<GitPane git={repoOf()} fontSize={12} />);
    const badge = screen.getByTitle('上游 origin/main');
    expect(badge.textContent).toContain('main');
    expect(badge.textContent).toContain('↑2');
    expect(badge.textContent).toContain('↓1');
  });

  it('未暂存来源渲染分区：untracked 行不可展开', () => {
    render(<GitPane git={repoOf()} fontSize={12} />);
    expect(screen.getByText('未暂存的更改')).toBeTruthy();
    expect(screen.getByText('未跟踪的文件')).toBeTruthy();
    expect(screen.getByText('tracked.txt')).toBeTruthy();
    // untracked 行无 diff——chevron 为空且不响应展开
    const untrackedRow = screen.getByLabelText('fresh.txt（无 diff）');
    expect(untrackedRow.getAttribute('aria-expanded')).toBeNull();
  });

  it('展开行懒加载 diff 并渲染增删行', async () => {
    vi.mocked(gitApi.diff).mockResolvedValue({
      diff: 'diff --git a/src/tracked.txt b/src/tracked.txt\n@@ -1 +1 @@\n-old\n+new',
      truncated: false, binary: false,
    });
    render(<GitPane git={repoOf()} fontSize={12} />);
    fireEvent.click(screen.getByLabelText('src/tracked.txt'));
    expect(gitApi.diff).toHaveBeenCalledWith('src/tracked.txt', 'unstaged');
    await waitFor(() => expect(screen.getByText('new')).toBeTruthy());
    expect(screen.getByText('old')).toBeTruthy();
  });

  it('切换到已暂存来源用 --cached 语义加载 diff', async () => {
    vi.mocked(gitApi.diff).mockResolvedValue({ diff: '@@ -0,0 +1 @@\n+staged', truncated: false, binary: false });
    render(<GitPane git={repoOf()} fontSize={12} />);
    fireEvent.click(screen.getByRole('tab', { name: '已暂存' }));
    expect(screen.getByText('已暂存的更改')).toBeTruthy();
    fireEvent.click(screen.getByLabelText('staged.txt'));
    expect(gitApi.diff).toHaveBeenCalledWith('staged.txt', 'staged');
  });

  it('分支对比来源拉取 filesOf 并以 branch 语义加载 diff', async () => {
    vi.mocked(gitApi.filesOf).mockResolvedValue([
      { path: 'feature.ts', xy: 'M', origPath: null, added: 10, removed: 1 },
    ]);
    vi.mocked(gitApi.diff).mockResolvedValue({ diff: '@@ -1 +1 @@\n-a\n+b', truncated: false, binary: false });
    render(<GitPane git={repoOf()} fontSize={12} />);
    fireEvent.click(screen.getByRole('tab', { name: '分支对比' }));
    await waitFor(() => expect(screen.getByText('feature.ts')).toBeTruthy());
    expect(gitApi.filesOf).toHaveBeenCalledWith('branch:origin/main');
    fireEvent.click(screen.getByLabelText('feature.ts'));
    expect(gitApi.diff).toHaveBeenCalledWith('feature.ts', 'branch:origin/main');
  });

  it('无上游时不出现在源切换中', () => {
    render(<GitPane git={repoOf({ summary: summaryOf({ upstream: null }) })} fontSize={12} />);
    expect(screen.queryByRole('tab', { name: '分支对比' })).toBeNull();
  });

  it('干净工作区显示空态文案', () => {
    render(<GitPane git={repoOf({ status: statusOf({ staged: [], unstaged: [], untracked: [] }) })} fontSize={12} />);
    expect(screen.getByText('工作区干净，没有未暂存的更改')).toBeTruthy();
  });
});
