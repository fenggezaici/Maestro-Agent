/// Chat scroll contract: follow new content only while the user is at the
/// bottom. A manual upward scroll pauses following, and the recorded position
/// is restored after every render — never `scrollTop = 0`.
import { useEffect, useLayoutEffect, useRef, type RefObject } from 'react';

export interface StickyScroll {
  onScroll: () => void;
  stickToBottom: () => void;
  reset: () => void;
  scrollToTop: (top: number) => void;
}

export function useStickyScroll<T extends HTMLElement>(
  ref: RefObject<T | null>,
  options: { threshold?: number; onNearBottom?: (near: boolean) => void } = {},
): StickyScroll {
  const { threshold = 40, onNearBottom } = options;
  const following = useRef(true);
  const lastTop = useRef(0);

  const pin = () => {
    const element = ref.current;
    if (!element) return;
    element.scrollTop = following.current ? element.scrollHeight : lastTop.current;
    // Record the clamped position immediately: scroll events are async, and an
    // upward wheel tick must be detected before the next render.
    if (following.current) lastTop.current = element.scrollTop;
  };

  useLayoutEffect(() => {
    pin();
    if (!following.current) return;
    // Batch tool nodes can land after the commit (markdown/images/transitions):
    // one more pin on the next frame keeps the bottom glued.
    const frame = requestAnimationFrame(pin);
    return () => cancelAnimationFrame(frame);
  });

  // Content height changes without a render (collapsible 200ms transition,
  // batch tool inserts, async media). Re-pin while following.
  useEffect(() => {
    const element = ref.current;
    const content = element?.firstElementChild;
    if (!element || !content || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => {
      if (following.current) pin();
    });
    observer.observe(content);
    return () => observer.disconnect();
  });

  const onScroll = () => {
    const element = ref.current;
    if (!element) return;
    const { scrollTop, scrollHeight, clientHeight } = element;
    const near = scrollHeight - scrollTop - clientHeight < threshold;
    // A shrinking list clamps scrollTop to the new maximum — that must not look
    // like the user scrolling up. Only leave the bottom when genuinely away.
    if (near) following.current = true;
    else if (scrollTop < lastTop.current) following.current = false;
    lastTop.current = scrollTop;
    onNearBottom?.(near);
  };

  const stickToBottom = () => {
    following.current = true;
    const element = ref.current;
    if (element) {
      // Compare future scroll events against the current position, not the
      // target: the smooth animation must never look like an upward scroll.
      lastTop.current = element.scrollTop;
      element.scrollTo({ top: element.scrollHeight, behavior: 'smooth' });
    }
    onNearBottom?.(true);
  };

  const reset = () => {
    following.current = true;
    lastTop.current = 0;
    const element = ref.current;
    if (element) element.scrollTop = element.scrollHeight;
    onNearBottom?.(true);
  };

  /// Jump to an explicit offset (turn navigator): stop following so streaming
  /// cannot pull the reader back to the bottom.
  const scrollToTop = (top: number) => {
    const element = ref.current;
    if (!element) return;
    following.current = false;
    lastTop.current = Math.max(0, Math.min(top, element.scrollHeight - element.clientHeight));
    element.scrollTop = lastTop.current;
    onNearBottom?.(element.scrollHeight - lastTop.current - element.clientHeight < threshold);
  };

  return { onScroll, stickToBottom, reset, scrollToTop };
}
