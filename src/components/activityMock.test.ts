import { expect, it } from 'vitest';
import { createMockDashboard, heatmapWeeks } from './activityMock';

it('provides a deterministic six-month dataset with consistent KPIs and thirty trend days', () => {
  const data = createMockDashboard({ id: 'maestro', today: { changedLines: 120, completedPlans: 3 } });
  expect(data.source).toBe('mock');
  expect(data.start).toBe('2026-05-09');
  expect(data.end).toBe('2026-11-08');
  expect(data.days.length).toBeGreaterThan(180);
  expect(data.trend).toHaveLength(30);
  expect(data.days.reduce((sum, day) => sum + day.lines, 0)).toBe(data.totalLines);
  expect(data.days.reduce((sum, day) => sum + day.tokens, 0)).toBe(data.totalTokens);
  expect(data.days.reduce((sum, day) => sum + day.plans, 0)).toBe(data.approvedPlans);
  expect(data.days.at(-1)).toMatchObject({ lines: 120, plans: 3 });
  const weeks = heatmapWeeks(data.days);
  expect(weeks.every((week) => week.length === 7)).toBe(true);
  expect(weeks.flat().filter(Boolean)).toEqual(data.days);
  expect(createMockDashboard({ id: 'maestro', today: { changedLines: 120, completedPlans: 3 } })).toEqual(data);
});

it('uses explicit mock defaults in a live workspace without session-derived metrics', () => {
  const data = createMockDashboard({ id: 'local' });
  expect(data.source).toBe('mock');
  expect(data.totalLines).toBe(12450);
  expect(data.totalTokens).toBe(1200000);
  expect(data.todayLines).toBe(120);
});
