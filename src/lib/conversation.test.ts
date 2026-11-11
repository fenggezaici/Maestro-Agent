import { expect, it } from 'vitest';

import { appendReasoning, appendTool, applyToolEventIndexed, closeReasoning } from './conversation';

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
