/// Workspace store (zustand): read-only explorer state — selected root, tree,
/// open file. All IPC goes through `api/workspace`; the store only manages
/// state and request revisions.
import { create } from 'zustand';

import { workspaceApi, type FileNode, type WorkspaceInfo } from '../api/workspace';
import type { ExplorerFile } from '../components/workbenchMock';
import { errorMessage } from '../lib/conversation';

const SAVED_PATH = 'maestro.workspace-path.v1';

export interface WorkspaceStore {
  info: WorkspaceInfo | null;
  tree: FileNode[];
  loading: boolean;
  error: string | null;
  file: ExplorerFile | null;
  fileLoading: boolean;
  fileError: string | null;
  selectedPath: string | null;

  load: (path?: string) => Promise<void>;
  selectWorkspace: (path: string) => Promise<void>;
  refresh: () => Promise<void>;
  selectFile: (path: string) => Promise<void>;
  disconnect: () => void;
}

let treeRevision = 0;
let fileRevision = 0;
let mounted = false;
let root: WorkspaceInfo | null = null;
let initialized = false;

export const useWorkspaceStore = create<WorkspaceStore>()((set, get) => ({
  info: null,
  tree: [],
  loading: false,
  error: null,
  file: null,
  fileLoading: false,
  fileError: null,
  selectedPath: null,

  load: async (path?: string) => {
    const revision = ++treeRevision;
    fileRevision++;
    root = null;
    set({ loading: true, error: null, tree: [], file: null, fileError: null, fileLoading: false, selectedPath: null, info: null });
    try {
      const workspace = path ? await workspaceApi.select(path) : await workspaceApi.info();
      if (!mounted || revision !== treeRevision) return;
      root = workspace;
      set({ info: workspace });
      if (path) { try { localStorage.setItem(SAVED_PATH, workspace.path ?? ''); } catch { /* Selection still succeeded in Rust. */ } }
      if (!workspace.path) return;
      const nodes = await workspaceApi.tree(workspace.path);
      if (mounted && revision === treeRevision) set({ tree: nodes });
    } catch (reason) {
      if (mounted && revision === treeRevision) set({ error: errorMessage(reason) });
    } finally {
      if (mounted && revision === treeRevision) set({ loading: false });
    }
  },

  selectWorkspace: (path) => get().load(path),
  refresh: () => get().load(),

  selectFile: async (path: string) => {
    const workspace = root;
    if (!workspace?.path) return;
    const revision = ++fileRevision;
    set({ selectedPath: path, file: null, fileError: null, fileLoading: true });
    try {
      const content = await workspaceApi.text(workspace.path, path);
      if (mounted && revision === fileRevision && root?.path === workspace.path) {
        set({ file: { id: `disk:${workspace.path}:${path}`, path, content, source: 'disk' } });
      }
    } catch (reason) {
      if (mounted && revision === fileRevision && root?.path === workspace.path) set({ fileError: errorMessage(reason) });
    } finally {
      if (mounted && revision === fileRevision && root?.path === workspace.path) set({ fileLoading: false });
    }
  },

  disconnect: () => {
    mounted = false;
    initialized = false;
    treeRevision++;
    fileRevision++;
    root = null;
    set({
      info: null, tree: [], loading: false, error: null,
      file: null, fileLoading: false, fileError: null, selectedPath: null,
    });
  },
}));

export function setWorkspaceMounted(value: boolean) {
  mounted = value;
}

/** First-enabled load with the persisted path; idempotent across remounts. */
export function ensureWorkspaceLoaded() {
  if (initialized) return;
  initialized = true;
  let saved: string | undefined;
  try { saved = localStorage.getItem(SAVED_PATH) || undefined; } catch { /* No saved choice. */ }
  void useWorkspaceStore.getState().load(saved);
}
