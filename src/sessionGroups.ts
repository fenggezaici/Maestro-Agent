import type { SessionView } from './api/types';

export interface SessionGroup {
  key: 'pinned' | 'today' | 'yesterday' | 'week' | 'older';
  label: string;
  sessions: SessionView[];
}

const GROUP_LABELS: Record<SessionGroup['key'], string> = {
  pinned: '置顶', today: '今天', yesterday: '昨天', week: '7 天内', older: '更早',
};

function dayStart(now: number, offsetDays: number): number {
  const date = new Date(now);
  date.setHours(0, 0, 0, 0);
  return date.getTime() - offsetDays * 86_400_000;
}

/// Orders sessions into pinned → today → yesterday → 7 days → older buckets.
/// `flat` skips date grouping (preview fixtures carry placeholder timestamps,
/// so the sample list keeps its flat look); pinned ordering still applies.
export function groupSessionsByDate(sessions: SessionView[], pinnedIds: readonly number[] = [], now = Date.now(), flat = false): SessionGroup[] {
  if (sessions.length === 0) return [];
  if (flat) {
    const pinned = sessions.filter((session) => pinnedIds.includes(session.id));
    const rest = sessions.filter((session) => !pinnedIds.includes(session.id));
    const groups: SessionGroup[] = [];
    if (pinned.length) groups.push({ key: 'pinned', label: GROUP_LABELS.pinned, sessions: pinned });
    if (rest.length) groups.push({ key: 'older', label: '', sessions: rest });
    return groups;
  }
  const sorted = [...sessions].sort((a, b) => b.updatedAt - a.updatedAt);
  const pinned = sorted.filter((session) => pinnedIds.includes(session.id));
  const buckets: SessionGroup[] = [
    { key: 'today', label: GROUP_LABELS.today, sessions: [] },
    { key: 'yesterday', label: GROUP_LABELS.yesterday, sessions: [] },
    { key: 'week', label: GROUP_LABELS.week, sessions: [] },
    { key: 'older', label: GROUP_LABELS.older, sessions: [] },
  ];
  const todayStart = dayStart(now, 0);
  const yesterdayStart = dayStart(now, 1);
  const weekStart = dayStart(now, 7);
  for (const session of sorted) {
    if (pinnedIds.includes(session.id)) continue;
    const time = session.updatedAt * 1000;
    if (time >= todayStart) buckets[0].sessions.push(session);
    else if (time >= yesterdayStart) buckets[1].sessions.push(session);
    else if (time >= weekStart) buckets[2].sessions.push(session);
    else buckets[3].sessions.push(session);
  }
  const groups: SessionGroup[] = [];
  if (pinned.length) groups.push({ key: 'pinned', label: GROUP_LABELS.pinned, sessions: pinned });
  return groups.concat(buckets.filter((bucket) => bucket.sessions.length > 0));
}

const pinsKey = (source: 'live' | 'preview') => `maestro.sessions.pins.v1.${source}`;

export function readPinnedSessions(source: 'live' | 'preview'): number[] {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(pinsKey(source)) ?? 'null');
    if (!Array.isArray(value)) return [];
    return value.filter((item): item is number => Number.isInteger(item));
  } catch { return []; }
}

export function savePinnedSessions(source: 'live' | 'preview', ids: number[]) {
  try { localStorage.setItem(pinsKey(source), JSON.stringify(ids.slice(0, 50))); } catch { /* Local ordering only. */ }
}
