/// Kernel facade hook: mounts the agent stream and exposes the kernel store
/// as the Workbench controller — same shape the old useKernel returned.
import { useAgentStream } from './useAgentStream';
import { conversationRunsFor, useKernelStore } from '../stores/useKernelStore';

export function useKernel() {
  useAgentStream();
  const store = useKernelStore();
  return { ...store, runs: conversationRunsFor(store) };
}
