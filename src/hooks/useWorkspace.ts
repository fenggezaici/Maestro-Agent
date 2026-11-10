/// Workspace hook: subscribes to the workspace store and triggers the
/// persisted-root load the first time the explorer/palette needs it.
import { useEffect } from 'react';

import {
  ensureWorkspaceLoaded, setWorkspaceMounted, useWorkspaceStore,
  type WorkspaceStore,
} from '../stores/useWorkspaceStore';

export function useWorkspace(enabled: boolean): WorkspaceStore {
  useEffect(() => {
    setWorkspaceMounted(true);
    return () => useWorkspaceStore.getState().disconnect();
  }, []);
  useEffect(() => {
    if (enabled) ensureWorkspaceLoaded();
  }, [enabled]);
  return useWorkspaceStore();
}

export type WorkspaceController = WorkspaceStore;
