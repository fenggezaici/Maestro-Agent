import { expect, it } from 'vitest';

import type { RunMetrics, RunView } from '../api/types';
import { cacheHitPercent, sessionUsage } from './SessionStats';

it('keeps live output moving and reports cache hit as an integer percent (DSH)', () => {
  const run: RunView = { id: 1, sessionId: 1, prompt: 'hello', mode: 'code', status: 'running', output: 'streaming text', error: null };
  const metrics: RunMetrics = { runId: 1, sessionId: 1, provider: 'openai', model: 'test', inputTokens: 1000, outputTokens: 0, cacheHitTokens: 999, elapsedMs: 1000, firstTokenMs: 100 };
  const usage = sessionUsage([run], { 1: metrics }, 8000);
  expect(usage.output).toBeGreaterThan(0);
  expect(usage.estimated).toBe(true);
  expect(usage.used).toBeGreaterThan(1000);
  // DSH cacheHitPercent = Math.round(cacheRead/billed*100)：999/1000 → 100。
  expect(cacheHitPercent(999, 1000)).toBe('100');
  expect(cacheHitPercent(674, 1000)).toBe('67');
  expect(cacheHitPercent(0, 0)).toBeNull();
});

it('derives tok/s only from provider usage, never from text estimates', () => {
  const run: RunView = { id: 2, sessionId: 1, prompt: 'hello', mode: 'code', status: 'completed', output: 'x'.repeat(4000), error: null };
  const metrics: RunMetrics = { runId: 2, sessionId: 1, provider: 'openai', model: 'test', inputTokens: 100, outputTokens: 1000, cacheHitTokens: 50, elapsedMs: 10_000, firstTokenMs: 2_000 };
  // 无 decode 字段的旧行回退 elapsed - firstToken 配对近似。
  expect(sessionUsage([run], { 2: metrics }, 8000).speed).toBe('125.0');
  expect(sessionUsage([run], { 2: { ...metrics, outputTokens: null } }, 8000).speed).toBe('—');
  expect(sessionUsage([run], {}, 8000).speed).toBe('—');
});

it('uses the paired decode window when v15 fields are present', () => {
  const run: RunView = { id: 2, sessionId: 1, prompt: 'hello', mode: 'code', status: 'completed', output: 'x', error: null };
  const metrics: RunMetrics = {
    runId: 2, sessionId: 1, provider: 'openai', model: 'test',
    inputTokens: 100, outputTokens: 1200, cacheHitTokens: 0,
    elapsedMs: 20_000, firstTokenMs: 5_000,
    decodeMs: 9_000, decodeTokens: 900, ttftSteps: 1, toolMs: 2_000,
  };
  const usage = sessionUsage([run], { 2: metrics }, 8000);
  // 900/9s = 100 tok/s —— 不是 1200/(20-5)s；工具时间与未配对 output 都不计。
  expect(usage.speed).toBe('100.0');
  expect(usage.ttft).toBe('5000');
  expect(usage.toolMs).toBe(2000);
});

it('counts only requests that pair a first delta with output usage', () => {
  const base: RunMetrics = { runId: 3, sessionId: 1, provider: 'openai', model: 'test', inputTokens: 10, outputTokens: 500, cacheHitTokens: 5, elapsedMs: 6_000, firstTokenMs: null };
  const runs: RunView[] = [
    { id: 3, sessionId: 1, prompt: 'a', mode: 'code', status: 'completed', output: '', error: null },
    { id: 4, sessionId: 1, prompt: 'b', mode: 'code', status: 'completed', output: '', error: null },
  ];
  // run 3 无首 delta 记录 → 不成对；只有 run 4 的 1000 tokens / 3000ms 计入。
  expect(sessionUsage(runs, { 3: base, 4: { ...base, runId: 4, outputTokens: 1000, elapsedMs: 4_000, firstTokenMs: 1_000 } }, 8000).speed).toBe('333.3');
  // run 4 缺 usage 时全部不成对 → —，不拿 run 3 的孤立 output 硬算。
  expect(sessionUsage(runs, { 3: base, 4: { ...base, runId: 4, outputTokens: null } }, 8000).speed).toBe('—');
});

it('averages TTFT over steps that saw a first delta, not every attempt', () => {
  const run: RunView = { id: 5, sessionId: 1, prompt: 'a', mode: 'code', status: 'completed', output: '', error: null };
  const metrics: RunMetrics = {
    runId: 5, sessionId: 1, provider: 'openai', model: 'test',
    inputTokens: 10, outputTokens: 10, cacheHitTokens: 0,
    elapsedMs: 10_000, firstTokenMs: 3_000, attempts: 4, ttftSteps: 2,
  };
  // Σ3000ms / 2 个有首 delta 的 step，而不是除以 4 次 attempts。
  expect(sessionUsage([run], { 5: metrics }, 8000).ttft).toBe('1500');
});

