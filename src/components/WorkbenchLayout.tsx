import { useEffect, useState, type ReactNode } from 'react';
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle, type Layout } from 'react-resizable-panels';

function breakpoint() {
  if (typeof window === 'undefined') return 'desktop';
  return window.innerWidth <= 650 ? 'mobile' : window.innerWidth <= 1040 ? 'stacked' : 'desktop';
}

function loadLayout(key: string, ids: string[]): Layout | undefined {
  try {
    const layout: unknown = JSON.parse(localStorage.getItem(key) ?? 'null');
    if (!layout || typeof layout !== 'object' || Array.isArray(layout)) return;
    const values = ids.map((id) => (layout as Layout)[id]);
    if (values.every((value) => Number.isFinite(value) && value >= 0 && value <= 100)
      && Math.abs(values.reduce((total, value) => total + value, 0) - 100) < 0.1) return layout as Layout;
  } catch { /* Ignore unavailable or invalid UI storage. */ }
}

function SavedGroup({ name, source, ids, orientation, children }: {
  name: string; source: string; ids: string[]; orientation: 'horizontal' | 'vertical'; children: ReactNode;
}) {
  const key = `maestro.panels.v1.${source}.${name}`;
  const [defaultLayout] = useState(() => loadLayout(key, ids));
  return <PanelGroup id={`panels-${name}`} orientation={orientation} defaultLayout={defaultLayout}
    className="workbench-panel-group" resizePreviewMode="panel" resizeTargetMinimumSize={{ fine: 8, coarse: 20 }}
    onLayoutChanged={(layout, meta) => {
      if (meta.isUserInteraction) {
        try { localStorage.setItem(key, JSON.stringify(meta.requestedLayout ?? layout)); } catch { /* UI preferences only. */ }
      }
    }}>{children}</PanelGroup>;
}

function Handle({ id, label }: { id: string; label: string }) {
  return <PanelResizeHandle id={id} aria-label={label} className="resizable-handle"><span /></PanelResizeHandle>;
}

export default function WorkbenchLayout({ sidebar, conversation, review, source }: {
  sidebar: ReactNode; conversation: ReactNode; review: ReactNode | null; source: 'live' | 'preview';
}) {
  const [mode, setMode] = useState(breakpoint);
  useEffect(() => {
    const update = () => setMode(breakpoint());
    window.addEventListener('resize', update);
    return () => window.removeEventListener('resize', update);
  }, []);

  if (mode === 'mobile') return <div className="workbench-mobile" data-layout="mobile">{sidebar}{conversation}{review}</div>;
  if (mode === 'stacked') return <div className="workbench-layout" data-layout="stacked">
    <SavedGroup key={`stacked-${!!review}`} name={`stacked-${!!review}`} source={source} ids={['sidebar', 'main']} orientation="horizontal">
      <Panel id="sidebar" defaultSize={320} minSize={180} maxSize="40%" groupResizeBehavior="preserve-pixel-size" className="panel-content">{sidebar}</Panel>
      <Handle id="resize-sidebar-stacked" label="调整侧栏宽度" />
      <Panel id="main" minSize={260} className="panel-content">
        <SavedGroup key={`rows-${!!review}`} name={`rows-${!!review}`} source={source} ids={review ? ['chat', 'review'] : ['chat']} orientation="vertical">
          <Panel id="chat" defaultSize={review ? '60%' : '100%'} minSize={280} className="panel-content">{conversation}</Panel>
          {review && <><Handle id="resize-review-stacked" label="调整对话与 Diff 高度" /><Panel id="review" minSize={180} className="panel-content">{review}</Panel></>}
        </SavedGroup>
      </Panel>
    </SavedGroup>
  </div>;

  return <div className="workbench-layout" data-layout="desktop">
    <SavedGroup key={`desktop-${!!review}`} name={`desktop-${!!review}`} source={source} ids={review ? ['sidebar', 'chat', 'review'] : ['sidebar', 'chat']} orientation="horizontal">
      <Panel id="sidebar" defaultSize={240} minSize={180} maxSize="35%" groupResizeBehavior="preserve-pixel-size" className="panel-content">{sidebar}</Panel>
      <Handle id="resize-sidebar" label="调整侧栏宽度" />
      <Panel id="chat" minSize={280} className="panel-content">{conversation}</Panel>
      {review && <><Handle id="resize-review" label="调整对话与 Diff 宽度" /><Panel id="review" defaultSize="45%" minSize={320} maxSize="65%" className="panel-content">{review}</Panel></>}
    </SavedGroup>
  </div>;
}


