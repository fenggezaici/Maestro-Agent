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
