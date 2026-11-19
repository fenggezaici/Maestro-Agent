import { describe, expect, it } from 'vitest';

import type { KernelEvent } from '../api/types';
import { tryMergeStreamEvent } from './streamMerge';

const content = (sequence: number, runId: number, token: string): KernelEvent => ({ sequence, type: 'content', runId, sessionId: 1, token });
const reasoning = (sequence: number, runId: number, token: string): KernelEvent => ({ sequence, type: 'reasoning', runId, token });
const subToken = (sequence: number, kind: 'content' | 'reasoning', token: string): KernelEvent => ({ sequence, type: 'subagent_token', runId: 1, subagentId: 's1', kind, token });

describe('tryMergeStreamEvent', () => {
  it('merges adjacent content tokens for the same run and keeps the latest sequence', () => {
    const into = content(1, 7, 'a');
    const next = content(2, 7, 'b');
    expect(tryMergeStreamEvent(into, next)).toBe(true);
    expect(into).toMatchObject({ type: 'content', runId: 7, token: 'ab', sequence: 2 });
  });

  it('refuses content events for different runs', () => {
    expect(tryMergeStreamEvent(content(1, 7, 'a'), content(2, 8, 'b'))).toBe(false);
  });

  it('merges reasoning tokens per run', () => {
    const into = reasoning(3, 4, 'r1');
    expect(tryMergeStreamEvent(into, reasoning(4, 4, 'r2'))).toBe(true);
    expect(into).toMatchObject({ token: 'r1r2' });
  });

  it('does not merge reasoning into content of the same run', () => {
    expect(tryMergeStreamEvent(content(1, 7, 'a'), reasoning(2, 7, 'b'))).toBe(false);
  });

  it('merges subagent tokens only for the same subagent and kind', () => {
    const into = subToken(1, 'reasoning', 'x');
    expect(tryMergeStreamEvent(into, subToken(2, 'reasoning', 'y'))).toBe(true);
    expect(into).toMatchObject({ token: 'xy' });
    expect(tryMergeStreamEvent(subToken(1, 'reasoning', 'x'), subToken(2, 'content', 'y'))).toBe(false);
    expect(tryMergeStreamEvent(subToken(1, 'reasoning', 'x'), { sequence: 2, type: 'subagent_token', runId: 1, subagentId: 's2', kind: 'reasoning', token: 'y' })).toBe(false);
  });

  it('never merges non-stream events', () => {
    const status: KernelEvent = { sequence: 5, type: 'sessions_changed' };
    expect(tryMergeStreamEvent(content(1, 7, 'a'), status)).toBe(false);
    expect(tryMergeStreamEvent(status, content(2, 7, 'a'))).toBe(false);
  });
});
