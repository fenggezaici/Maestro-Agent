import type { SessionView } from '../ipc';

export interface ActivityDay {
  date: string;
  level: 0 | 1 | 2 | 3 | 4;
}

export interface WorkbenchProject {
  id: string;
  name: string;
  path?: string | null;
  archived?: boolean;
  sessions: SessionView[];
  archivedSessions?: SessionView[];
  activity?: ActivityDay[];
  today?: { changedLines: number; completedPlans: number };
}

/// Unix-second timestamps become compact relative ages; preview fixtures use 1.
export function relativeSessionAge(updatedAt: number, source?: 'preview' | 'live', now = Date.now()): string {
  if (source === 'preview' || updatedAt <= 0) return '示例';
  const seconds = Math.max(0, Math.floor(now / 1000 - updatedAt));
  if (seconds < 90) return '刚刚';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} 分钟前`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} 小时前`;
  const days = Math.floor(hours / 24);
  if (days < 365) return `${days} 天前`;
  return `${Math.floor(days / 365)} 年前`;
}
