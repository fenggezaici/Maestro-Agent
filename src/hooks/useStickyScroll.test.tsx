import { useRef } from 'react';
import { act, fireEvent, render } from '@testing-library/react';
import { expect, it } from 'vitest';

import { useStickyScroll, type StickyScroll } from './useStickyScroll';

function Probe({ tick, api }: { tick: number; api?: { current: StickyScroll | null } }) {
  const ref = useRef<HTMLDivElement>(null);
  const scroll = useStickyScroll(ref);
  if (api) api.current = scroll;
  return <div ref={ref} data-testid="pane" data-tick={tick} onScroll={scroll.onScroll} />;
}

function metrics(pane: HTMLElement) {
  let top = 0;
  Object.defineProperty(pane, 'scrollTop', { configurable: true, get: () => top, set: (value: number) => { top = value; } });
  Object.defineProperty(pane, 'scrollHeight', { configurable: true, value: 400 });
  Object.defineProperty(pane, 'clientHeight', { configurable: true, value: 200 });
  return { get: () => top, set: (value: number) => { top = value; } };
}

it('keeps the user position on upward scroll instead of jumping to the top', () => {
  const view = render(<Probe tick={0} />);
  const pane = view.getByTestId('pane');
  const top = metrics(pane);

  view.rerender(<Probe tick={1} />);
  expect(top.get()).toBe(400);

  top.set(120);
  fireEvent.scroll(pane);
  view.rerender(<Probe tick={2} />);
  expect(top.get()).toBe(120);

  top.set(200);
  fireEvent.scroll(pane);
  view.rerender(<Probe tick={3} />);
  expect(top.get()).toBe(400);
});

it('jumps to an explicit offset and keeps it across streaming renders', () => {
  const api: { current: StickyScroll | null } = { current: null };
  const view = render(<Probe tick={0} api={api} />);
  const pane = view.getByTestId('pane');
  const top = metrics(pane);
  view.rerender(<Probe tick={1} api={api} />);
  expect(top.get()).toBe(400);

  act(() => api.current!.scrollToTop(80));
  expect(top.get()).toBe(80);
  view.rerender(<Probe tick={2} api={api} />);
  expect(top.get()).toBe(80);
});

it('releases following at wheel-up input time, before the async scroll event arrives', () => {
  // 真实竞态：wheel 输入同步派发 → 浏览器滚动 → scroll 事件异步派发。
  // 流式渲染抢在 scroll 事件前提交时，若跟随状态未在输入层脱离，
  // pin 会把位置拉回底部，迟到的 scroll 事件读到底部位置——滚动被永久吞掉。
  const view = render(<Probe tick={0} />);
  const pane = view.getByTestId('pane');
  const top = metrics(pane);
  view.rerender(<Probe tick={1} />);
  expect(top.get()).toBe(400);

  // 用户向上滚轮：输入事件同步到达，浏览器随即移动视口，scroll 事件尚未派发。
  fireEvent.wheel(pane, { deltaY: -120 });
  top.set(120);

  // 流式渲染抢先提交：pin 不得把位置拉回底部。
  view.rerender(<Probe tick={2} />);
  expect(top.get()).toBe(120);

  // 迟到的 scroll 事件确认脱离后，后续渲染同样不得回拉。
  fireEvent.scroll(pane);
  view.rerender(<Probe tick={3} />);
  expect(top.get()).toBe(120);
});

it('keeps following when the wheel-up happens over a nested scrollable child', () => {
  const view = render(<Probe tick={0} />);
  const pane = view.getByTestId('pane');
  const top = metrics(pane);
  view.rerender(<Probe tick={1} />);
  expect(top.get()).toBe(400);

  // 内部可滚动块（代码块/工具输出）：滚轮意在滚动它而非正文容器。
  const inner = document.createElement('pre');
  inner.style.overflowY = 'auto';
  pane.appendChild(inner);
  let innerTop = 10;
  Object.defineProperty(inner, 'scrollTop', { configurable: true, get: () => innerTop, set: (value: number) => { innerTop = value; } });
  Object.defineProperty(inner, 'scrollHeight', { configurable: true, value: 300 });
  Object.defineProperty(inner, 'clientHeight', { configurable: true, value: 100 });

  fireEvent.wheel(inner, { deltaY: -120 });
  view.rerender(<Probe tick={2} />);
  expect(top.get()).toBe(400);
});

it('keeps following when shrinking content clamps the scroll position', () => {
  const view = render(<Probe tick={0} />);
  const pane = view.getByTestId('pane');
  const top = metrics(pane);
  view.rerender(<Probe tick={1} />);
  expect(top.get()).toBe(400);

  // Content shrank: the browser clamps scrollTop to the new bottom (40).
  Object.defineProperty(pane, 'scrollHeight', { configurable: true, value: 240 });
  top.set(40);
  fireEvent.scroll(pane);

  // New content arrives; a false "user scrolled up" would restore 40.
  Object.defineProperty(pane, 'scrollHeight', { configurable: true, value: 500 });
  view.rerender(<Probe tick={2} />);
  expect(top.get()).toBe(500);
});
