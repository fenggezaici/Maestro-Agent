import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { BrainCircuit, ChevronDown, ChevronRight } from 'lucide-react';
import type { ReasoningSegment } from '../lib/conversation';
import type { UiPreferences } from '../uiPreferences';

function ReasoningText({ text, streaming }: { text: string; streaming: boolean }) {
  const ref = useRef<HTMLParagraphElement>(null);
  const stick = useRef(true);
  useLayoutEffect(() => {
    const element = ref.current;
    if (element && streaming && stick.current) element.scrollTop = element.scrollHeight;
  }, [text, streaming]);
  return <p ref={ref} onScroll={(event) => {
    const element = event.currentTarget;
    stick.current = element.scrollHeight - element.scrollTop - element.clientHeight < 24;
  }}>{text}</p>;
}

/// One thinking burst inside a subagent trace, matching the main chat node.
export default function SubagentReasoning({ segment, display }: { segment: ReasoningSegment; display: UiPreferences['reasoningDisplay'] }) {
  const active = segment.endedAt === undefined;
  const seconds = active ? 0 : Math.max(1, Math.round((segment.endedAt! - segment.startedAt) / 1000));
  const [open, setOpen] = useState(active);
  const manual = useRef(false);
  useEffect(() => {
    if (manual.current) return;
    setOpen(active);
  }, [active]);
  if (display === 'hidden') return null;
  return <div className="execution-reasoning">
    <button type="button" className="execution-row reasoning-toggle" aria-expanded={open}
      onClick={() => { manual.current = true; setOpen((value) => !value); }}>
      <BrainCircuit size={12} /><span>{active ? '思考 · 进行中…' : `思考 · ${seconds} 秒`}</span>
      {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
    </button>
    {open && <div className="reasoning-content execution-reasoning-body"><ReasoningText text={segment.text} streaming={active} /></div>}
  </div>;
}
