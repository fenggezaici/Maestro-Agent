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
