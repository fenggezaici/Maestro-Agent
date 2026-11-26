import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import GitCommitLog from './GitCommitLog';

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

function commitOf(partial: Record<string, unknown> = {}) {
  return {
    hash: 'abcdef1234567890', parents: ['parent1'], author: '甲', email: 'a@b.c',
    timestamp: Math.floor(Date.now() / 1000) - 120, subject: '修复登录', refs: [], ...partial,
  };
}

describe('GitCommitLog', () => {
  it('激活时拉取第一页并渲染提交行', async () => {
    vi.mocked(gitApi.log).mockResolvedValue([commitOf() as never]);
    render(<GitCommitLog fontSize={12} active />);
    await waitFor(() => expect(screen.getByText('修复登录')).toBeTruthy());
    expect(gitApi.log).toHaveBeenCalledWith(100, 0);
    expect(screen.getByText('abcdef1')).toBeTruthy();
    expect(screen.getByText(/甲 ·/)).toBeTruthy();
  });

  it('未激活时不发请求', () => {
    vi.mocked(gitApi.log).mockClear();
    render(<GitCommitLog fontSize={12} active={false} />);
    expect(gitApi.log).not.toHaveBeenCalled();
  });

  it('refs 渲染徽章：HEAD 分支高亮、tag 去前缀', async () => {
    vi.mocked(gitApi.log).mockResolvedValue([commitOf({ refs: ['HEAD -> main', 'tag: v1.0', 'origin/main'] }) as never]);
    render(<GitCommitLog fontSize={12} active />);
    await waitFor(() => expect(screen.getByText('main')).toBeTruthy());
    expect(screen.getByText('v1.0')).toBeTruthy();
    expect(screen.getByText('origin/main')).toBeTruthy();
    expect(screen.getByText('main').className).toContain('head');
  });

  it('展开提交加载文件清单，文件行可再展开 diff', async () => {
    vi.mocked(gitApi.log).mockResolvedValue([commitOf() as never]);
    vi.mocked(gitApi.filesOf).mockResolvedValue([
      { path: 'src/a.ts', xy: 'M', origPath: null, added: 5, removed: 1 },
    ]);
    vi.mocked(gitApi.diff).mockResolvedValue({ diff: '@@ -1 +1 @@\n-x\n+y', truncated: false, binary: false });
    render(<GitCommitLog fontSize={12} active />);
    await waitFor(() => expect(screen.getByText('修复登录')).toBeTruthy());
    fireEvent.click(screen.getByLabelText(/^提交 abcdef1/));
    await waitFor(() => expect(screen.getByText('a.ts')).toBeTruthy());
    expect(gitApi.filesOf).toHaveBeenCalledWith('commit:abcdef1234567890');
    fireEvent.click(screen.getByLabelText('src/a.ts'));
    expect(gitApi.diff).toHaveBeenCalledWith('src/a.ts', 'commit:abcdef1234567890');
    await waitFor(() => expect(screen.getByText('y')).toBeTruthy());
  });

  it('分页：加载更多带 skip，末页后按钮消失', async () => {
    const full = Array.from({ length: 100 }, (_, index) => commitOf({ hash: `h${index}abcdef`, subject: `c${index}` }));
    vi.mocked(gitApi.log).mockResolvedValueOnce(full as never).mockResolvedValue([commitOf({ hash: 'last000', subject: '末页' })] as never);
    render(<GitCommitLog fontSize={12} active />);
    await waitFor(() => expect(screen.getByText('c0')).toBeTruthy());
    const more = screen.getByText('加载更多');
    fireEvent.click(more);
    await waitFor(() => expect(screen.getByText('末页')).toBeTruthy());
    expect(gitApi.log).toHaveBeenLastCalledWith(100, 100);
    // 第二页只有 1 条 < PAGE_SIZE → 分页耗尽
    await waitFor(() => expect(screen.queryByText('加载更多')).toBeNull());
  });

  it('空历史显示空态文案', async () => {
    vi.mocked(gitApi.log).mockResolvedValue([]);
    render(<GitCommitLog fontSize={12} active />);
    await waitFor(() => expect(screen.getByText('没有提交历史')).toBeTruthy());
  });
});
