/// Dumb chat stream: renders conversation turns, tool-call blocks, reasoning
/// panes and plan cards. Everything arrives as props — no IPC, no stores.
import type { RefObject } from 'react';
import { BrainCircuit, Copy, ShieldCheck, Sparkles, WandSparkles } from 'lucide-react';

import type { PlanCard as Plan, PlanDecision, RunStatus, RunView, ToolCallView } from '../api/types';
import type { UiPreferences } from '../uiPreferences';
import CollapsibleBlock from './CollapsibleBlock';
import PlanCard from './PlanCard';
import ToolCallBlocks from './ToolCallBlocks';
import MarkdownMessage from './MarkdownMessage';
import LocalAttachments, { type LocalAttachment } from './LocalAttachments';

const statusLabels: Record<RunStatus, string> = { running: '正在生成', completed: '已完成', cancelled: '已停止', failed: '失败' };

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

export default function ChatStream({ historyRef, onScroll, runs, loadingSession, detailTools, liveTools, liveReasoning, liveReasoningMs, plans, reasoningDisplay, theme, preview, messageAttachments, pending, onCopyError, onReviewPlan, onDecidePlan, onResolveApproval }: {
  historyRef: RefObject<HTMLDivElement | null>;
  onScroll: () => void;
  runs: RunView[];
  loadingSession: boolean;
  detailTools: ToolCallView[] | undefined;
  liveTools?: Record<number, ToolCallView[]>;
  liveReasoning?: Record<number, string>;
  liveReasoningMs?: Record<number, number>;
  plans: Plan[];
  reasoningDisplay: UiPreferences['reasoningDisplay'];
  theme: string;
  preview: boolean;
  messageAttachments: Record<number, LocalAttachment[]>;
  pending: boolean;
  onCopyError: (message: string) => void;
  onReviewPlan: (id: number) => void;
  onDecidePlan: (plan: Plan, decision: PlanDecision) => void;
  onResolveApproval?: (runId: number, approved: boolean) => void;
}) {
  return <div ref={historyRef} className="chat-history" aria-label="对话消息" onScroll={onScroll}>
    <div className="chat-history-inner">
      {runs.length === 0 && <div className="conversation-welcome"><span className="welcome-icon"><WandSparkles size={27} strokeWidth={1.2} /></span><h3>What should we build?</h3><p>描述你的任务。Agent 会先规划，再提交可审查的变更。</p><div className="welcome-suggestions">重构模块<span>·</span>排查问题<span>·</span>审查代码</div></div>}
      {loadingSession && runs.length === 0 && <p className="loading-note">正在加载会话…</p>}
      {runs.map((run) => <section key={run.id} className="conversation-turn" aria-label={`对话轮次 ${run.id}`}>
        <article className="user-message"><div className="message-meta"><span className="user-avatar">Y</span><strong>You</strong><button type="button" className="user-copy-button" aria-label={`复制用户消息 ${run.id}`} onClick={async () => { try { await navigator.clipboard.writeText(run.prompt); } catch { onCopyError('Cannot access clipboard'); } }}><Copy size={12} /></button><span>{preview ? '11:42 AM' : `#${run.id}`}</span></div><p>{run.prompt}</p><LocalAttachments items={messageAttachments[run.id] ?? []} sent /></article>
        {(() => { const live = liveTools?.[run.id] ?? [];
          // Live event rows already cover this run's calls; the DB audit
          // snapshot would duplicate them, so prefer live whenever present.
          const calls = live.length ? live : (detailTools ?? []).filter((call) => call.runId === run.id);
          return calls.length > 0 && <ToolCallBlocks calls={calls} onResolveApproval={preview ? undefined : onResolveApproval} />; })()}
        {(() => {
          const { reasoning: inlineReasoning, content } = extractReasoning(run.output || '');
          const reasoning = (liveReasoning?.[run.id] ?? '') + inlineReasoning;
          const body = cleanRunOutput(content);
          const thinking = run.status === 'running' && body === '';
          const elapsed = liveReasoningMs?.[run.id];
          const title = thinking ? '思考过程 · 思考中…' : elapsed !== undefined ? `思考过程 · 耗时 ${Math.max(1, Math.round(elapsed / 1000))} 秒` : '思考过程';
          return <>
            {reasoning && reasoningDisplay !== 'hidden' &&
              <CollapsibleBlock title={title} icon={<BrainCircuit size={13} />} open={thinking ? true : undefined} defaultOpen={reasoningDisplay === 'summary'}
                contentClass={`reasoning-content ${reasoningDisplay === 'summary' ? 'reasoning-clamp' : ''}`}><p>{reasoning}</p></CollapsibleBlock>}
            {(body !== '' || run.status !== 'running') &&
              <div aria-label={`回复内容 ${run.id}`} aria-busy={run.status === 'running'} className="agent-copy"><MarkdownMessage content={body || '无输出'} light={theme === 'light'} />{run.status === 'running' && <span aria-hidden="true" className="typing-cursor" />}</div>}
          </>;
        })()}
        <article aria-label={`Agent 回复 ${run.id}`} className="agent-message"><div className="message-meta"><span className="agent-avatar"><Sparkles size={14} strokeWidth={1.5} /></span><strong>Maestro</strong><span className="message-model">Agent</span><button type="button" className="user-copy-button" aria-label={`复制回复 ${run.id}`} onClick={async () => { try { await navigator.clipboard.writeText(run.output); } catch { onCopyError('Cannot access clipboard'); } }}><Copy size={12} /></button><span>{statusLabels[run.status]}</span></div>
          {run.error && <p className="error-message">{run.error}</p>}
          {plans.filter((plan) => plan.runId === run.id).map((plan) => <PlanCard key={plan.id} plan={plan} preview={preview} canDecide={!preview && run.status === 'completed'} pending={pending} onReview={onReviewPlan} onDecision={(_id, decision) => { if (!preview) onDecidePlan(plan, decision); }} />)}
          <div className="message-footnote"><ShieldCheck size={12} />{preview ? '示例对话 · 不执行任何操作' : '模型回复 · 不执行工具或应用修改'}</div>
        </article>
      </section>)}
    </div>
  </div>;
}
