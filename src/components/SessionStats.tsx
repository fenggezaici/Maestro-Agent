import { useEffect, useRef, useState } from 'react';
import type { RunMetrics, RunView } from '../api/types';

export function estimateTokens(text: string): number {
  const cjk = (text.match(/[\u3400-\u9fff]/g) ?? []).length;
  return cjk + Math.ceil((text.length - cjk) / 4);
}
const compact = (value: number) => value >= 1000000 ? `${(value / 1000000).toFixed(1)}M` : value >= 1000 ? `${(value / 1000).toFixed(1)}k` : String(value);

export default function SessionStats({ runs, metrics, context }: { runs: RunView[]; metrics?: Record<number, RunMetrics>; context: number | null }) {
  const current = runs.at(-1);
  const [elapsed, setElapsed] = useState(0);
  const started = useRef(performance.now());
  useEffect(() => {
    started.current = performance.now(); setElapsed(0);
    if (current?.status !== 'running') return;
    const timer = window.setInterval(() => setElapsed(performance.now() - started.current), 250);
    return () => window.clearInterval(timer);
  }, [current?.id, current?.status]);
  const report = current ? metrics?.[current.id] : undefined;
  const known = report?.inputTokens != null && report.outputTokens != null;
  const used = known ? report.inputTokens! + report.outputTokens! : estimateTokens(runs.slice(-32).map((run) => `${run.prompt}\n${run.output}`).join('\n'));
  const ms = current?.status === 'running' ? Math.max(report?.elapsedMs ?? 0, elapsed) : report?.elapsedMs ?? 0;
  const speedMs = Math.max(1, ms - (report?.firstTokenMs ?? 0));
  const output = report?.outputTokens ?? estimateTokens(current?.output ?? '');
  const speed = ms > 0 && output > 0 ? (output / (speedMs / 1000)).toFixed(1) : '—';
  return <div className="session-stats" aria-label="实时会话统计">
    <span>Turns: {runs.length}</span><i />
    <span title={known ? 'Provider-reported last-request usage' : 'Estimate only: no provider usage available; not a tokenizer count'}>Tokens: {compact(used)} / {context ? compact(context) : 'unknown'} {!known && <small>est.</small>}</span><i />
    {report?.inputTokens != null && report.cacheHitTokens != null && report.inputTokens > 0 && <>
      <span title={`本轮缓存命中 ${compact(report.cacheHitTokens)} / 输入 ${compact(report.inputTokens)} tokens（DeepSeek prompt_cache_hit / OpenAI cached_tokens / Anthropic cache_read）`}>命中 {Math.round((report.cacheHitTokens / report.inputTokens) * 100)}%</span><i /></>}
    <span>Speed: {speed} t/s {report?.outputTokens == null && speed !== '—' && <small>est.</small>}</span>
  </div>;
}
