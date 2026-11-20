import { describe, expect, it } from 'vitest';
import { activityStreaks, aggregateHeatmap, type DashboardDay } from './activityMock';

const day = (date: string, lines: number, tokens = 0): DashboardDay =>
  ({ date, shortDate: date.slice(5), lines, tokens, plans: 0, level: 0 });

describe('activityStreaks', () => {
  it('counts current streak from today and the longest streak overall', () => {
    const days = [
      day('2026-11-10', 5), day('2026-11-11', 5), day('2026-11-12', 5),
      day('2026-11-13', 0), day('2026-11-14', 0),
      day('2026-11-15', 3), day('2026-11-16', 0, 800),
    ];
    expect(activityStreaks(days)).toEqual({ current: 2, longest: 3 });
  });

  it('returns zero current streak when today is inactive', () => {
    const days = [day('2026-11-10', 5), day('2026-11-11', 0)];
    expect(activityStreaks(days)).toEqual({ current: 0, longest: 1 });
  });
});

describe('aggregateHeatmap', () => {
  // days 必须连续密集（与后端输出一致）：11-16(一)..11-21 为周0，11-22(日)..11-24 为周1。
  const days = [
    ...Array.from({ length: 6 }, (_, i) => day(`2026-11-${16 + i}`, 10)),
    ...Array.from({ length: 3 }, (_, i) => day(`2026-11-${22 + i}`, 30)),
  ];

  it('daily returns days unchanged', () => {
    expect(aggregateHeatmap(days, 'daily').map((d) => d.lines)).toEqual(days.map((d) => d.lines));
  });

  it('cumulative accumulates lines from the window start', () => {
    const result = aggregateHeatmap(days, 'cumulative');
    expect(result[0].lines).toBe(10);
    expect(result.at(-1)!.lines).toBe(150);
  });

  it('weekly sums each calendar week into every cell', () => {
    expect(aggregateHeatmap(days, 'weekly').map((d) => d.lines)).toEqual([60, 60, 60, 60, 60, 60, 90, 90, 90]);
  });
});
