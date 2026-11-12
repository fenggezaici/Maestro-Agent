import { useEffect, useRef, useState } from 'react';
import { Clock3, HardDrive, Zap } from 'lucide-react';
import type { RunMetrics, RunView } from '../api/types';
import ContextRing from './ContextRing';

export function estimateTokens(text: string): number {
  const cjk = (text.match(/[\u3400-\u9fff]/g) ?? []).length;
  return cjk + Math.ceil((text.length - cjk) / 4);
}
export const compactTokens = (value: number) => value >= 1000000 ? `${(value / 1000000).toFixed(1)}M` : value >= 1000 ? `${(value / 1000).toFixed(1)}k` : String(value);

/// Shared usage projection for the composer ring and the status bar.
export function sessionUsage(runs: RunView[], metrics: Record<number, RunMetrics> | undefined, context: number | null) {
  const current = runs.at(-1);
  const report = current ? metrics?.[current.id] : undefined;
  const known = report?.inputTokens != null && report.outputTokens != null;
  const used = known ? report.inputTokens! + report.outputTokens! : estimateTokens(runs.slice(-32).map((run) => `${run.prompt}\n${run.output}`).join('\n'));
  const output = report?.outputTokens ?? estimateTokens(current?.output ?? '');
  const elapsed = Math.max(report?.elapsedMs ?? 0, 0);
  const speedMs = Math.max(1, elapsed - (report?.firstTokenMs ?? 0));
  const speed = elapsed > 0 && output > 0 ? (output / (speedMs / 1000)).toFixed(1) : '—';
  const cachePct = report?.inputTokens != null && report.cacheHitTokens != null && report.inputTokens > 0
    ? Math.round((report.cacheHitTokens / report.inputTokens) * 100)
    : null;
  return { current, report, known, used, output, speed, cachePct, context };
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
  const liveMs = current?.status === 'running' ? Math.max(usage.report?.elapsedMs ?? 0, elapsed) : usage.report?.elapsedMs ?? 0;
  const liveOutput = usage.report?.outputTokens ?? estimateTokens(current?.output ?? '');
  const liveSpeed = liveMs > 0 && liveOutput > 0 ? (liveOutput / (Math.max(1, liveMs - (usage.report?.firstTokenMs ?? 0)) / 1000)).toFixed(1) : usage.speed;
  return <div className="session-stats" aria-label="实时会话统计">
    <span><Clock3 size={11} />{runs.length} 轮 {steps} 步</span><i />
    <span title={usage.report?.outputTokens == null ? '按文本估算的生成速度' : 'Provider-reported output speed'}><Zap size={11} />{liveSpeed} tok/s{usage.report?.outputTokens == null && liveSpeed !== '—' && <small>est.</small>}</span><i />
    <span title={usage.known ? 'Provider-reported last-request usage' : 'Estimate only: no provider usage available'}>
      <HardDrive size={11} />{compactTokens(usage.used)} tok{usage.cachePct !== null && ` · 缓存命中 ${usage.cachePct}%`}{!usage.known && <small>est.</small>}
    </span><i />
    <ContextRing used={usage.used} context={context} size={18} />
  </div>;
}
