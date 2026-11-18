import { useEffect, useRef } from 'react';
import { ArrowDown, Square } from 'lucide-react';
import type { SubagentView } from '../api/types';
import type { UiPreferences } from '../uiPreferences';
import MarkdownMessage from './MarkdownMessage';
import { ToolCallRow } from './ToolCallBlocks';
import SubagentReasoning from './SubagentReasoning';
import { useStickyScroll } from '../hooks/useStickyScroll';

/// Provider-reported decode speed only: no estimates when timing is missing.
/// DSH 口径：Σ outputTokens / Σ decode 窗口（首 delta → 末 delta），不含
/// 工具执行时间；缺 decodeMs 的旧行回退 elapsed - firstToken 近似。
function subagentSpeed(trace: SubagentView): string | null {
  // 优先成对的 decodeTokens/decodeMs；旧行没有 decodeTokens 时回退 outputTokens。
  const output = (trace.decodeTokens != null && trace.decodeTokens > 0)
    ? trace.decodeTokens
    : trace.outputTokens ?? null;
  if (output == null || output <= 0) return null;
  const decodeMs = trace.decodeMs != null && trace.decodeMs > 0
    ? trace.decodeMs
    : trace.elapsedMs != null && trace.firstTokenMs != null
      ? trace.elapsedMs - trace.firstTokenMs
      : 0;
  if (decodeMs <= 0) return null;
  return (output / (decodeMs / 1000)).toFixed(1);
}

const STATUS_LABEL: Record<SubagentView['status'], string> = {
  running: '执行中',
  completed: '已完成',
  failed: '失败',
  cancelled: '已取消',
};

/// Read-only drill-down for one child trace: header + prompt, then the same
/// interleaved reasoning/tool/content timeline used by the main chat.
export default function SubagentTracePanel({ trace, light, reasoningDisplay, onOpenFile, onCancel }: {
  trace: SubagentView;
  light: boolean;
  reasoningDisplay: UiPreferences['reasoningDisplay'];
  onOpenFile?: (path: string) => void;
  onCancel?: (id: string) => void;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const scroll = useStickyScroll(scrollRef, { threshold: 40 });
  const timeline = trace.timeline ?? [];
  useEffect(() => {
    if (trace.status === 'running') scroll.stickToBottom();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only new items should follow.
  }, [timeline.length, trace.status]);

  return <div className="subagent-trace" aria-label={`子智能体 ${trace.agent}`}>
    <header className="subagent-trace-header">
      <div className="subagent-trace-title"><strong>{trace.agent}</strong><span className={`subagent-status ${trace.status}`}>{STATUS_LABEL[trace.status]}</span></div>
      <p>{trace.description}</p>
      <div className="subagent-meta">
        <span>{trace.toolCount} 次工具</span>
        {trace.model && <span>{trace.model}</span>}
        {trace.inputTokens != null && <span>in {trace.inputTokens}</span>}
        {trace.outputTokens != null && <span>out {trace.outputTokens}</span>}
        {trace.reasoningTokens != null && <span>think {trace.reasoningTokens}</span>}
        {subagentSpeed(trace) && <span>{subagentSpeed(trace)} tok/s</span>}
        {trace.firstTokenMs != null && <span>TTFT {(trace.firstTokenMs / 1000).toFixed(1)}s</span>}
        {trace.status === 'running' && onCancel && <button type="button" className="button-ghost" onClick={() => onCancel(trace.id)}><Square size={11} />停止</button>}
      </div>
    </header>
    <details className="subagent-section"><summary>Prompt</summary><pre className="subagent-pre">{trace.prompt}</pre></details>
    <div className="subagent-trace-scroll" ref={scrollRef} onScroll={scroll.onScroll}>
      {timeline.length > 0
        ? timeline.map((item, index) => {
          if (item.kind === 'reasoning') return <SubagentReasoning key={`r-${index}`} segment={item.segment} display={reasoningDisplay} />;
          if (item.kind === 'content') return <div key={`c-${index}`} className="agent-copy"><MarkdownMessage content={item.text} light={light} /></div>;
          const call = trace.tools[item.index];
          return call ? <div key={`t-${index}`} className="tool-events" role="list"><ToolCallRow call={call} onOpenFile={onOpenFile} /></div> : null;
        })
        : <>
          {trace.tools.length > 0 && <div className="tool-events" role="list">{trace.tools.map((call, index) => <ToolCallRow key={`${call.tool}-${index}`} call={call} onOpenFile={onOpenFile} />)}</div>}
          {trace.content.trim() ? <div className="agent-copy"><MarkdownMessage content={trace.content} light={light} /></div> : <p className="subagent-empty">暂无输出</p>}
        </>}
      {trace.error && <p role="alert" className="subagent-error">{trace.error}</p>}
    </div>
    <button type="button" className="subagent-scroll-fab" aria-label="回到底部" onClick={scroll.stickToBottom}><ArrowDown size={14} /></button>
  </div>;
}
