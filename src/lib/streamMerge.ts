/// JS-side stream coalescing — mirrors the Rust `try_merge_stream_event` in
/// lib.rs. Kernel events arrive ~30/s while streaming; merging same-key token
/// events before dispatch keeps `set()`/re-render work bounded so invoke
/// responses and lazy `import()` are not starved by markdown re-parses.
import type { KernelEvent } from '../api/types';

/// Merge `next` into `into` when both carry streamed text for the same target.
/// Returns false for any other pair so callers preserve event order.
export function tryMergeStreamEvent(into: KernelEvent, next: KernelEvent): boolean {
  const mergeable =
    (into.type === 'content' && next.type === 'content' && into.runId === next.runId) ||
    (into.type === 'reasoning' && next.type === 'reasoning' && into.runId === next.runId) ||
    (into.type === 'subagent_token' &&
      next.type === 'subagent_token' &&
      into.runId === next.runId &&
      into.subagentId === next.subagentId &&
      into.kind === next.kind);
  if (mergeable && 'token' in into && 'token' in next) {
    into.token += next.token;
    into.sequence = next.sequence;
  }
  return mergeable;
}
