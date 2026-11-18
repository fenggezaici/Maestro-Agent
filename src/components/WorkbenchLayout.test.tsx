import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi, type MockInstance } from 'vitest';
import WorkbenchLayout from './WorkbenchLayout';

const SHELL_WIDTH = 1200;

function setViewport(width: number) {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: width });
}

function renderShell(width = 1400) {
  setViewport(width);
  const view = render(<WorkbenchLayout
    source="live"
    sidebar={<nav aria-label="侧栏内容" />}
    conversation={<div aria-label="对话内容" />}
    review={<div aria-label="Diff 内容" />}
  />);
  return { view, shell: view.container.querySelector('.workbench-layout') as HTMLDivElement };
}

let rectSpy: MockInstance;

beforeEach(() => {
  localStorage.clear();
  // jsdom reports a 0×0 box; give the shell a real width so the clamp math runs.
  rectSpy = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
    width: SHELL_WIDTH, height: 800, top: 0, left: 0, right: SHELL_WIDTH, bottom: 800, x: 0, y: 0,
    toJSON: () => ({}),
  } as DOMRect);
});

afterEach(() => vi.restoreAllMocks());

it('exposes the sidebar separator as a focusable, keyboard-operable control', () => {
  const { shell } = renderShell();
  const separator = screen.getByRole('separator', { name: '调整侧栏宽度' });
  expect(separator).toHaveAttribute('tabindex', '0');
  expect(separator).toHaveAttribute('aria-orientation', 'vertical');
  expect(separator).toHaveAttribute('aria-controls', 'sidebar-panel');
  expect(separator).toHaveAttribute('aria-valuemin', '220');
  expect(separator).toHaveAttribute('aria-valuemax', '360');
  expect(separator).toHaveAttribute('aria-valuenow', '280');

  fireEvent.keyDown(separator, { key: 'ArrowRight' });
  expect(separator).toHaveAttribute('aria-valuenow', '296');
  expect(shell.style.getPropertyValue('--sidebar-width')).toBe('296px');
  expect(localStorage.getItem('maestro.sidebar.width.v1')).toBe('296');

  fireEvent.keyDown(separator, { key: 'ArrowLeft' });
  fireEvent.keyDown(separator, { key: 'ArrowLeft' });
  expect(separator).toHaveAttribute('aria-valuenow', '264');

  fireEvent.keyDown(separator, { key: 'Home' });
  expect(separator).toHaveAttribute('aria-valuenow', '220');
  fireEvent.keyDown(separator, { key: 'End' });
  expect(separator).toHaveAttribute('aria-valuenow', '360');
  expect(localStorage.getItem('maestro.sidebar.width.v1')).toBe('360');
});

it('clamps the review separator to the shell and persists the keyboard width', () => {
  const { shell } = renderShell(1200);
  const separator = screen.getByRole('separator', { name: '调整对话与 Diff 宽度' });
  // 1200 * 0.42 fallback on first mount.
  expect(separator).toHaveAttribute('aria-valuenow', '504');
  fireEvent.keyDown(separator, { key: 'ArrowLeft' });
  expect(separator).toHaveAttribute('aria-valuenow', '520');

  fireEvent.keyDown(separator, { key: 'End' });
  // 1200 * 0.65 = 780
  expect(separator).toHaveAttribute('aria-valuenow', '780');
  expect(shell.style.getPropertyValue('--review-width')).toBe('780px');
  expect(localStorage.getItem('maestro.review.width.v1')).toBe('780');
});

it('commits a pointer drag and restores the start width on cancel', async () => {
  const { shell } = renderShell();
  const separator = screen.getByRole('separator', { name: '调整侧栏宽度' });
  const start = Number(separator.getAttribute('aria-valuenow'));

  fireEvent.pointerDown(separator, { pointerId: 7, pointerType: 'mouse', button: 0, clientX: 400 });
  fireEvent.pointerMove(separator, { pointerId: 7, clientX: 440 });
  // Drag paints inside a requestAnimationFrame, never synchronously per event.
  await waitFor(() => expect(shell.style.getPropertyValue('--sidebar-width')).toBe(`${start + 40}px`));
  fireEvent.pointerUp(separator, { pointerId: 7, clientX: 440 });
  expect(separator).toHaveAttribute('aria-valuenow', String(start + 40));
  expect(localStorage.getItem('maestro.sidebar.width.v1')).toBe(String(start + 40));

  fireEvent.pointerDown(separator, { pointerId: 8, pointerType: 'mouse', button: 0, clientX: 400 });
  fireEvent.pointerMove(separator, { pointerId: 8, clientX: 200 });
  fireEvent.pointerCancel(separator, { pointerId: 8, clientX: 200 });
  expect(separator).toHaveAttribute('aria-valuenow', String(start + 40));
  expect(shell.style.getPropertyValue('--sidebar-width')).toBe(`${start + 40}px`);
});

it('coalesces pointer moves into one animation frame', async () => {
  const { shell } = renderShell();
  const separator = screen.getByRole('separator', { name: '调整侧栏宽度' });
  const start = Number(separator.getAttribute('aria-valuenow'));
  const frames: FrameRequestCallback[] = [];
  const raf = vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => {
    frames.push(callback);
    return frames.length;
  });

  fireEvent.pointerDown(separator, { pointerId: 3, pointerType: 'mouse', button: 0, clientX: 400 });
  fireEvent.pointerMove(separator, { pointerId: 3, clientX: 410 });
  fireEvent.pointerMove(separator, { pointerId: 3, clientX: 420 });
  fireEvent.pointerMove(separator, { pointerId: 3, clientX: 430 });
  expect(frames).toHaveLength(1);
  expect(shell.style.getPropertyValue('--sidebar-width')).toBe(`${start}px`);

  frames[0](0);
  await waitFor(() => expect(shell.style.getPropertyValue('--sidebar-width')).toBe(`${start + 30}px`));
  raf.mockRestore();
});

it('ignores pointer events from another pointer id', async () => {
  const { shell } = renderShell();
  const separator = screen.getByRole('separator', { name: '调整侧栏宽度' });
  const start = Number(separator.getAttribute('aria-valuenow'));
  fireEvent.pointerDown(separator, { pointerId: 1, pointerType: 'mouse', button: 0, clientX: 400 });
  fireEvent.pointerMove(separator, { pointerId: 2, clientX: 520 });
  await waitFor(() => expect(shell.style.getPropertyValue('--sidebar-width')).toBe(`${start}px`));
});

it('never reads layout during render', () => {
  const { view } = renderShell();
  rectSpy.mockClear();
  // A keystroke in the composer re-renders the whole workbench tree with new
  // element props; that re-render must not force a synchronous layout.
  view.rerender(<WorkbenchLayout
    source="live"
    sidebar={<nav aria-label="侧栏内容" />}
    conversation={<div aria-label="对话内容" />}
    review={<div aria-label="Diff 内容" />}
  />);
  expect(rectSpy).not.toHaveBeenCalled();
});

it('renders the stacked layout without a fake draggable divider', () => {
  renderShell(900);
  expect(screen.queryByRole('separator', { name: '调整对话与 Diff 宽度' })).not.toBeInTheDocument();
  const divider = document.querySelector('.stacked-divider');
  expect(divider).not.toBeNull();
  expect(divider).toHaveAttribute('aria-hidden', 'true');
  expect(document.querySelector('.resize-handle.is-horizontal')).toBeNull();
});