it('adds the in-flight surface estimate to output and derives a live TPS', () => {
  const run: RunView = { id: 20, sessionId: 1, prompt: 'a', mode: 'code', status: 'running', output: 'abc', error: null };
  const metrics: RunMetrics = {
    runId: 20, sessionId: 1, provider: 'openai', model: 'm',
    inputTokens: 100, outputTokens: null, cacheHitTokens: 0,
    elapsedMs: 5_000, firstTokenMs: null,
    lastElapsedMs: 3_000, lastFirstTokenMs: 1_000,
    liveSurfaceTokens: 400,
  };
  const usage = sessionUsage([run], { 20: metrics }, 8000);
  // exactOutput(0) + surface est(400)；est 标记生效。
  expect(usage.output).toBe(400);
  expect(usage.estimated).toBe(true);
  // 400 tokens / (3000−1000)ms 窗口 = 200 tok/s。
  expect(usage.liveTps).toBeCloseTo(200);
});

it('treats a null surface estimate as zero instead of falling back to whole-output text', () => {
  const run: RunView = { id: 21, sessionId: 1, prompt: 'a', mode: 'code', status: 'running', output: 'x'.repeat(1000), error: null };
  const metrics: RunMetrics = {
    runId: 21, sessionId: 1, provider: 'openai', model: 'm',
    inputTokens: 100, outputTokens: 50, cacheHitTokens: 0,
    elapsedMs: 1_000, firstTokenMs: null,
    liveSurfaceTokens: null,
  };
  const usage = sessionUsage([run], { 21: metrics }, 8000);
  // 字段为 null = 当前请求尚无 delta —— 不回退整段文本估算（会重复计数）。
  expect(usage.output).toBe(50);
  expect(usage.estimated).toBe(false);
  expect(usage.liveTps).toBeNull();
});

it('falls back to whole-output estimate when liveSurfaceTokens is absent', () => {
  const run: RunView = { id: 22, sessionId: 1, prompt: 'a', mode: 'code', status: 'running', output: 'x'.repeat(400), error: null };
  const metrics: RunMetrics = {
    runId: 22, sessionId: 1, provider: 'openai', model: 'm',
    inputTokens: 10, outputTokens: null, cacheHitTokens: 0,
    elapsedMs: 1_000, firstTokenMs: null,
  };
  const usage = sessionUsage([run], { 22: metrics }, 8000);
  expect(usage.output).toBe(100);
  expect(usage.estimated).toBe(true);
  expect(usage.liveTps).toBeNull();
});

it('derives the last-request TPS from the last_* paired window', () => {
  const run: RunView = { id: 30, sessionId: 1, prompt: 'a', mode: 'code', status: 'completed', output: '', error: null };
  const metrics: RunMetrics = {
    runId: 30, sessionId: 1, provider: 'openai', model: 'm',
    inputTokens: 10, outputTokens: 500, cacheHitTokens: 0,
    elapsedMs: 5_000, firstTokenMs: 1_000,
    lastOutputTokens: 400, lastElapsedMs: 3_000, lastFirstTokenMs: 500,
  };
  const usage = sessionUsage([run], { 30: metrics }, 8000);
  // 400 tokens / (3000−500)ms = 160 tok/s —— per-request 视角。
  expect(usage.lastTps).toBeCloseTo(160);
  expect(sessionUsage([run], { 30: { ...metrics, lastFirstTokenMs: null } }, 8000).lastTps).toBeNull();
});

it('keeps the last known prompt size when the newest report has no usage yet', () => {
  const runs: RunView[] = [
    { id: 10, sessionId: 1, prompt: 'a', mode: 'code', status: 'completed', output: '', error: null },
    { id: 11, sessionId: 1, prompt: 'b', mode: 'code', status: 'running', output: '', error: null },
  ];
  const older: RunMetrics = {
    runId: 10, sessionId: 1, provider: 'openai', model: 'm', inputTokens: 1, outputTokens: 1,
    cacheHitTokens: 0, elapsedMs: 1, firstTokenMs: null, lastInputTokens: 136_047,
  };
  const newer: RunMetrics = {
    runId: 11, sessionId: 1, provider: 'openai', model: 'm', inputTokens: null, outputTokens: null,
    cacheHitTokens: null, elapsedMs: 0, firstTokenMs: null, lastInputTokens: null,
  };
  expect(sessionUsage(runs, { 10: older, 11: newer }, 1_000_000).contextUsed).toBe(136_047);
});
