import { useEffect, useRef, useState } from 'react';
import * as Popover from '@radix-ui/react-popover';
import { Clock3, Database, Gauge } from 'lucide-react';
import type { RunMetrics, RunView } from '../api/types';
import { formatContextPercent, getRenderableContextUsage } from '../lib/contextUsage';

export function estimateTokens(text: string): number {
  const cjk = (text.match(/[㐀-鿿]/g) ?? []).length;
  return cjk + Math.ceil((text.length - cjk) / 4);
}
export const compactTokens = (value: number) => value >= 1000000 ? `${(value / 1000000).toFixed(1)}M` : value >= 1000 ? `${(value / 1000).toFixed(1)}k` : String(Math.round(value));

/// DSH cacheHitPercent：cache reads / billed prompt input，整数百分比。
export function cacheHitPercent(cacheRead: number, billedInput: number): string | null {
  if (billedInput <= 0) return null;
  return String(Math.round((cacheRead / billedInput) * 100));
}

/// DSH tok/s format: one decimal below 10, integer above.
export function formatSpeed(value: string): string {
  if (value === '—') return value;
  const numeric = Number(value);
  return Number.isFinite(numeric) && numeric >= 10 ? String(Math.round(numeric)) : value;
}

/// DSH formatDuration：45.2s under a minute, 11m48s from there on.
export function formatDuration(ms: number): string {
  const s = ms / 1000;
  if (s < 60) return `${Math.round(s * 10) / 10}s`;
  const whole = Math.round(s);
  return `${Math.floor(whole / 60)}m${whole % 60}s`;
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
  // DSH "步" = step/end 计数 = 已完成 LLM 请求数（attempts），不是工具调用数。
  const attempts = reports.reduce((total, report) => total + (report.attempts ?? 0), 0);
  // DSH sessionStats：TTFT = Σ首 delta 延迟 / 记录了首 delta 的 step 数；
  // 缺新字段的旧行回退到 attempts（与后端 v15 读取回退一致）。
  const firstTotal = reports.reduce((total, report) => total + (report.firstTokenMs ?? 0), 0);
  const ttftSteps = reports.reduce(
    (total, report) => total + (report.ttftSteps ?? (report.firstTokenMs != null ? (report.attempts ?? 0) : 0)),
    0,
  );
  const ttft = ttftSteps > 0 ? (firstTotal / ttftSteps).toFixed(0) : null;
  // The backend now keeps a monotonic projection (`MetricsAccumulator`), so a
  // tick without usage never clears the value. This last-known fold is the
  // defence layer for older rows/events; ZCode does the same via its session
  // projection (zcodeSessionProjection.ts:3146-3200).
  const contextUsed = reports.reduce<number | null>(
    (latest, report) => report.lastInputTokens ?? latest,
    null,
  );
  const inputKnown = reports.length > 0 && reports.every((report) => report.inputTokens != null);
  const cacheKnown = reports.length > 0 && reports.every((report) => report.cacheHitTokens != null && report.inputTokens != null);
  // DSH surfaceTokens 口径：后端按当前请求流式 delta 文本估算（含
  // reasoning），只在 live 快照携带；字段为 null 表示本请求尚无 delta。
  // 旧事件源（字段缺失 undefined）回退整段输出文本估算，粒度更粗。
  const running = current?.status === 'running';
  const currentReport = current ? metrics?.[current.id] : undefined;
  const surfaceEst = currentReport?.liveSurfaceTokens;
  const surfaceLive = typeof surfaceEst === 'number' ? surfaceEst : 0;
  const legacyEst = running === true && surfaceEst === undefined ? estimateTokens(current?.output ?? '') : 0;
  const output = running ? Math.max(exactOutput, exactOutput + surfaceLive, legacyEst) : exactOutput;
  const estimated = running === true && output > exactOutput;
  // 在途速度（估）= 当前请求 surface 估算 / 当前请求 decode 窗口
  // （lastElapsed − lastFirstToken 均为 live view 中的当前请求值）。
  const liveWindowMs = running === true && currentReport?.lastFirstTokenMs != null
    ? Math.max(0, (currentReport.lastElapsedMs ?? 0) - currentReport.lastFirstTokenMs)
    : 0;
  const liveTps = surfaceLive > 0 && liveWindowMs > 0
    ? surfaceLive / (liveWindowMs / 1000)
    : null;
  // 最近完成请求的 per-request 速度（last_* 三元组即成对窗口）。
  const report = reports.at(-1);
  const lastTps = report?.lastOutputTokens != null && report.lastFirstTokenMs != null && (report.lastElapsedMs ?? 0) > report.lastFirstTokenMs
    ? report.lastOutputTokens / (((report.lastElapsedMs ?? 0) - report.lastFirstTokenMs) / 1000)
    : null;
  const total = exactInput + output;
  const cachePct = cacheKnown ? cacheHitPercent(cacheRead, exactInput) : null;
  const uncachedInput = cacheKnown ? Math.max(0, exactInput - cacheRead - cacheWrite) : null;
  const elapsed = reports.reduce((total, report) => total + report.elapsedMs, 0);
  const toolMs = reports.reduce((total, report) => total + (report.toolMs ?? 0), 0);
  // DSH sessionStats decode 口径（官方叫"输出速度 TPS"）：tok/s =
  // ΣdecodeTokens / ΣdecodeMs；两者只在「首 delta 与 output usage 同有」的
  // 请求上成对累计 —— outputTokens 已含 reasoning，绝不重复相加。缺新字段
  // 的旧行回退 elapsed-firstToken 配对近似。
  const decode = reports.reduce(
    (acc, report) => {
      if (report.decodeMs != null || report.decodeTokens != null) {
        return { tokens: acc.tokens + (report.decodeTokens ?? 0), ms: acc.ms + (report.decodeMs ?? 0) };
      }
      if (report.firstTokenMs != null && report.outputTokens != null) {
        return {
          tokens: acc.tokens + report.outputTokens,
          ms: acc.ms + Math.max(0, report.elapsedMs - report.firstTokenMs),
        };
      }
      return acc;
    },
    { tokens: 0, ms: 0 },
  );
  const speed = decode.tokens > 0 && decode.ms > 0
    ? (decode.tokens / (decode.ms / 1000)).toFixed(1)
    : '—';
  const renderableContext = getRenderableContextUsage(contextUsed, context);
  const contextPercent = renderableContext ? Math.min(100, (renderableContext.used / renderableContext.context) * 100) : null;

  return {
    current, report, known: inputKnown && !estimated, input: exactInput, output, total, used: total,
    cacheRead, cacheWrite, reasoning, attempts, ttft, contextUsed, contextPercent,
    cachePct, uncachedInput, speed, toolMs, context, estimated, elapsed, liveTps, lastTps,
  };
}

