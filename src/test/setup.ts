import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterEach, beforeEach, vi } from 'vitest';

class TestPointerEvent extends MouseEvent {
  pointerType: string;
  pointerId: number;
  constructor(type: string, options: PointerEventInit = {}) {
	super(type, options);
	this.pointerType = options.pointerType ?? 'mouse';
	this.pointerId = options.pointerId ?? 0;
  }
}
vi.stubGlobal('PointerEvent', TestPointerEvent);
vi.stubGlobal('ResizeObserver', class {
  observe() {}
  unobserve() {}
  disconnect() {}
});
HTMLElement.prototype.scrollIntoView = vi.fn();
// jsdom has no layout engine; sticky-scroll and trace panes call scrollTo.
HTMLElement.prototype.scrollTo = vi.fn();
HTMLElement.prototype.hasPointerCapture = vi.fn(() => false);
HTMLElement.prototype.setPointerCapture = vi.fn();
HTMLElement.prototype.releasePointerCapture = vi.fn();

// View state persists to sessionStorage (reload survival); without a reset it
// leaks across tests in the same jsdom environment and restores stale panels.
beforeEach(() => { sessionStorage.clear(); });
afterEach(cleanup);
