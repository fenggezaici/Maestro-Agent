import { describe, expect, it } from 'vitest';
import { conversationRuns, initialSnapshot, kernelReducer, pendingApproval, reconcileSnapshot } from './lib/conversation';
import type { RunView } from './api/types';

describe('IPC state reconciliation', () => {
  const run: RunView = { id: 1, sessionId: 2, prompt: 'hello', mode: 'plan', status: 'running', output: '', error: null };

  it('does not overwrite a new stream with an older snapshot', () => {
    let state = kernelReducer(initialSnapshot, { type: 'event', event: { type: 'status', run, sequence: 2 } });
    state = kernelReducer(state, { type: 'event', event: { type: 'content', runId: 1, sessionId: 2, token: 'new', sequence: 3 } });
    state = kernelReducer(state, { type: 'snapshot', snapshot: { ...initialSnapshot, ready: true, sequence: 1 } });
    expect(state.ready).toBe(true);
    expect(state.sequence).toBe(3);
    expect(state.activeRun?.output).toBe('new');
  });

  it('ignores stale events and tokens for another run', () => {
    let state = kernelReducer(initialSnapshot, { type: 'event', event: { type: 'status', run, sequence: 2 } });
    state = kernelReducer(state, { type: 'event', event: { type: 'mode_changed', mode: 'auto', sequence: 1 } });
    state = kernelReducer(state, { type: 'event', event: { type: 'content', runId: 99, sessionId: 2, token: 'foreign', sequence: 3 } });
    expect(state.mode).toBe('code');
    expect(state.activeRun?.output).toBe('');
  });

  it('restores a mid-run baseline before replaying buffered tokens', () => {
    const lastRun: RunView = { ...run, id: 99, status: 'completed', output: 'previous' };
    const state = reconcileSnapshot({
      ready: true, mode: 'code', activeRun: { ...run, output: 'baseline' },
      lastRun, sequence: 5,
    }, [
      { type: 'content', runId: 1, sessionId: 2, token: ' + new', sequence: 6 },
      { type: 'content', runId: 1, sessionId: 2, token: 'already included', sequence: 5 },
    ]);
    expect(state.activeRun?.output).toBe('baseline + new');
    expect(state.lastRun).toEqual(lastRun);
    expect(state.mode).toBe('code');
    expect(state.sequence).toBe(6);
  });

  it('keeps earlier turns and overlays live output only in the selected session', () => {
    const saved = { ...run, id: 3, status: 'completed' as const, output: 'saved' };
    const detail = { session: { id: 2, title: 'session', updatedAt: 1, runCount: 2, projectId: 1, archived: false }, runs: [saved, run], plans: [] };
    const snapshot = { ...initialSnapshot, activeRun: { ...run, output: 'live' }, lastRun: saved };
    expect(conversationRuns(detail, 2, snapshot).map((item) => item.output)).toEqual(['live', 'saved']);
    expect(conversationRuns(detail, 99, snapshot)).toEqual([]);
  });
});

describe('pendingApproval', () => {
  it('finds the single awaiting call and decodes its preview', () => {
    expect(pendingApproval(undefined)).toBeNull();
    expect(pendingApproval({ 1: [{ runId: 1, tool: 'read_file', detail: '', allowed: true, pending: true }] })).toBeNull();
    const view = pendingApproval({ 7: [{ runId: 7, tool: 'write_file', detail: '{"kind":"write","path":"a.txt","exists":false,"old":"","new":"hi"}', allowed: true, awaitingApproval: true }] });
    expect(view?.runId).toBe(7);
    expect(view?.preview?.kind).toBe('write');
    const cmd = pendingApproval({ 2: [{ runId: 2, tool: 'run_command', detail: '{"kind":"command","command":"npm test"}', allowed: true, awaitingApproval: true }] });
    expect(cmd?.preview?.kind === 'command' && cmd.preview.command).toBe('npm test');
  });
});
