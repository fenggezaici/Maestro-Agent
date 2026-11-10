import { beforeEach, describe, expect, it } from 'vitest';
import { groupSessionsByDate, readPinnedSessions, savePinnedSessions } from './sessionGroups';
import type { SessionView } from './api/types';

function session(id: number, updatedAt: number, overrides: Partial<SessionView> = {}): SessionView {
  return { id, title: `会话 ${id}`, updatedAt, runCount: 1, projectId: 1, archived: false, ...overrides };
}

const NOW = new Date('2026-11-10T15:00:00').getTime();
const DAY = 86_400_000;

beforeEach(() => localStorage.clear());

describe('groupSessionsByDate', () => {
  it('buckets sessions into today / yesterday / week / older by local day', () => {
    const groups = groupSessionsByDate([
      session(1, (NOW - 3600_000) / 1000),
      session(2, (NOW - DAY - 3600_000) / 1000),
      session(3, (NOW - 3 * DAY) / 1000),
      session(4, (NOW - 30 * DAY) / 1000),
    ], [], NOW);
    expect(groups.map((group) => [group.key, group.sessions.map((item) => item.id)])).toEqual([
      ['today', [1]], ['yesterday', [2]], ['week', [3]], ['older', [4]],
    ]);
  });

  it('sorts newest first inside each bucket and lifts pinned sessions to the top group', () => {
    const groups = groupSessionsByDate([
      session(1, (NOW - 2 * DAY) / 1000),
      session(2, (NOW - 3600_000) / 1000),
      session(3, (NOW - 7200_000) / 1000),
    ], [1], NOW);
    expect(groups.map((group) => group.key)).toEqual(['pinned', 'today']);
    expect(groups[0].sessions[0].id).toBe(1);
    expect(groups[1].sessions.map((item) => item.id)).toEqual([2, 3]);
  });

  it('keeps preview sample data flat with an unlabeled rest group', () => {
    const groups = groupSessionsByDate([session(1, 1), session(2, 1), session(3, 1)], [2], NOW, true);
    expect(groups.map((group) => [group.key, group.label])).toEqual([['pinned', '置顶'], ['older', '']]);
    expect(groups[1].sessions.map((item) => item.id)).toEqual([1, 3]);
  });

  it('returns no groups for an empty list', () => {
    expect(groupSessionsByDate([], [1], NOW)).toEqual([]);
  });
});

describe('pinned sessions storage', () => {
  it('round-trips ids per source and ignores malformed entries', () => {
    savePinnedSessions('live', [3, 7]);
    savePinnedSessions('preview', [1]);
    expect(readPinnedSessions('live')).toEqual([3, 7]);
    expect(readPinnedSessions('preview')).toEqual([1]);
    localStorage.setItem('maestro.sessions.pins.v1.live', '["x",4,null]');
    expect(readPinnedSessions('live')).toEqual([4]);
  });
});
