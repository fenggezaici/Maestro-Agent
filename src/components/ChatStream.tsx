/// Dumb chat stream: renders conversation turns, tool-call blocks, reasoning
/// panes and plan cards. Everything arrives as props — no IPC, no stores.
import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import { BrainCircuit, Copy, Pencil, WandSparkles, Wrench } from 'lucide-react';

import type { PlanCard as Plan, PlanDecision, RunView, ToolCallView } from '../api/types';
import { relativeTime, type ReasoningSegment, type TimelineItem } from '../lib/conversation';
import type { UiPreferences } from '../uiPreferences';
import CollapsibleBlock from './CollapsibleBlock';
import PlanCard from './PlanCard';
import ToolCallBlocks, { ToolCallRow } from './ToolCallBlocks';
import MarkdownMessage from './MarkdownMessage';
import LocalAttachments, { type LocalAttachment } from './LocalAttachments';

/// Models without `reasoning_content` emit inline <think>…</think> blocks —
/// pull them into the reasoning pane instead of the answer body.
function extractReasoning(text: string): { reasoning: string; content: string } {
  const parts: string[] = [];
  const content = text.replace(/<think>([\s\S]*?)(?:<\/think>|$)/g, (_m, inner: string) => { parts.push(inner); return ''; });
  return { reasoning: parts.join('\n'), content };
}

/// Strip tool-call narration leaked into persisted output (pre-fix runs) or
/// emitted by non-function-calling models. Keeps the intentional
/// `> 工具调用预算已用尽` note.
function cleanRunOutput(text: string): string {
  return text.replace(/(^|\n)> (?:调用工具|工具调用（?)：?(?!预算已用尽)[^\n]*/g, '$1').trimStart();
}

/// Reasoning body: the `<p>` is its own scroll container (max-height 250px).
/// Follow new tokens while streaming, but stop fighting the user the moment
/// they scroll up — resuming once they are back at the bottom.
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

/// One thinking burst in the timeline: expanded and auto-following while it is
/// live, collapsed to a single row (`思考 · 持续了 X 秒`) once it ends.
function ReasoningNode({ segment, display }: { segment: ReasoningSegment; display: UiPreferences['reasoningDisplay'] }) {
  if (display === 'hidden') return null;
  const active = segment.endedAt === undefined;
  const seconds = active ? 0 : Math.max(1, Math.round((segment.endedAt! - segment.startedAt) / 1000));
  return <CollapsibleBlock title={active ? '思考 · 进行中…' : `思考 · 持续了 ${seconds} 秒`} icon={<BrainCircuit size={13} />}
    open={active ? true : undefined} contentClass="reasoning-content"><ReasoningText text={segment.text} streaming={active} /></CollapsibleBlock>;
}

const CARD_STATUS: Record<RunView['status'], string> = { running: '进行中', completed: '已完成', cancelled: '已停止', failed: '失败' };

/// Dark preview card for one turn: role badges, relative time, prompt and
/// answer snippets, plus the last tool call. Rendered next to the rail.
function TurnCard({ run, calls, preview }: { run: RunView; calls: ToolCallView[]; preview: boolean }) {
  const answer = cleanRunOutput(extractReasoning(run.output || '').content).trim();
  const lastTool = calls[calls.length - 1]?.tool;
  return <>
    <div className="turn-card-head">
      <span className="turn-role user">You</span>
      <span className="turn-arrow">→</span>
      <span className="turn-role agent">Maestro</span>
      <span className="turn-time">{preview ? '示例' : relativeTime(run.createdAt)}</span>
    </div>
    <p className="turn-summary">{run.prompt || '（无提问）'}</p>
    {answer && <p className="turn-answer">{answer}</p>}
    <div className="turn-card-foot">
      <span className={`turn-state ${run.status}`}>{CARD_STATUS[run.status]}</span>
      {lastTool && <span className="turn-tool"><Wrench size={10} />{lastTool}</span>}
    </div>
  </>;
}

