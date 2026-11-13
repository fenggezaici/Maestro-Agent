import { expect, it } from 'vitest';

import type { RunMetrics, RunView } from '../api/types';
import { cacheHitPercent, sessionUsage } from './SessionStats';

it('keeps live output moving and never rounds a partial cache hit to 100%', () => {
  const run: RunView = { id: 1, sessionId: 1, prompt: 'hello', mode: 'code', status: 'running', output: 'streaming text', error: null };
  const metrics: RunMetrics = { runId: 1, sessionId: 1, provider: 'openai', model: 'test', inputTokens: 1000, outputTokens: 0, cacheHitTokens: 999, elapsedMs: 1000, firstTokenMs: 100 };
  const usage = sessionUsage([run], { 1: metrics }, 8000);
  expect(usage.output).toBeGreaterThan(0);
  expect(usage.estimated).toBe(true);
  expect(usage.used).toBeGreaterThan(1000);
  expect(cacheHitPercent(999, 1000)).toBe('99.9');
  expect(cacheHitPercent(1000, 1000)).toBe('100');
  expect(cacheHitPercent(0, 0)).toBeNull();
});

it('derives tok/s only from provider usage, never from text estimates', () => {
  const run: RunView = { id: 2, sessionId: 1, prompt: 'hello', mode: 'code', status: 'completed', output: 'x'.repeat(4000), error: null };
  const metrics: RunMetrics = { runId: 2, sessionId: 1, provider: 'openai', model: 'test', inputTokens: 100, outputTokens: 1000, cacheHitTokens: 50, elapsedMs: 10_000, firstTokenMs: 2_000 };
  expect(sessionUsage([run], { 2: metrics }, 8000).speed).toBe('125.0');
  expect(sessionUsage([run], { 2: { ...metrics, outputTokens: null } }, 8000).speed).toBe('—');
  expect(sessionUsage([run], {}, 8000).speed).toBe('—');
});

it('aggregates speed from reports that have usage, even when some lack timing', () => {
  const base: RunMetrics = { runId: 3, sessionId: 1, provider: 'openai', model: 'test', inputTokens: 10, outputTokens: 500, cacheHitTokens: 5, elapsedMs: 6_000, firstTokenMs: null };
  const runs: RunView[] = [
    { id: 3, sessionId: 1, prompt: 'a', mode: 'code', status: 'completed', output: '', error: null },
    { id: 4, sessionId: 1, prompt: 'b', mode: 'code', status: 'completed', output: '', error: null },
  ];
  expect(sessionUsage(runs, { 3: base, 4: { ...base, runId: 4, outputTokens: 1000, elapsedMs: 4_000, firstTokenMs: 1_000 } }, 8000).speed).toBe('166.7');
  expect(sessionUsage(runs, { 3: base, 4: { ...base, runId: 4, outputTokens: null } }, 8000).speed).toBe('83.3');
});
