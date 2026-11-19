import { describe, expect, it } from 'vitest';

import { appendReasoning, appendTool, applyToolEventIndexed, closeReasoning, initialSnapshot, kernelReducer, relativeTime } from './conversation';

it('formats relative time for the turn preview card', () => {
  const now = 1_700_000_000_000;
  expect(relativeTime(undefined, now)).toBe('刚刚');
  expect(relativeTime(now / 1000 - 30, now)).toBe('刚刚');
  expect(relativeTime(now / 1000 - 120, now)).toBe('2 分钟前');
  expect(relativeTime(now / 1000 - 7200, now)).toBe('2 小时前');
  expect(relativeTime(now / 1000 - 172800, now)).toBe('2 天前');
});

it('splits reasoning bursts around tool calls and seals each one', () => {
  let items = appendReasoning(undefined, 'a', 100);
  items = appendReasoning(items, 'b', 110);
  expect(items).toEqual([{ kind: 'reasoning', segment: { text: 'ab', startedAt: 100 } }]);

  items = appendTool(items, 0, 200);
  expect(items).toEqual([
    { kind: 'reasoning', segment: { text: 'ab', startedAt: 100, endedAt: 200 } },
    { kind: 'tool', index: 0 },
  ]);

  items = appendReasoning(items, 'c', 300);
  expect(items).toHaveLength(3);
  expect(items[2]).toEqual({ kind: 'reasoning', segment: { text: 'c', startedAt: 300 } });

  const sealed = closeReasoning(items, 450);
  expect(sealed?.[2]).toEqual({ kind: 'reasoning', segment: { text: 'c', startedAt: 300, endedAt: 450 } });
  expect(closeReasoning(sealed, 500)).toBe(sealed);
  expect(appendTool(sealed, 0, 600)).toEqual(sealed);
});

it('returns the touched tool row index so the timeline keeps event order', () => {
  const started = applyToolEventIndexed({}, { sequence: 1, type: 'tool_call', runId: 1, tool: 'read_file', phase: 'started', detail: 'a.ts' });
  expect(started.index).toBe(0);
  expect(started.next[1]).toHaveLength(1);
  expect(started.next[1][0]).toMatchObject({ tool: 'read_file', pending: true, target: 'a.ts' });

  const finished = applyToolEventIndexed(started.next, { sequence: 2, type: 'tool_call', runId: 1, tool: 'read_file', phase: 'finished', detail: 'ok' });
  expect(finished.index).toBe(0);
  expect(finished.next[1]).toHaveLength(1);
  expect(finished.next[1][0]).toMatchObject({ detail: 'ok', pending: undefined, target: 'a.ts' });
});

it('advances the snapshot on task_plan_updated without touching run state', () => {
  const event = {
    sequence: 3,
    type: 'task_plan_updated' as const,
    runId: 7,
    sessionId: 2,
    steps: [{ id: '1', description: '分析项目', status: 'in_progress' as const }],
  };
  const next = kernelReducer({ ...initialSnapshot, ready: true, sequence: 2 }, { type: 'event', event });
  expect(next.sequence).toBe(3);
  expect(next.activeRun).toBeNull();
  expect(next.ready).toBe(true);
});

describe('subagent tool rows', () => {
  it('keeps subagentId across started and finished phases', () => {
    const started = applyToolEventIndexed({}, { sequence: 1, type: 'tool_call', runId: 1, tool: 'spawn_subagent', phase: 'started', detail: '调研', subagentId: 'sa-1-1' });
    const finished = applyToolEventIndexed(started.next, { sequence: 2, type: 'tool_call', runId: 1, tool: 'spawn_subagent', phase: 'finished', detail: 'done', subagentId: 'sa-1-1' });
    expect(finished.next[1][0].subagentId).toBe('sa-1-1');
  });
});

describe('pickSummaryRun', () => {
  const run = (id: number, sessionId: number) => ({ id, sessionId, status: 'completed', output: '', mode: 'code' }) as never;

  it('scopes activeRun/lastRun to the selected session', async () => {
    const { pickSummaryRun } = await import('./conversation');
    const foreign = run(7, 1);
    const mine = run(3, 2);
    // Another session's lastRun must not leak — fall back to this session's runs.
    expect(pickSummaryRun({ activeRun: null, lastRun: foreign, runs: [mine], sessionId: 2 })?.id).toBe(3);
    // Fresh session with no runs → null, no stale card.
    expect(pickSummaryRun({ activeRun: null, lastRun: foreign, runs: [], sessionId: 2 })).toBeNull();
    // Own session's activeRun wins.
    expect(pickSummaryRun({ activeRun: mine, lastRun: foreign, runs: [], sessionId: 2 })?.id).toBe(3);
  });
});

describe('reviewFiles write_result counts', () => {
  const detail = (added: number, removed: number, after = 'x', netAdded?: number, netRemoved?: number, path = 'a.py') =>
    JSON.stringify({ kind: 'write_result', path, before: '', after, added, removed, net_added: netAdded, net_removed: netRemoved });
  const call = (runId: number, detailStr: string) => ({
    runId, tool: 'write_file', detail: detailStr, allowed: true, pending: false, awaitingApproval: false,
  }) as never;

  it('carries kernel added/removed and swaps in net counts when merging a second write', async () => {
    const { reviewFiles } = await import('./conversation');
    const files = reviewFiles({ liveTools: { 1: [call(1, detail(346, 0, 'x', 346, 0))] }, pending: null, plans: [], runId: 1 });
    expect(files[0].added).toBe(346);
    expect(files[0].netAdded).toBe(346);

    const merged = reviewFiles({
      liveTools: { 1: [call(1, detail(346, 0, 'x', 346, 0)), call(1, detail(5, 1, 'y', 340, 6))] },
      pending: null, plans: [], runId: 1,
    });
    expect(merged).toHaveLength(1);
    expect(merged[0].after).toBe('y');
    expect(merged[0].added).toBeUndefined();
    expect(merged[0].netAdded).toBe(340);
    expect(merged[0].netRemoved).toBe(6);
  });

  it('scopes applied rows to the summary run so earlier runs never leak', async () => {
    const { reviewFiles } = await import('./conversation');
    const liveTools = {
      1: [call(1, detail(54, 0, 'v1', 54, 0, 'verify.py'))],
      2: [call(2, detail(14, 4, 'v2', 14, 4, 'rig.py'))],
    };
    const files = reviewFiles({ liveTools, pending: null, plans: [], runId: 2 });
    expect(files.map((file) => file.path)).toEqual(['rig.py']);
    // No runId → no applied rows at all (pending/proposal still work).
    expect(reviewFiles({ liveTools, pending: null, plans: [] })).toEqual([]);
  });
});
