import type { SessionView } from '../ipc';

export interface ActivityDay {
  date: string;
  level: 0 | 1 | 2 | 3 | 4;
}

export interface WorkbenchProject {
  id: string;
  name: string;
  sessions: SessionView[];
  activity?: ActivityDay[];
  today?: { changedLines: number; completedPlans: number };
}

export function readExpandedProjects(key: string, projects: WorkbenchProject[]): string[] {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(key) ?? 'null');
    if (Array.isArray(saved) && saved.every((id) => typeof id === 'string')) {
      return saved.filter((id) => projects.some((project) => project.id === id));
    }
  } catch { /* UI preferences must not prevent startup. */ }
  return projects.map((project) => project.id);
}

export function saveExpandedProjects(key: string, values: string[]) {
  try { localStorage.setItem(key, JSON.stringify(values)); } catch { /* Storage may be unavailable. */ }
}
