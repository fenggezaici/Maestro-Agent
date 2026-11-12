import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react';

const SIDEBAR_KEY = 'maestro.sidebar.width.v1';
const REVIEW_KEY = 'maestro.review.width.v1';
const SIDEBAR_MIN = 220;
const SIDEBAR_MAX = 360;
const REVIEW_MIN = 320;

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

/// ZCode-style outer shell: plain flex + pixel widths + pointer drag. The
/// sidebar is a normal DOM node, so inserting/removing the review column never
/// remounts it or recalculates its width.
export default function WorkbenchLayout({ sidebar, conversation, review }: {
  sidebar: ReactNode; conversation: ReactNode; review: ReactNode | null; source: 'live' | 'preview';
}) {
  const [mode, setMode] = useState(breakpoint);
  const shellRef = useRef<HTMLDivElement>(null);
  const drag = useRef<{ kind: 'sidebar' | 'review'; startX: number; startWidth: number } | null>(null);
  const [sidebarWidth, setSidebarWidth] = useState(() => clamp(storedWidth(SIDEBAR_KEY, 280), SIDEBAR_MIN, SIDEBAR_MAX));
  const [reviewWidth, setReviewWidth] = useState(() => storedWidth(REVIEW_KEY, 0));

  useEffect(() => {
    const update = () => setMode(breakpoint());
    window.addEventListener('resize', update);
    return () => window.removeEventListener('resize', update);
  }, []);

  useEffect(() => {
    const shell = shellRef.current;
    if (!shell) return;
    shell.style.setProperty('--sidebar-width', `${sidebarWidth}px`);
    const fallbackReview = shell.getBoundingClientRect().width * 0.42;
    const width = clamp(reviewWidth || fallbackReview, REVIEW_MIN, shell.getBoundingClientRect().width * 0.65);
    if (!reviewWidth) setReviewWidth(width);
    shell.style.setProperty('--review-width', `${width}px`);
  }, [sidebarWidth, reviewWidth]);

  const start = (kind: 'sidebar' | 'review', event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.pointerType === 'mouse' && event.button !== 0) return;
    const shell = shellRef.current;
    if (!shell) return;
    const startWidth = kind === 'sidebar' ? sidebarWidth : reviewWidth;
    drag.current = { kind, startX: event.clientX, startWidth };
    event.currentTarget.setPointerCapture(event.pointerId);
    event.preventDefault();
  };

  const move = (event: ReactPointerEvent<HTMLDivElement>) => {
    const session = drag.current;
    const shell = shellRef.current;
    if (!session || !shell) return;
    const shellWidth = shell.getBoundingClientRect().width;
    const delta = event.clientX - session.startX;
    // Sidebar handle is on its right edge; review handle is on its left edge.
    const next = session.kind === 'sidebar' ? session.startWidth + delta : session.startWidth - delta;
    const width = session.kind === 'sidebar'
      ? clamp(next, SIDEBAR_MIN, SIDEBAR_MAX)
      : clamp(next, REVIEW_MIN, shellWidth * 0.65);
    shell.style.setProperty(session.kind === 'sidebar' ? '--sidebar-width' : '--review-width', `${width}px`);
  };

  const end = () => {
    const session = drag.current;
    const shell = shellRef.current;
    drag.current = null;
    if (!session || !shell) return;
    const raw = Number.parseFloat(shell.style.getPropertyValue(session.kind === 'sidebar' ? '--sidebar-width' : '--review-width'));
    if (!Number.isFinite(raw)) return;
    if (session.kind === 'sidebar') {
      setSidebarWidth(raw);
      try { localStorage.setItem(SIDEBAR_KEY, String(Math.round(raw))); } catch { /* UI only. */ }
    } else {
      setReviewWidth(raw);
      try { localStorage.setItem(REVIEW_KEY, String(Math.round(raw))); } catch { /* UI only. */ }
    }
  };

  if (mode === 'mobile') return <div className="workbench-mobile" data-layout="mobile">{sidebar}{conversation}{review}</div>;

  return <div className={`workbench-layout ${mode === 'stacked' ? 'is-stacked' : ''}`} data-layout={mode} ref={shellRef}>
    <aside className="panel-content sidebar-panel">{sidebar}</aside>
    <div className="resize-handle" role="separator" aria-orientation="vertical" aria-label="调整侧栏宽度"
      onPointerDown={(event) => start('sidebar', event)} onPointerMove={move} onPointerUp={end} onPointerCancel={end} />
    <main className="panel-content main-panel">
      {mode === 'stacked'
        ? <div className="stacked-rows">
            <div className="panel-content chat-panel">{conversation}</div>
            {review && <><div className="resize-handle is-horizontal" /><section className="panel-content review-panel is-stacked">{review}</section></>}
          </div>
        : conversation}
    </main>
    {mode === 'desktop' && review && <>
      <div className="resize-handle" role="separator" aria-orientation="vertical" aria-label="调整对话与 Diff 宽度"
        onPointerDown={(event) => start('review', event)} onPointerMove={move} onPointerUp={end} onPointerCancel={end} />
      <section className="panel-content review-panel">{review}</section>
    </>}
  </div>;
}
