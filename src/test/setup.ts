import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterEach, vi } from 'vitest';

class TestPointerEvent extends MouseEvent {
  pointerType: string;
  constructor(type: string, options: PointerEventInit = {}) {
	super(type, options);
	this.pointerType = options.pointerType ?? 'mouse';
  }
}
vi.stubGlobal('PointerEvent', TestPointerEvent);
vi.stubGlobal('ResizeObserver', class {
  observe() {}
  unobserve() {}
  disconnect() {}
});
HTMLElement.prototype.scrollIntoView = vi.fn();
HTMLElement.prototype.hasPointerCapture = vi.fn(() => false);
HTMLElement.prototype.setPointerCapture = vi.fn();
HTMLElement.prototype.releasePointerCapture = vi.fn();

afterEach(cleanup);