export default function ChatStream({ historyRef, onScroll, runs, loadingSession, detailTools, liveTools, liveTimeline, plans, reasoningDisplay, theme, preview, messageAttachments, pending, onCopyError, onReviewPlan, onDecidePlan, onSelectFile, onEditPrompt, onJumpToTurn }: {
  historyRef: RefObject<HTMLDivElement | null>;
  onScroll: () => void;
  runs: RunView[];
  loadingSession: boolean;
  detailTools: ToolCallView[] | undefined;
  liveTools?: Record<number, ToolCallView[]>;
  liveTimeline?: Record<number, TimelineItem[]>;
  plans: Plan[];
  reasoningDisplay: UiPreferences['reasoningDisplay'];
  theme: string;
  preview: boolean;
  messageAttachments: Record<number, LocalAttachment[]>;
  pending: boolean;
  onCopyError: (message: string) => void;
  onReviewPlan: (id: number) => void;
  onDecidePlan: (plan: Plan, decision: PlanDecision) => void;
  onSelectFile?: (path: string) => void;
  onEditPrompt?: (text: string) => void;
  onJumpToTurn?: (runId: number) => void;
}) {
  const railRef = useRef<HTMLElement>(null);
  const [activeRunId, setActiveRunId] = useState<number | null>(null);
  const [card, setCard] = useState<{ runId: number; top: number } | null>(null);
  const [flashRunId, setFlashRunId] = useState<number | null>(null);
  const flashTimer = useRef<number | undefined>(undefined);
  const turnKey = runs.map((run) => run.id).join(',');

  const showCard = (runId: number, element: HTMLElement) => {
    const shell = element.closest('.chat-history-shell');
    const shellRect = shell?.getBoundingClientRect();
    const rect = element.getBoundingClientRect();
    const top = shellRect
      ? Math.max(8, Math.min(rect.top - shellRect.top + rect.height / 2 - 36, shellRect.height - 130))
      : 0;
    setCard({ runId, top });
  };

  const flashTurn = (runId: number) => {
    setFlashRunId(runId);
    if (flashTimer.current) window.clearTimeout(flashTimer.current);
    flashTimer.current = window.setTimeout(() => setFlashRunId(null), 1200);
  };

  useEffect(() => () => { if (flashTimer.current) window.clearTimeout(flashTimer.current); }, []);

  // Current turn = the last one whose top has crossed the viewport top.
  useEffect(() => {
    const root = historyRef.current;
    if (!root || !turnKey) return;
    let frame = 0;
    const update = () => {
      const rootTop = root.getBoundingClientRect().top;
      const nodes = root.querySelectorAll<HTMLElement>('[data-turn-id]');
      let active: number | null = null;
      for (const node of nodes) {
        if (node.getBoundingClientRect().top <= rootTop + 48) active = Number(node.dataset.turnId);
        else break;
      }
      if (active === null && nodes.length) active = Number(nodes[0].dataset.turnId);
      setActiveRunId((current) => (current === active ? current : active));
    };
    const handleScroll = () => {
      if (frame) return;
      frame = requestAnimationFrame(() => { frame = 0; update(); });
    };
    update();
    root.addEventListener('scroll', handleScroll, { passive: true });
    return () => {
      if (frame) cancelAnimationFrame(frame);
      root.removeEventListener('scroll', handleScroll);
    };
  }, [historyRef, turnKey]);

  // Keep the active tick visible when a long conversation scrolls.
  useEffect(() => {
    const rail = railRef.current;
    const active = rail?.querySelector<HTMLElement>('[aria-current="location"]');
    if (!rail || !active) return;
    const top = active.offsetTop;
    const bottom = top + active.offsetHeight;
    if (top < rail.scrollTop) rail.scrollTop = top;
    else if (bottom > rail.scrollTop + rail.clientHeight) rail.scrollTop = bottom - rail.clientHeight;
  }, [activeRunId]);

  return <div className={`chat-history-shell ${runs.length > 1 ? 'has-rail' : ''}`}>
    {runs.length > 1 && <>
      <nav ref={railRef} className="chat-turn-rail" aria-label="对话轮次导航">
        {runs.map((run, index) => <button key={run.id} type="button" className="chat-turn-tick"
          aria-current={activeRunId === run.id ? 'location' : undefined}
          aria-label={`跳转到第 ${index + 1} 轮：${run.prompt.replace(/\s+/g, ' ').slice(0, 80)}`}
          onMouseEnter={(event) => showCard(run.id, event.currentTarget)}
          onMouseLeave={() => setCard(null)}
          onFocus={(event) => showCard(run.id, event.currentTarget)}
          onBlur={() => setCard(null)}
          onClick={() => { flashTurn(run.id); onJumpToTurn?.(run.id); }}><span aria-hidden="true" /></button>)}
      </nav>
      {card && (() => {
        const run = runs.find((item) => item.id === card.runId);
        if (!run) return null;
        const calls = liveTools?.[run.id] ?? (detailTools ?? []).filter((call) => call.runId === run.id);
        return <div className="turn-hover-card" role="tooltip" style={{ top: card.top }}>
          <span className="turn-card-arrow" aria-hidden="true" />
          <TurnCard run={run} calls={calls} preview={preview} />
        </div>;
      })()}
    </>}
    <div ref={historyRef} className="chat-history" aria-label="对话消息" onScroll={onScroll}>
      <div className="chat-history-inner">
        {runs.length === 0 && <div className="conversation-welcome"><span className="welcome-icon"><WandSparkles size={27} strokeWidth={1.2} /></span><h3>What should we build?</h3><p>描述你的任务。Agent 会先规划，再提交可审查的变更。</p><div className="welcome-suggestions">重构模块<span>·</span>排查问题<span>·</span>审查代码</div></div>}
        {loadingSession && runs.length === 0 && <p className="loading-note">正在加载会话…</p>}
        {runs.map((run) => <section key={run.id} className={`conversation-turn ${flashRunId === run.id ? 'turn-flash' : ''}`} data-turn-id={run.id} aria-label={`对话轮次 ${run.id}`}>
          <div className="user-message-group">
            <article className="user-message"><div className="message-meta"><span className="user-avatar">Y</span><strong>You</strong><span>{preview ? '11:42 AM' : `#${run.id}`}</span></div><p>{run.prompt}</p><LocalAttachments items={messageAttachments[run.id] ?? []} sent /></article>
            <div className="user-message-actions">
              <button type="button" className="message-action" aria-label={`复制用户消息 ${run.id}`} onClick={async () => { try { await navigator.clipboard.writeText(run.prompt); } catch { onCopyError('Cannot access clipboard'); } }}><Copy size={12} /></button>
              {onEditPrompt && <button type="button" className="message-action" aria-label={`编辑用户消息 ${run.id}`} onClick={() => onEditPrompt(run.prompt)}><Pencil size={12} /></button>}
            </div>
          </div>
        {(() => {
          const { reasoning: inlineReasoning, content } = extractReasoning(run.output || '');
          const body = cleanRunOutput(content);
          const liveCalls = liveTools?.[run.id] ?? [];
          const calls = liveCalls.length ? liveCalls : (detailTools ?? []).filter((call) => call.runId === run.id);
          const timeline = liveTimeline?.[run.id];
          // Historical runs have no persisted segmentation: fall back to the
          // audit tool list plus one inline-thinking block.
          return <>
            {timeline?.length
              ? <div className="reasoning-timeline" aria-label={`思考与工具时间线 ${run.id}`}>
                {timeline.map((item, index) => {
                  if (item.kind === 'reasoning') return <ReasoningNode key={`r${index}`} segment={item.segment} display={reasoningDisplay} />;
                  const call = liveCalls[item.index];
                  return call ? <div key={`t${index}`} className="tool-events" role="list" aria-label="工具调用明细"><ToolCallRow call={call} onSelectFile={onSelectFile} /></div> : null;
                })}
              </div>
              : <>
                {calls.length > 0 && <ToolCallBlocks calls={calls} onSelectFile={onSelectFile} />}
                {inlineReasoning && reasoningDisplay !== 'hidden' &&
                  <CollapsibleBlock title="思考" icon={<BrainCircuit size={13} />} contentClass="reasoning-content"><ReasoningText text={inlineReasoning} streaming={false} /></CollapsibleBlock>}
              </>}
            {(body !== '' || run.status !== 'running') &&
              <div aria-label={`回复内容 ${run.id}`} aria-busy={run.status === 'running'} className="agent-copy"><MarkdownMessage content={body || '无输出'} light={theme === 'light'} />{run.status === 'running' && <span aria-hidden="true" className="typing-cursor" />}</div>}
          </>;
        })()}
        <article aria-label={`Agent 回复 ${run.id}`} className="agent-message">
          {run.error && <p className="error-message">{run.error}</p>}
          {plans.filter((plan) => plan.runId === run.id).map((plan) => <PlanCard key={plan.id} plan={plan} preview={preview} canDecide={!preview && run.status === 'completed'} pending={pending} onReview={onReviewPlan} onDecision={(_id, decision) => { if (!preview) onDecidePlan(plan, decision); }} />)}
        </article>
      </section>)}
      </div>
    </div>
  </div>;
}
