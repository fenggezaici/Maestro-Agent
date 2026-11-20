import type { ActivityDay, WorkbenchProject } from './workspace';

export interface DashboardDay extends ActivityDay {
  lines: number;
  tokens: number;
  plans: number;
  shortDate: string;
}

export interface ModelUsage {
  model: string;
  tokens: number;
  runs: number;
}

export interface ModelTrendDay {
  date: string;
  shortDate: string;
  model: string;
  tokens: number;
}

export interface DashboardData {
  source: 'mock' | 'live';
  start: string;
  end: string;
  totalLines: number;
  approvedPlans: number;
  totalTokens: number;
  peakTokens: number;
  longestRunMs: number;
  cost: number | null;
  todayLines: number;
  days: DashboardDay[];
  trend: DashboardDay[];
  models: ModelUsage[];
  modelTrend: ModelTrendDay[];
}

// 活跃度判定：写入行数或 token 消耗任一非零即算当天有活动。
const active = (day: Pick<DashboardDay, 'lines' | 'tokens'>) => day.lines > 0 || day.tokens > 0;

// 连续天数统计：当前连续只算从今天（末格）往回，最长连续取全局峰值。
export function activityStreaks(days: DashboardDay[]): { current: number; longest: number } {
  let current = 0;
  for (let index = days.length - 1; index >= 0 && active(days[index]); index--) current++;
  let longest = 0; let run = 0;
  for (const day of days) { run = active(day) ? run + 1 : 0; if (run > longest) longest = run; }
  return { current, longest };
}

// 热力图聚合口径：daily 原样；weekly 按自然周求和；cumulative 自窗口起点累计。
export function aggregateHeatmap(days: DashboardDay[], mode: 'daily' | 'weekly' | 'cumulative'): DashboardDay[] {
  if (mode === 'daily') return days;
  if (mode === 'cumulative') {
    let sum = 0;
    return days.map((day) => { sum += day.lines; return { ...day, lines: sum }; });
  }
  const offset = new Date(`${days[0].date}T00:00:00Z`).getUTCDay();
  const weekTotals = new Map<number, number>();
  days.forEach((day, index) => {
    const week = Math.floor((offset + index) / 7);
    weekTotals.set(week, (weekTotals.get(week) ?? 0) + day.lines);
  });
  return days.map((day, index) => ({ ...day, lines: weekTotals.get(Math.floor((offset + index) / 7)) ?? 0 }));
}

function allocate(total: number, weights: number[]): number[] {
  const sum = weights.reduce((result, value) => result + value, 0);
  const values = weights.map((weight) => Math.floor(total * weight / sum));
  let remainder = total - values.reduce((result, value) => result + value, 0);
  for (let index = 0; remainder > 0; index = (index + 1) % values.length) {
    if (weights[index] > 0) { values[index]++; remainder--; }
  }
  return values;
}

// Fixed UTC mock period: never derives metrics from live session/run/controller data.
export function createMockDashboard(project: Pick<WorkbenchProject, 'id' | 'today'>): DashboardData {
  const start = Date.UTC(2026, 4, 9);
  const end = Date.UTC(2027, 4, 8);
  const count = Math.round((end - start) / 86400000) + 1;
  const seed = [...project.id].reduce((result, character) => result + character.charCodeAt(0), 0);
  const weights = Array.from({ length: count }, (_, index) => index % 13 === 0 ? 0 : 20 + (index * 29 + seed) % 97);
  const todayLines = Math.min(12450, Math.max(0, project.today?.changedLines ?? 120));
  const todayPlans = Math.min(45, Math.max(0, project.today?.completedPlans ?? 3));
  const lines = [...allocate(12450 - todayLines, weights.slice(0, -1)), todayLines];
  const tokens = allocate(1200000, weights);
  const days: DashboardDay[] = weights.map((_, index) => {
    const date = new Date(start + index * 86400000).toISOString().slice(0, 10);
    return {
      date, shortDate: date.slice(5), lines: lines[index], tokens: tokens[index],
      plans: index === count - 1 ? todayPlans : index % 4 === 0 && index / 4 < 45 - todayPlans ? 1 : 0,
      level: lines[index] === 0 ? 0 : lines[index] < 45 ? 1 : lines[index] < 75 ? 2 : lines[index] < 100 ? 3 : 4,
    };
  });
  const models: ModelUsage[] = [
    { model: 'deepseek-v4.1-flash', tokens: 780000, runs: 32 },
    { model: 'claude-sonnet-4.5', tokens: 320000, runs: 11 },
    { model: 'gpt-5.2', tokens: 100000, runs: 4 },
  ];
  const modelTrend: ModelTrendDay[] = days.slice(-30).flatMap((day) => models.map((entry, index) => ({
    date: day.date, shortDate: day.shortDate, model: entry.model,
    tokens: Math.round(day.tokens * [0.65, 0.25, 0.1][index]),
  })));
  return {
    source: 'mock', start: days[0].date, end: days.at(-1)!.date,
    totalLines: 12450, approvedPlans: 45, totalTokens: 1200000, peakTokens: Math.max(...tokens),
    longestRunMs: 5_370_000, cost: 3.5,
    todayLines,
    days, trend: days.slice(-30), models, modelTrend,
  };
}

export function heatmapWeeks(days: DashboardDay[]): (DashboardDay | null)[][] {
  const offset = new Date(`${days[0].date}T00:00:00Z`).getUTCDay();
  const cells: (DashboardDay | null)[] = [...Array<null>(offset).fill(null), ...days];
  while (cells.length % 7 !== 0) cells.push(null);
  return Array.from({ length: cells.length / 7 }, (_, index) => cells.slice(index * 7, index * 7 + 7));
}



/// Zero-filled live placeholder while the Rust snapshot is loading.
export function createEmptyDashboard(): DashboardData {
  const end = new Date();
  const days: DashboardDay[] = Array.from({ length: 364 }, (_, index) => {
    const date = new Date(Date.UTC(end.getUTCFullYear(), end.getUTCMonth(), end.getUTCDate() - 363 + index)).toISOString().slice(0, 10);
    return { date, shortDate: date.slice(5), lines: 0, tokens: 0, plans: 0, level: 0 };
  });
  return { source: 'live', start: days[0].date, end: days.at(-1)!.date, totalLines: 0, approvedPlans: 0, totalTokens: 0, peakTokens: 0, longestRunMs: 0, cost: null, todayLines: 0, days, trend: days.slice(-30), models: [], modelTrend: [] };
}
