import { describe, expect, it } from 'vitest';
import { CACHE_HIT_RATE_DISPLAY_THRESHOLD, getRenderableContextUsage } from './contextUsage';

describe('context usage rendering', () => {
  it('hides context usage until both prompt and window are valid', () => {
    expect(getRenderableContextUsage(null, 128000)).toBeNull();
    expect(getRenderableContextUsage(77937, null)).toBeNull();
    expect(getRenderableContextUsage(0, 128000)).toBeNull();
    expect(getRenderableContextUsage(77937, 0)).toBeNull();
    expect(getRenderableContextUsage(Number.NaN, 128000)).toBeNull();
    expect(getRenderableContextUsage(77937, 128000)).toEqual({ used: 77937, context: 128000 });
  });

  it('uses the ZCode production cache-hit threshold', () => {
    expect(CACHE_HIT_RATE_DISPLAY_THRESHOLD).toBe(78);
  });
});
