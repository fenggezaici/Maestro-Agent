import { useEffect, useRef, useState } from 'react';
import * as Popover from '@radix-ui/react-popover';
import { Clock3, Database, Gauge } from 'lucide-react';
import type { RunMetrics, RunView } from '../api/types';

export function estimateTokens(text: string): number {
  const cjk = (text.match(/[\u3400-\u9fff]/g) ?? []).length;
  return cjk + Math.ceil((text.length - cjk) / 4);
}
export const compactTokens = (value: number) => value >= 1000000 ? `${(value / 1000000).toFixed(1)}M` : value >= 1000 ? `${(value / 1000).toFixed(1)}k` : String(Math.round(value));

/// DeepSeek Harness-style cache-hit share: cache reads over billed prompt
/// input, output excluded. Never rounds a partial hit up to 100%.
export function cacheHitPercent(cacheRead: number, billedInput: number): string | null {
  if (billedInput <= 0) return null;
  const raw = (cacheRead / billedInput) * 100;
  if (raw >= 100) return '100';
  if (raw >= 99.95) return '99.9';
  return raw.toFixed(1);
}

/// Harness speed format: one decimal below 100, integer above.
export function formatSpeed(value: string): string {
  if (value === '—') return value;
  const numeric = Number(value);
  return Number.isFinite(numeric) && numeric >= 100 ? String(Math.round(numeric)) : value;
}

/// Session-level projection from provider-reported run metrics, following the
/// Harness tokenUsage/contextPressure split: Input is billed (cache + uncached),
/// Output is provider-reported, Context is the newest prompt.
export function sessionUsage(runs: RunView[], metrics: Record<number, RunMetrics> | undefined, context: number | null) {
  const current = runs.at(-1);
  const reports = runs.map((run) => metrics?.[run.id]).filter((report): report is RunMetrics => Boolean(report));
  const exactInput = reports.reduce((total, report) => total + (report.inputTokens ?? 0), 0);
  const exactOutput = reports.reduce((total, report) => total + (report.outputTokens ?? 0), 0);
  const cacheRead = reports.reduce((total, report) => total + (report.cacheHitTokens ?? 0), 0);
  const cacheWrite = reports.reduce((total, report) => total + (report.cacheWriteTokens ?? 0), 0);
  const reasoning = reports.reduce((total, report) => total + (report.reasoningTokens ?? 0), 0);
  const attempts = reports.reduce((total, report) => total + (report.attempts ?? 0), 0);
  const firstTotal = reports.reduce((total, report) => total + (report.firstTokenMs ?? 0), 0);
  const ttft = attempts > 0 ? (firstTotal / attempts).toFixed(0) : null;
  const lastReport = [...reports].reverse().find((report) => report.lastOutputTokens != null && (report.lastElapsedMs ?? 0) > (report.lastFirstTokenMs ?? 0));
  const lastSpeed = lastReport
    ? ((lastReport.lastOutputTokens ?? 0) / (((lastReport.lastElapsedMs ?? 0) - (lastReport.lastFirstTokenMs ?? 0)) / 1000)).toFixed(1)
    : '—';
  const contextUsed = reports.at(-1)?.lastInputTokens ?? reports.at(-1)?.inputTokens ?? 0;
  const inputKnown = reports.length > 0 && reports.every((report) => report.inputTokens != null);
  const cacheKnown = reports.length > 0 && reports.every((report) => report.cacheHitTokens != null && report.inputTokens != null);
  const liveOutput = current?.status === 'running' ? estimateTokens(current.output) : 0;
  const output = Math.max(exactOutput, liveOutput);
  const estimated = current?.status === 'running' && output > exactOutput;
  const total = exactInput + output;
  const cachePct = cacheKnown ? cacheHitPercent(cacheRead, exactInput) : null;
  const uncachedInput = cacheKnown ? Math.max(0, exactInput - cacheRead - cacheWrite) : null;
  const elapsed = reports.reduce((total, report) => total + report.elapsedMs, 0);
  const speedReports = reports.filter((report) => report.outputTokens != null && report.elapsedMs > 0);
  const decodeTokens = speedReports.reduce((total, report) => total + (report.outputTokens ?? 0), 0);
  const decodeMs = speedReports.reduce((total, report) => total + Math.max(0, report.elapsedMs - (report.firstTokenMs ?? 0)), 0);
  const speed = decodeTokens > 0 && decodeMs > 0 ? (decodeTokens / (decodeMs / 1000)).toFixed(1) : '—';
  const contextPercent = context && contextUsed > 0 ? Math.min(100, (contextUsed / context) * 100) : null;
  return {
    current, report: reports.at(-1), known: inputKnown && !estimated, input: exactInput, output, total, used: total,
    cacheRead, cacheWrite, reasoning, attempts, ttft, lastSpeed, contextUsed, contextPercent,
    cachePct, uncachedInput, speed, context, estimated, elapsed,
  };
}

