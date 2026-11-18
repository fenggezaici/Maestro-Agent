import { useCallback, useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react';

const SIDEBAR_KEY = 'maestro.sidebar.width.v1';
const REVIEW_KEY = 'maestro.review.width.v1';
const SIDEBAR_MIN = 220;
const SIDEBAR_MAX = 360;
const REVIEW_MIN = 320;
const KEYBOARD_STEP = 16;
const REVIEW_MAX_RATIO = 0.65;

type Pane = 'sidebar' | 'review';

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

function storedWidth(key: string, fallback: number) {
  try {
    const value = Number(localStorage.getItem(key));
    return Number.isFinite(value) && value > 0 ? value : fallback;
  } catch { return fallback; }
}

function breakpoint() {
  if (typeof window === 'undefined') return 'desktop';
  return window.innerWidth <= 650 ? 'mobile' : window.innerWidth <= 1040 ? 'stacked' : 'desktop';
}

/// Outer shell: plain flex + pixel widths + pointer drag, plus a keyboard/ARIA
/// contract on each separator (Arrow/Home/End, aria-valuenow).
///
/// Alignment notes (see D:\zcode-ref): ZCode's splitter caches the container
/// width at pointerdown and writes the CSS variable directly during the drag
/// (WorkspaceShellLayout.tsx:1045-1061) — we do the same. The rAF coalescing is
/// borrowed from its scrollbar handler (:683-692), not from the splitter.
///
/// Layout is never read during render: the shell width lives in state (updated
/// by a ResizeObserver), so a keystroke cannot force a synchronous layout.
export default function WorkbenchLayout({ sidebar, conversation, review }: {
  sidebar: ReactNode; conversation: ReactNode; review: ReactNode | null; source: 'live' | 'preview';
}) {
  const [mode, setMode] = useState(breakpoint);
  const shellRef = useRef<HTMLDivElement>(null);
  const drag = useRef<{ pane: Pane; startX: number; startWidth: number; pointerId: number } | null>(null);
  const [sidebarWidth, setSidebarWidth] = useState(() => clamp(storedWidth(SIDEBAR_KEY, 280), SIDEBAR_MIN, SIDEBAR_MAX));
  const [reviewWidth, setReviewWidth] = useState(() => storedWidth(REVIEW_KEY, 0));
  const [shellWidth, setShellWidth] = useState(0);
  const shellWidthRef = useRef(0);
  const pendingPaint = useRef<{ pane: Pane; width: number } | null>(null);
  const frame = useRef<number | null>(null);

  const measure = useCallback(() => {
    const shell = shellRef.current;
    if (!shell) return;
    const width = shell.getBoundingClientRect().width;
    shellWidthRef.current = width;
    setShellWidth((current) => (Math.abs(current - width) < 0.5 ? current : width));
  }, []);

  useEffect(() => {
    const update = () => setMode(breakpoint());
    window.addEventListener('resize', update);
    return () => window.removeEventListener('resize', update);
  }, []);

  useEffect(() => {
    measure();
    const shell = shellRef.current;
    if (!shell) return;
    const observer = new ResizeObserver(() => measure());
    observer.observe(shell);
    return () => observer.disconnect();
  }, [measure, mode]);

  const bounds = useCallback((pane: Pane, width: number) => {
    const min = pane === 'sidebar' ? SIDEBAR_MIN : REVIEW_MIN;
    const max = pane === 'sidebar'
      ? SIDEBAR_MAX
      : Math.max(REVIEW_MIN, width * REVIEW_MAX_RATIO);
    return { min, max };
  }, []);

  const paint = useCallback((pane: Pane, width: number) => {
    const shell = shellRef.current;
    if (!shell) return;
    const { min, max } = bounds(pane, shellWidthRef.current);
    shell.style.setProperty(
      pane === 'sidebar' ? '--sidebar-width' : '--review-width',
      `${clamp(Math.round(width), min, max)}px`,
    );
  }, [bounds]);

  const commit = useCallback((pane: Pane, width: number) => {
    const { min, max } = bounds(pane, shellWidthRef.current);
    const next = clamp(Math.round(width), min, max);
    paint(pane, next);
    if (pane === 'sidebar') setSidebarWidth(next);
    else setReviewWidth(next);
    try { localStorage.setItem(pane === 'sidebar' ? SIDEBAR_KEY : REVIEW_KEY, String(next)); } catch { /* UI only. */ }
  }, [bounds, paint]);

  useEffect(() => {
    const shell = shellRef.current;
    if (!shell) return;
    shell.style.setProperty('--sidebar-width', `${sidebarWidth}px`);
    const fallbackReview = shellWidthRef.current * 0.42;
    const width = clamp(reviewWidth || fallbackReview, REVIEW_MIN, shellWidthRef.current * REVIEW_MAX_RATIO);
    if (!reviewWidth) setReviewWidth(width);
    shell.style.setProperty('--review-width', `${width}px`);
  }, [sidebarWidth, reviewWidth]);

  // Shrinking the window only clamps the rendered width; the user's saved
  // width stays intact so reopening the panel restores their intent.
  useEffect(() => {
    const shell = shellRef.current;
    if (!shell || mode === 'mobile' || shellWidth <= 0) return;
    const { min, max } = bounds('review', shellWidth);
    const width = clamp(reviewWidth || shellWidth * 0.42, min, max);
    shell.style.setProperty('--review-width', `${width}px`);
  }, [mode, reviewWidth, shellWidth, bounds]);

  const schedulePaint = (pane: Pane, width: number) => {
    pendingPaint.current = { pane, width };
    if (frame.current !== null) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = null;
      const pending = pendingPaint.current;
      pendingPaint.current = null;
      if (pending) paint(pending.pane, pending.width);
    });
  };

  const start = (pane: Pane, event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.pointerType === 'mouse' && event.button !== 0) return;
    if (!shellRef.current) return;
    drag.current = {
      pane,
      startX: event.clientX,
      startWidth: pane === 'sidebar' ? sidebarWidth : reviewWidth,
      pointerId: event.pointerId,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
    event.preventDefault();
  };

  const move = (event: ReactPointerEvent<HTMLDivElement>) => {
    const session = drag.current;
    if (!session || !shellRef.current || event.pointerId !== session.pointerId) return;
    const delta = event.clientX - session.startX;
    // Sidebar handle is on its right edge; review handle is on its left edge.
    schedulePaint(session.pane, session.pane === 'sidebar' ? session.startWidth + delta : session.startWidth - delta);
  };

  const end = (event: ReactPointerEvent<HTMLDivElement>, cancelled = false) => {
    const session = drag.current;
    const shell = shellRef.current;
    if (!session || !shell || event.pointerId !== session.pointerId) return;
    drag.current = null;
    if (frame.current !== null) {
      cancelAnimationFrame(frame.current);
      frame.current = null;
    }
    const pending = pendingPaint.current;
    pendingPaint.current = null;
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    if (cancelled) {
      commit(session.pane, session.startWidth);
      return;
    }
    const painted = Number.parseFloat(
      shell.style.getPropertyValue(session.pane === 'sidebar' ? '--sidebar-width' : '--review-width'),
    );
    const target = pending?.pane === session.pane ? pending.width : painted;
    commit(session.pane, Number.isFinite(target) ? target : session.startWidth);
  };

  const onKeyDown = (pane: Pane) => (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const current = pane === 'sidebar' ? sidebarWidth : reviewWidth;
    // The sidebar handle sits on its right edge, the review handle on its left,
    // so the same arrow grows the pane the pointer is moving toward.
    const grow = pane === 'sidebar' ? KEYBOARD_STEP : -KEYBOARD_STEP;
    let next: number | null = null;
    if (event.key === 'ArrowLeft') next = current - grow;
    else if (event.key === 'ArrowRight') next = current + grow;
    else if (event.key === 'Home') next = bounds(pane, shellWidth).min;
    else if (event.key === 'End') next = bounds(pane, shellWidth).max;
    if (next === null) return;
    event.preventDefault();
    commit(pane, next);
  };

  const separatorProps = (pane: Pane, label: string) => ({
    role: 'separator' as const,
    tabIndex: 0,
    'aria-orientation': 'vertical' as const,
    'aria-label': label,
    'aria-controls': pane === 'sidebar' ? 'sidebar-panel' : 'review-panel',
    'aria-valuemin': bounds(pane, shellWidth).min,
    'aria-valuemax': Math.round(bounds(pane, shellWidth).max),
    'aria-valuenow': Math.round(pane === 'sidebar' ? sidebarWidth : reviewWidth),
    onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) => start(pane, event),
    onPointerMove: move,
    onPointerUp: (event: ReactPointerEvent<HTMLDivElement>) => end(event),
    onPointerCancel: (event: ReactPointerEvent<HTMLDivElement>) => end(event, true),
    onKeyDown: onKeyDown(pane),
  });

  if (mode === 'mobile') return <div className="workbench-mobile" data-layout="mobile">{sidebar}{conversation}{review}</div>;

  return <div className={`workbench-layout ${mode === 'stacked' ? 'is-stacked' : ''}`} data-layout={mode} ref={shellRef}>
    <aside id="sidebar-panel" className="panel-content sidebar-panel">{sidebar}</aside>
    <div className="resize-handle" data-pane="sidebar" {...separatorProps('sidebar', '调整侧栏宽度')} />
    <main className="panel-content main-panel">
      {mode === 'stacked'
        ? <div className="stacked-rows">
            <div className="panel-content chat-panel">{conversation}</div>
            {/* Stacked panes have a fixed 42% review height; the old 4px strip
                looked draggable but had no handler, so it is a plain divider. */}
            {review && <><div className="stacked-divider" aria-hidden="true" /><section className="panel-content review-panel is-stacked">{review}</section></>}
          </div>
        : conversation}
    </main>
    {mode === 'desktop' && review && <>
      <div className="resize-handle" data-pane="review" {...separatorProps('review', '调整对话与 Diff 宽度')} />
      <section id="review-panel" className="panel-content review-panel">{review}</section>
    </>}
  </div>;
}
