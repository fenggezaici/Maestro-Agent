import { useCallback, useRef, useState, type KeyboardEvent, type PointerEvent, type RefObject } from 'react';

const STORAGE_KEY = 'maestro.right-list-width';
const MIN_LIST = 160;
const MIN_PREVIEW = 200;

/// ZCode WorkbenchSplitDivider-style resize: pointer moves are coalesced into
/// one CSS-variable write per animation frame; React state commits on release.
export default function SplitDivider({ containerRef, onCommit }: {
  containerRef: RefObject<HTMLDivElement | null>;
  onCommit: (width: number) => void;
}) {
  const drag = useRef<{ pointerId: number; startX: number; startWidth: number; latestX: number; raf: number | null } | null>(null);
  const [dragging, setDragging] = useState(false);

  const limits = useCallback(() => {
    const total = containerRef.current?.clientWidth ?? 0;
    const max = Math.max(MIN_LIST, Math.min(total * 0.5, total - MIN_PREVIEW - 4));
    return { min: MIN_LIST, max };
  }, [containerRef]);

  const apply = useCallback((clientX: number) => {
    const current = drag.current;
    if (!current) return;
    const { min, max } = limits();
    const width = Math.min(max, Math.max(min, current.startWidth + clientX - current.startX));
    containerRef.current?.style.setProperty('--right-list-width', `${width}px`);
  }, [containerRef, limits]);

  const schedule = useCallback((clientX: number) => {
    const current = drag.current;
    if (!current || current.raf !== null) return;
    current.latestX = clientX;
    current.raf = requestAnimationFrame(() => {
      if (!drag.current) return;
      drag.current.raf = null;
      apply(drag.current.latestX);
    });
  }, [apply]);

  const finish = useCallback((clientX?: number) => {
    const current = drag.current;
    if (!current) return;
    if (current.raf !== null) cancelAnimationFrame(current.raf);
    if (clientX !== undefined) apply(clientX);
    const list = containerRef.current?.querySelector<HTMLElement>('.right-list-pane');
    const width = Math.round(list?.getBoundingClientRect().width ?? current.startWidth);
    drag.current = null;
    setDragging(false);
    delete document.body.dataset.panelDragging;
    onCommit(width);
    window.dispatchEvent(new Event('maestro-panel-resize-end'));
  }, [apply, containerRef, onCommit]);

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    const list = containerRef.current?.querySelector<HTMLElement>('.right-list-pane');
    if (!list) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { pointerId: event.pointerId, startX: event.clientX, startWidth: list.getBoundingClientRect().width, latestX: event.clientX, raf: null };
    setDragging(true);
    document.body.dataset.panelDragging = 'true';
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (drag.current?.pointerId !== event.pointerId) return;
    schedule(event.clientX);
  };

  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    if (drag.current?.pointerId !== event.pointerId) return;
    finish(event.clientX);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const list = containerRef.current?.querySelector<HTMLElement>('.right-list-pane');
    if (!list) return;
    const { min, max } = limits();
    const current = list.getBoundingClientRect().width;
    const next = event.key === 'ArrowLeft' ? current - 16
      : event.key === 'ArrowRight' ? current + 16
      : event.key === 'Home' ? min
      : event.key === 'End' ? max
      : null;
    if (next === null) return;
    event.preventDefault();
    const width = Math.min(max, Math.max(min, next));
    containerRef.current?.style.setProperty('--right-list-width', `${width}px`);
    onCommit(Math.round(width));
  };

  return <div role="separator" aria-orientation="vertical" tabIndex={0} data-dragging={dragging}
    className="resizable-handle split-divider" onPointerDown={onPointerDown}
    onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerCancel={() => finish()}
    onKeyDown={onKeyDown}><span /></div>;
}

export { STORAGE_KEY };
