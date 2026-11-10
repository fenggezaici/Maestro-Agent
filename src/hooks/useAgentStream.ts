/// Agent stream hook: owns the `kernel-event` subscription, replay buffer,
/// and baseline reconciliation. Live effects (refresh on status/sessions)
/// stay here; state writes go through the kernel store.
import { useEffect } from 'react';

import { kernelApi } from '../api/kernel';
import type { KernelEvent } from '../api/types';
import { errorMessage, reconcileSnapshot } from '../lib/conversation';
import { useKernelStore } from '../stores/useKernelStore';

export function useAgentStream() {
  useEffect(() => {
    const store = useKernelStore.getState();
    store.setConnected(true);
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let buffered: KernelEvent[] | null = [];
    let lastEventSequence = 0;
    async function connect() {
      try {
        const stop = await kernelApi.listen((event) => {
          if (disposed) return;
          // A reload can attach mid-run. Do not apply deltas before the full baseline.
          if (buffered !== null) { buffered.push(event); return; }
          if (event.type !== 'client_error') {
            if (event.sequence <= lastEventSequence) return;
            lastEventSequence = event.sequence;
          }
          const store = useKernelStore.getState();
          store.dispatch({ type: 'event', event });
          store.applySideEffects(event);
          if (event.type === 'backend_error') {
            void kernelApi.snapshot().then((snapshot) => {
              if (!disposed) useKernelStore.getState().dispatch({ type: 'snapshot', snapshot });
            }).catch(() => { /* Retain the explicit storage failure instead of masking it. */ });
          }
          if (event.type === 'sessions_changed') {
            void store.refreshSessions().catch((reason: unknown) => { if (!disposed) useKernelStore.setState({ error: errorMessage(reason) }); });
            void store.refreshProjects().catch(() => { /* Session data still refreshed above. */ });
          }
          if (event.type === 'plan_updated' || (event.type === 'status' && event.run.status !== 'running')) {
            void store.refreshDetail().catch((reason: unknown) => {
              if (!disposed) useKernelStore.setState({ error: errorMessage(reason) });
            });
          }
        });
        if (disposed) { stop(); return; }
        unlisten = stop;
        const snapshot = await kernelApi.snapshot();
        if (disposed) return;
        const reconciled = reconcileSnapshot(snapshot, buffered ?? []);
        lastEventSequence = reconciled.sequence;
        const store = useKernelStore.getState();
        store.dispatch({ type: 'snapshot', snapshot: reconciled });
        for (const event of buffered ?? []) {
          store.applySideEffects(event);
        }
        if (!reconciled.ready && reconciled.activeRun?.error) {
          useKernelStore.setState({ error: reconciled.activeRun.error });
        }
        if (reconciled.activeRun) {
          const current = useKernelStore.getState();
          if (current.selectedSessionId === null) current.selectSession(reconciled.activeRun.sessionId);
        }
        buffered = null;
        await store.refreshSessions();
        await store.refreshProjects();
      } catch (reason) {
        if (!disposed) {
          useKernelStore.setState({ error: `无法连接 Rust 后端，请通过 npm run tauri dev 启动。${errorMessage(reason)}` });
        }
      }
    }
    void connect();
    return () => {
      disposed = true;
      useKernelStore.getState().disconnect();
      unlisten?.();
    };
  }, []);
}