export default function SessionStats({ runs, metrics, context, steps = 0 }: { runs: RunView[]; metrics?: Record<number, RunMetrics>; context: number | null; steps?: number }) {
  const current = runs.at(-1);
  const [elapsed, setElapsed] = useState(0);
  const started = useRef(performance.now());
  const runIdRef = useRef<number | undefined>(undefined);
  useEffect(() => {
    if (runIdRef.current === current?.id) return;
    runIdRef.current = current?.id;
    started.current = performance.now();
    setElapsed(0);
  }, [current?.id]);
  useEffect(() => {
    if (current?.status !== 'running') return;
    const timer = window.setInterval(() => setElapsed(performance.now() - started.current), 250);
    return () => window.clearInterval(timer);
  }, [current?.id, current?.status]);
  const usage = sessionUsage(runs, metrics, context);
  const liveMs = current?.status === 'running' ? Math.max(usage.elapsed, elapsed) : usage.elapsed;
  const hasData = runs.length > 0;
  const chip = (label: string, value: string) => <span className="stats-chip"><span className="stats-chip-label">{label}</span>{value}</span>;

  return <div className="session-stats" aria-label="会话统计">
    <Popover.Root>
      <Popover.Trigger asChild>
        <button type="button" className="stats-line" aria-label={`会话统计 Context ${usage.contextPercent === null ? '—' : `${usage.contextPercent.toFixed(1)}%`} Input ${compactTokens(usage.input)} Output ${compactTokens(usage.output)} Hit ${usage.cachePct ?? '—'} Speed ${formatSpeed(usage.speed)} tok/s TTFT ${usage.ttft === null ? '—' : `${usage.ttft}ms`}`}>
          <Gauge size={11} className="stats-line-icon" />
          {chip('Context', usage.contextPercent === null ? '—' : `${usage.contextPercent.toFixed(1)}%`)}
          <span className="stats-sep" />
          {chip('Input', hasData ? compactTokens(usage.input) : '—')}
          <span className="stats-sep" />
          {chip('Output', hasData ? compactTokens(usage.output) : '—')}
          <span className="stats-sep" />
          {chip('Hit', usage.cachePct === null ? '—' : `${usage.cachePct}%`)}
          <span className="stats-sep" />
          {chip('', `${formatSpeed(usage.speed)} tok/s`)}
          <span className="stats-sep" />
          {chip('TTFT', usage.ttft === null ? '—' : `${(Number(usage.ttft) / 1000).toFixed(1)}s`)}
        </button>
      </Popover.Trigger>
      <Popover.Portal><Popover.Content className="stat-popover" side="top" align="center" sideOffset={6}>
        <div className="stat-popover-title"><Clock3 size={13} />会话统计</div>
        <dl>
          <dt>轮次 / 步数</dt><dd>{runs.length} / {steps}</dd>
          <dt>模型流耗时</dt><dd>{(liveMs / 1000).toFixed(1)}s</dd>
          <dt>会话解码速度</dt><dd>{formatSpeed(usage.speed)} tok/s</dd>
          <dt>最近一轮速度</dt><dd>{usage.lastSpeed} tok/s</dd>
          <dt>平均首 token</dt><dd>{usage.ttft === null ? '—' : `${usage.ttft} ms`}</dd>
          <dt>输入（计费）</dt><dd>{compactTokens(usage.input)}{!usage.known && <small> est.</small>}</dd>
          <dt>缓存读取</dt><dd>{usage.cachePct === null ? '—' : compactTokens(usage.cacheRead)}</dd>
          <dt>缓存写入</dt><dd>{usage.cacheWrite ? compactTokens(usage.cacheWrite) : '—'}</dd>
          <dt>未缓存输入</dt><dd>{usage.uncachedInput === null ? '—' : compactTokens(usage.uncachedInput)}</dd>
          <dt>输出</dt><dd>{compactTokens(usage.output)}{usage.estimated && <small> est.</small>}</dd>
          <dt>思考 token</dt><dd>{usage.reasoning ? compactTokens(usage.reasoning) : '—'}</dd>
          <dt>缓存命中</dt><dd>{usage.cachePct === null ? '—' : `${usage.cachePct}%`}</dd>
          <dt>上下文</dt><dd>{context ? compactTokens(context) : '服务端配置'}</dd>
        </dl>
      </Popover.Content></Popover.Portal>
    </Popover.Root>
    <span className="stats-source" aria-hidden="true"><Database size={10} />{hasData ? `${runs.length} 轮` : '等待首次生成'}</span>
  </div>;
}
