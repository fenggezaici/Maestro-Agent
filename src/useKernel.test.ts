import { describe, expect, it } from 'vitest';
import { conversationRuns, initialSnapshot, kernelReducer, reconcileSnapshot } from './useKernel';
import type { RunView } from './ipc';

describe('IPC state reconciliation', () => {
  const run: RunView = { id: 1, sessionId: 2, prompt: 'hello', mode: 'plan', status: 'running', output: '', error: null };

  it('does not overwrite a new stream with an older snapshot', () => {
    let state = kernelReducer(initialSnapshot, { type: 'event', event: { type: 'status', run, sequence: 2 } });
    state = kernelReducer(state, { type: 'event', event: { type: 'stream_token', runId: 1, token: 'new', sequence: 3 } });
    state = kernelReducer(state, { type: 'snapshot', snapshot: { ...initialSnapshot, ready: true, sequence: 1 } });
    expect(state.ready).toBe(true);
    expect(state.sequence).toBe(3);
    expect(state.activeRun?.output).toBe('new');
  });

  it('ignores stale events and tokens for another run', () => {
    let state = kernelReducer(initialSnapshot, { type: 'event', event: { type: 'status', run, sequence: 2 } });
    state = kernelReducer(state, { type: 'event', event: { type: 'mode_changed', mode: 'execute', sequence: 1 } });
    state = kernelReducer(state, { type: 'event', event: { type: 'stream_token', runId: 99, token: 'foreign', sequence: 3 } });
    expect(state.mode).toBe('plan');
    expect(state.activeRun?.output).toBe('');
  });

  it('restores a mid-run baseline before replaying buffered tokens', () => {
    const lastRun: RunView = { ...run, id: 99, status: 'completed', output: 'previous' };
    const state = reconcileSnapshot({
      ready: true, mode: 'execute', activeRun: { ...run, output: 'baseline' },
      lastRun, sequence: 5,
    }, [
      { type: 'stream_token', runId: 1, token: ' + new', sequence: 6 },
      { type: 'stream_token', runId: 1, token: 'already included', sequence: 5 },
    ]);
    expect(state.activeRun?.output).toBe('baseline + new');
    expect(state.lastRun).toEqual(lastRun);
    expect(state.mode).toBe('execute');
    expect(state.sequence).toBe(6);
  });

  it('keeps earlier turns and overlays live output only in the selected session', () => {
    const saved = { ...run, id: 3, status: 'completed' as const, output: 'saved' };
    const detail = { session: { id: 2, title: 'session', updatedAt: 1, runCount: 2 }, runs: [saved, run], plans: [] };
    const snapshot = { ...initialSnapshot, activeRun: { ...run, output: 'live' }, lastRun: saved };
    expect(conversationRuns(detail, 2, snapshot).map((item) => item.output)).toEqual(['live', 'saved']);
    expect(conversationRuns(detail, 99, snapshot)).toEqual([]);
  });
});
