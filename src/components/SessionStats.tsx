import { useEffect, useRef, useState } from 'react';
import * as Popover from '@radix-ui/react-popover';
import { Clock3, Database, Gauge, Zap } from 'lucide-react';
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

/// Session-level projection from provider-reported run metrics. The active
/// run's output falls back to a live text estimate and is marked `est.`.
export function sessionUsage(runs: RunView[], metrics: Record<number, RunMetrics> | undefined, context: number | null) {
  const current = runs.at(-1);
  const reports = runs.map((run) => metrics?.[run.id]).filter((report): report is RunMetrics => Boolean(report));
  const exactInput = reports.reduce((total, report) => total + (report.inputTokens ?? 0), 0);
  const exactOutput = reports.reduce((total, report) => total + (report.outputTokens ?? 0), 0);
  const cacheRead = reports.reduce((total, report) => total + (report.cacheHitTokens ?? 0), 0);
  const inputKnown = reports.length > 0 && reports.every((report) => report.inputTokens != null);
  const cacheKnown = reports.length > 0 && reports.every((report) => report.cacheHitTokens != null && report.inputTokens != null);
  const liveOutput = current?.status === 'running' ? estimateTokens(current.output) : 0;
  const output = Math.max(exactOutput, liveOutput);
  const estimated = current?.status === 'running' && output > exactOutput;
  const total = exactInput + output;
  const cachePct = cacheKnown ? cacheHitPercent(cacheRead, exactInput) : null;
  const uncachedInput = cacheKnown ? Math.max(0, exactInput - cacheRead) : null;
  const elapsed = reports.reduce((longest, report) => Math.max(longest, report.elapsedMs), 0);
  const firstToken = reports.at(-1)?.firstTokenMs ?? 0;
  const speed = elapsed > 0 && output > 0 ? (output / (Math.max(1, elapsed - firstToken) / 1000)).toFixed(1) : '—';
  return { current, report: reports.at(-1), known: inputKnown && !estimated, input: exactInput, output, total, used: total, cacheRead, cachePct, uncachedInput, speed, context, estimated, elapsed };
}

export default function SessionStats({ runs, metrics, context, steps = 0 }: { runs: RunView[]; metrics?: Record<number, RunMetrics>; context: number | null; steps?: number }) {
  const current = runs.at(-1);
  const [elapsed, setElapsed] = useState(0);
  const started = useRef(performance.now());
  useEffect(() => {
    started.current = performance.now(); setElapsed(0);
    if (current?.status !== 'running') return;
    const timer = window.setInterval(() => setElapsed(performance.now() - started.current), 250);
    return () => window.clearInterval(timer);
  }, [current?.id, current?.status]);
  const usage = sessionUsage(runs, metrics, context);
  const liveMs = current?.status === 'running' ? Math.max(usage.elapsed, elapsed) : usage.elapsed;
  const liveSpeed = liveMs > 0 && usage.output > 0 ? (usage.output / (Math.max(1, liveMs - (usage.report?.firstTokenMs ?? 0)) / 1000)).toFixed(1) : usage.speed;
  const hasData = runs.length > 0;

  return <div className="session-stats" aria-label="实时会话统计">
    <Popover.Root>
      <Popover.Trigger asChild>
        <button type="button" className="stat-pill" aria-label={`会话统计 ${runs.length} 轮 ${steps} 步 ${liveSpeed} tok/s`}>
          <Gauge size={12} /><span>{hasData ? `${runs.length} 轮 · ${steps} 步 · ${liveSpeed} tok/s` : '等待首次生成'}</span>
        </button>
      </Popover.Trigger>
      <Popover.Portal><Popover.Content className="stat-popover" side="top" align="center" sideOffset={6}>
        <div className="stat-popover-title"><Clock3 size={13} />会话统计</div>
        <dl><dt>轮次 / 步数</dt><dd>{runs.length} / {steps}</dd><dt>耗时</dt><dd>{(liveMs / 1000).toFixed(1)}s</dd><dt>输出速度</dt><dd>{liveSpeed} tok/s</dd><dt>输出 token</dt><dd>{compactTokens(usage.output)}{usage.estimated && <small> est.</small>}</dd></dl>
      </Popover.Content></Popover.Portal>
    </Popover.Root>
    <Popover.Root>
      <Popover.Trigger asChild>
        <button type="button" className="stat-pill" aria-label={`Token 用量 ${compactTokens(usage.total)} tok 缓存命中 ${usage.cachePct ?? '—'}`}>
          <Database size={12} /><span>{hasData ? `${compactTokens(usage.total)} tok · 缓存命中 ${usage.cachePct ?? '—'}${usage.cachePct ? '%' : ''}` : '等待首次生成'}</span>
        </button>
      </Popover.Trigger>
      <Popover.Portal><Popover.Content className="stat-popover" side="top" align="center" sideOffset={6}>
        <div className="stat-popover-title"><Database size={13} />Token 用量</div>
        <dl><dt>输入</dt><dd>{compactTokens(usage.input)}{!usage.known && <small> est.</small>}</dd><dt>缓存读取</dt><dd>{usage.cachePct === null ? '—' : compactTokens(usage.cacheRead)}</dd><dt>未缓存输入</dt><dd>{usage.uncachedInput === null ? '—' : compactTokens(usage.uncachedInput)}</dd><dt>输出</dt><dd>{compactTokens(usage.output)}{usage.estimated && <small> est.</small>}</dd><dt>缓存命中</dt><dd>{usage.cachePct === null ? '—' : `${usage.cachePct}%`}</dd><dt>上下文</dt><dd>{context ? compactTokens(context) : '服务端配置'}</dd></dl>
      </Popover.Content></Popover.Portal>
    </Popover.Root>
  </div>;
}