export default function SessionStats({ runs, metrics, context }: { runs: RunView[]; metrics?: Record<number, RunMetrics>; context: number | null }) {
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
  const sep = <span className="stats-sep" />;

  // DSH StatsLine：`N 轮 M 步 · X tok/s · Y tok · 缓存命中 Z% · W%`
  // （步 = LLM 请求数；tok/s = 输出速度 TPS；末尾 W% = 上下文占用）。
  return <div className="session-stats" aria-label="会话统计">
    <Popover.Root>
      <Popover.Trigger asChild>
        <button type="button" className="stats-line" aria-label={`会话统计 ${runs.length} 轮 ${usage.attempts} 步 输出速度 ${formatSpeed(usage.speed)} tok/s 用量 ${compactTokens(usage.total)} tok 缓存命中 ${usage.cachePct ?? '—'}% Context ${usage.contextPercent === null ? '—' : formatContextPercent(usage.contextPercent / 100)}`}>
          <Gauge size={11} className="stats-line-icon" />
          <span className="stats-chip">{runs.length} 轮 {usage.attempts} 步</span>
          {sep}
          <span className="stats-chip" title={usage.liveTps !== null ? '本轮速度（估）' : undefined}>{usage.liveTps !== null ? `≈${formatSpeed(usage.liveTps.toFixed(1))}` : formatSpeed(usage.speed)} tok/s</span>
          {sep}
          <span className="stats-chip">{hasData ? `${compactTokens(usage.total)} tok` : '—'}</span>
          {usage.cachePct !== null && <>{sep}<span className="stats-chip">缓存命中 {usage.cachePct}%</span></>}
          {usage.contextPercent !== null && <>{sep}<span className="stats-chip">{formatContextPercent(usage.contextPercent / 100)}</span></>}
        </button>
      </Popover.Trigger>
      <Popover.Portal><Popover.Content className="stat-popover" side="top" align="center" sideOffset={6}>
        <div className="stat-popover-title"><Clock3 size={13} />会话统计</div>
        <dl>
          <dt>轮次 / 步数</dt><dd>{runs.length} / {usage.attempts}</dd>
          <dt>模型用时</dt><dd>{formatDuration(liveMs)}</dd>
          <dt>工具调用用时</dt><dd>{usage.toolMs > 0 ? formatDuration(usage.toolMs) : '—'}</dd>
          <dt>首 token 平均 (TTFT)</dt><dd>{usage.ttft === null ? '—' : formatDuration(Number(usage.ttft))}</dd>
          <dt>输出速度 (TPS)</dt><dd>{formatSpeed(usage.speed)} tok/s</dd>
          <dt>本轮速度（估）</dt><dd>{usage.liveTps === null ? '—' : `${formatSpeed(usage.liveTps.toFixed(1))} tok/s`}</dd>
          <dt>最近请求 TPS</dt><dd>{usage.lastTps === null ? '—' : `${formatSpeed(usage.lastTps.toFixed(1))} tok/s`}</dd>
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
