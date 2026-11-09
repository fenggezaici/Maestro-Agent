import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import PreviewApp from '../preview/PreviewApp';
import MarkdownMessage from './MarkdownMessage';
import type { ProposedDiff } from '../ipc';
import type { ExplorerFile } from './workbenchMock';

vi.mock('./MonacoDiff', () => ({ default: ({ diff }: { diff: ProposedDiff }) => <section aria-label={`Diff ${diff.path}`} /> }));
vi.mock('./MonacoCode', () => ({ default: ({ file }: { file: ExplorerFile }) => <section aria-label={`文件预览 ${file.path}`} /> }));
const copy = vi.fn().mockResolvedValue(undefined);
const revoke = vi.fn();
beforeEach(() => {
  localStorage.clear();
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: copy } });
  Object.defineProperty(URL, 'createObjectURL', { configurable: true, value: vi.fn(() => 'blob:local-preview') });
  Object.defineProperty(URL, 'revokeObjectURL', { configurable: true, value: revoke });
});

it('renders safe Markdown with code language and clipboard controls without raw HTML', async () => {
  const view = render(<MarkdownMessage light={false} content={'## Plan\n\n```typescript\nconst value = 42;\n```\n\n<script>alert(1)</script>\n\n![remote](https://example.invalid/image.png)'} />);
  expect(screen.getByRole('heading', { name: 'Plan' })).toBeInTheDocument();
  expect(screen.getByText('typescript')).toBeInTheDocument();
  expect(view.container.querySelector('script')).toBeNull();
  expect(view.container.querySelector('img')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: '复制代码' }));
  await waitFor(() => expect(copy).toHaveBeenCalledWith('const value = 42;'));
});

it('applies Light and basic language labels immediately, with isolated UI preferences', () => {
  render(<PreviewApp />);
  fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
  fireEvent.click(screen.getByRole('button', { name: 'Appearance' }));
  fireEvent.keyDown(screen.getByLabelText('主题'), { key: 'ArrowDown' });
  fireEvent.click(screen.getByRole('option', { name: 'Light' }));
  expect(document.documentElement.dataset.theme).toBe('light');
  fireEvent.click(screen.getByRole('button', { name: 'General' }));
  fireEvent.keyDown(screen.getByLabelText('界面语言'), { key: 'ArrowDown' });
  fireEvent.click(screen.getByRole('option', { name: '简体中文' }));
  expect(document.documentElement.lang).toBe('zh-CN');
  expect(screen.getByRole('button', { name: '外观' })).toBeInTheDocument();
});

it('shows local image thumbnails only and revokes removed object URLs', () => {
  render(<PreviewApp />);
  const image = new File(['local'], 'sample.png', { type: 'image/png' });
  fireEvent.change(screen.getByLabelText('选择本地媒体附件'), { target: { files: [image] } });
  expect(screen.getByRole('img', { name: 'sample.png' })).toHaveAttribute('src', 'blob:local-preview');
  expect(screen.getByText('Local preview only')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: '移除附件 sample.png' }));
  expect(revoke).toHaveBeenCalledWith('blob:local-preview');
});

it('follows system appearance and cleans up the media-query listener', () => {
  let dark = true;
  const listeners = new Set<() => void>();
  const media = { get matches() { return dark; }, addEventListener: (_type: string, callback: () => void) => listeners.add(callback), removeEventListener: (_type: string, callback: () => void) => listeners.delete(callback) };
  vi.stubGlobal('matchMedia', () => media);
  const view = render(<PreviewApp />);
  fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
  fireEvent.click(screen.getByRole('button', { name: 'Appearance' }));
  fireEvent.keyDown(screen.getByLabelText('主题'), { key: 'ArrowDown' });
  fireEvent.click(screen.getByRole('option', { name: 'System' }));
  expect(document.documentElement.dataset.theme).toBe('dark');
  act(() => { dark = false; listeners.forEach((callback) => callback()); });
  expect(document.documentElement.dataset.theme).toBe('light');
  view.unmount();
  expect(listeners.size).toBe(0);
  vi.unstubAllGlobals();
});
