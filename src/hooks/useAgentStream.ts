/// Agent stream hook: owns the `kernel-event` subscription, replay buffer,
/// and baseline reconciliation. Live effects (refresh on status/sessions)
/// stay here; state writes go through the kernel store.
import { startTransition, useEffect } from 'react';

import { kernelApi } from '../api/kernel';
import type { KernelEvent } from '../api/types';
import { errorMessage, reconcileSnapshot } from '../lib/conversation';
import { tryMergeStreamEvent } from '../lib/streamMerge';
import { useKernelStore } from '../stores/useKernelStore';

/// Merge streamed token events into one dispatch per flush window. Without
/// this, ~30 events/s each rebuild liveTimeline and re-parse the streaming
/// markdown — the webview main thread saturates and invoke responses /
/// lazy `import()` starve (架构沙盘卡在「加载规则…」的病根之一).
const FLUSH_MS = 40;
/// vitest 下同步 flush，保持现有测试「事件即生效」的时序语义。
const SYNC_FLUSH = Boolean(import.meta.env?.VITEST);

export function useAgentStream() {
  useEffect(() => {
    const store = useKernelStore.getState();
    store.setConnected(true);
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let buffered: KernelEvent[] | null = [];
    let lastEventSequence = 0;
    let pending: KernelEvent[] = [];
    let flushTimer: ReturnType<typeof setTimeout> | null = null;

    function apply(event: KernelEvent) {
      if (event.type !== 'client_error') {
        if (event.sequence <= lastEventSequence) return;
        lastEventSequence = event.sequence;
      }
      const store = useKernelStore.getState();
      // 高频流式事件降级为可中断渲染：zustand 状态写入本身仍同步保序，
      // startTransition 只把订阅者渲染标为低优先级——点击文件/面板交互
      // 的 urgent 渲染得以插队，不再排在逐 token 的重渲队列后数秒。
      if (event.type === 'content' || event.type === 'reasoning' || event.type === 'subagent_token') {
        startTransition(() => store.dispatch({ type: 'event', event }));
      } else {
        store.dispatch({ type: 'event', event });
      }
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
    }

    function flush() {
      flushTimer = null;
      const batch = pending;
      pending = [];
      for (const event of batch) apply(event);
    }

    function enqueue(event: KernelEvent) {
      // A reload can attach mid-run. Do not apply deltas before the full baseline.
      if (buffered !== null) { buffered.push(event); return; }
      if (SYNC_FLUSH) { apply(event); return; }
      // 非流式事件同样入队：原序 flush 天然保序，代价只是 ≤FLUSH_MS 延迟。
      const last = pending[pending.length - 1];
      if (!last || !tryMergeStreamEvent(last, event)) pending.push(event);
      if (!flushTimer) flushTimer = setTimeout(flush, FLUSH_MS);
    }

    async function connect() {
      try {
        const stop = await kernelApi.listen((event) => {
          if (disposed) return;
          enqueue(event);
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
      if (flushTimer) clearTimeout(flushTimer);
      useKernelStore.getState().disconnect();
      unlisten?.();
    };
  }, []);
}
