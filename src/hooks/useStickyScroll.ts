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
    if (!element || !following.current) return;
    // 不跟随时完全放手：React 重渲不会重置 scrollTop，内容上方位移由浏览器
    // 原生 scroll anchoring 补偿——手动 restore 是滚动吞没竞态的温床（它写出的
    // 位置会被迟到的 scroll 事件误读为用户意图）。
    element.scrollTop = element.scrollHeight;
    lastTop.current = element.scrollTop;
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

  // 输入层意图捕获：wheel/touch/key 输入事件同步派发——先于浏览器滚动与异步
  // scroll 事件脱离跟随，流式渲染的 pin 因此永远抢不过用户滚动（wheel 是离散
  // 输入，被吞后没有后续事件自愈；滚动条拖拽是连续输入，scroll 事件兜底）。
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    // 嵌套可滚动块（代码块/工具输出）会消费这次滚轮——容器位置并未改变，
    // 不算用户离开底部。
    const consumedByNested = (event: Event) => {
      let node = event.target as HTMLElement | null;
      while (node && node !== element) {
        if (/(auto|scroll)/.test(window.getComputedStyle(node).overflowY) && node.scrollHeight > node.clientHeight && node.scrollTop > 0) {
          return true;
        }
        node = node.parentElement;
      }
      return false;
    };
    const unfollow = () => {
      following.current = false;
      onNearBottom?.(false);
    };
    const onWheel = (event: WheelEvent) => {
      if (event.deltaY < 0 && element.scrollTop > 0 && !consumedByNested(event)) unfollow();
    };
    let touchY: number | null = null;
    const onTouchStart = (event: TouchEvent) => {
      touchY = event.touches[0]?.clientY ?? null;
    };
    const onTouchMove = (event: TouchEvent) => {
      // 手指下滑 = 内容向顶部方向回卷。
      const y = event.touches[0]?.clientY;
      if (touchY != null && y != null && y > touchY + 2 && element.scrollTop > 0) unfollow();
    };
    const onKeydown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (target?.closest('input, textarea, [contenteditable]')) return;
      if (event.key === 'PageUp' || event.key === 'ArrowUp' || event.key === 'Home') unfollow();
    };
    element.addEventListener('wheel', onWheel, { passive: true });
    element.addEventListener('touchstart', onTouchStart, { passive: true });
    element.addEventListener('touchmove', onTouchMove, { passive: true });
    element.addEventListener('keydown', onKeydown);
    return () => {
      element.removeEventListener('wheel', onWheel);
      element.removeEventListener('touchstart', onTouchStart);
      element.removeEventListener('touchmove', onTouchMove);
      element.removeEventListener('keydown', onKeydown);
    };
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
