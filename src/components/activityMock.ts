import type { ActivityDay, WorkbenchProject } from './workspace';

export interface DashboardDay extends ActivityDay {
  lines: number;
  tokens: number;
  plans: number;
  shortDate: string;
}

export interface MockDashboardData {
  source: 'mock';
  start: string;
  end: string;
  totalLines: number;
  approvedPlans: number;
  totalTokens: number;
  cost: number;
  todayLines: number;
  days: DashboardDay[];
  trend: DashboardDay[];
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
export function createMockDashboard(project: Pick<WorkbenchProject, 'id' | 'today'>): MockDashboardData {
  const start = Date.UTC(2026, 4, 9);
  const end = Date.UTC(2026, 10, 8);
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
  return {
    source: 'mock', start: days[0].date, end: days.at(-1)!.date,
    totalLines: 12450, approvedPlans: 45, totalTokens: 1200000, cost: 3.5,
    todayLines,
    days, trend: days.slice(-30),
  };
}

export function heatmapWeeks(days: DashboardDay[]): (DashboardDay | null)[][] {
  const offset = new Date(`${days[0].date}T00:00:00Z`).getUTCDay();
  const cells: (DashboardDay | null)[] = [...Array<null>(offset).fill(null), ...days];
  while (cells.length % 7 !== 0) cells.push(null);
  return Array.from({ length: cells.length / 7 }, (_, index) => cells.slice(index * 7, index * 7 + 7));
}

