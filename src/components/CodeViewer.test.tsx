import { render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { ReactNode } from 'react';

import CodeViewer from './CodeViewer';
import type { ExplorerFile } from './workbenchMock';

const body = vi.hoisted(() => vi.fn());
vi.mock('../viewer/loader', () => ({
  loadViewer: () => Promise.resolve({
    ViewerPool: ({ children }: { children: ReactNode }) => <div data-testid="pool">{children}</div>,
    ViewerBody: (props: { file: ExplorerFile; fontSize: number; fontFamily: string }) => {
      body(props);
      return <div data-testid="body">{props.file.content}</div>;
    },
  }),
  preloadViewer: vi.fn(),
}));

const file: ExplorerFile = { id: 'f1', path: 'src/main.ts', content: 'const a = 1;', source: 'disk' };

describe('CodeViewer', () => {
  it('lazy-loads the viewer chunk and forwards the file to ViewerBody inside the pool', async () => {
    render(<CodeViewer file={file} fontSize={13} fontFamily="Cascadia Code" />);
    expect(screen.getByLabelText('文件预览 src/main.ts')).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId('body')).toHaveTextContent('const a = 1;'));
    expect(body).toHaveBeenCalledWith(expect.objectContaining({ file, fontSize: 13, fontFamily: 'Cascadia Code' }));
    expect(screen.getByTestId('pool')).toContainElement(screen.getByTestId('body'));
  });
});
