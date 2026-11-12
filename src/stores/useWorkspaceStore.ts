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
  refreshing: boolean;
  error: string | null;
  file: ExplorerFile | null;
  fileLoading: boolean;
  fileError: string | null;
  selectedPath: string | null;

  load: (path?: string, options?: { force?: boolean }) => Promise<void>;
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
/// Last successful scan, keyed by resolved workspace path. Re-opening the
/// Explorer (or remounting the hook) rehydrates from here instead of rescanning.
let cached: { path: string; tree: FileNode[] } | null = null;

export const useWorkspaceStore = create<WorkspaceStore>()((set, get) => ({
  info: null,
  tree: [],
  loading: false,
  refreshing: false,
  error: null,
  file: null,
  fileLoading: false,
  fileError: null,
  selectedPath: null,

  load: async (path?: string, options?: { force?: boolean }) => {
    const force = options?.force === true;
    // Only a genuinely different root invalidates the visible tree; a cached
    // path keeps it on screen while the (cheap) select round-trip runs.
    const switching = path !== undefined && path !== cached?.path;
    const revision = ++treeRevision;
    // Keep the previous root during a background refresh so an open file
    // preview keeps working; only a root switch invalidates it.
    if (switching) { root = null; fileRevision++; }
    set({
      loading: switching || !cached,
      refreshing: !switching && !!cached && force,
      error: null,
      ...(switching ? { info: null, tree: [], file: null, fileLoading: false, fileError: null, selectedPath: null } : {}),
    });
    try {
      const workspace = path ? await workspaceApi.select(path) : await workspaceApi.info();
      if (!mounted || revision !== treeRevision) return;
      root = workspace;
      set({ info: workspace });
      if (path) { try { localStorage.setItem(SAVED_PATH, workspace.path ?? ''); } catch { /* Selection still succeeded in Rust. */ } }
      if (!workspace.path) { cached = null; set({ tree: [] }); return; }
      if (!force && cached?.path === workspace.path) { set({ tree: cached.tree }); return; }
      const nodes = await workspaceApi.tree(workspace.path);
      if (mounted && revision === treeRevision) {
        cached = { path: workspace.path, tree: nodes };
        set({ tree: nodes });
      }
    } catch (reason) {
      // Keep the last successful tree so a failed background refresh does not
      // blank the explorer; the error message explains what happened.
      if (mounted && revision === treeRevision) set({ error: errorMessage(reason) });
    } finally {
      if (mounted && revision === treeRevision) set({ loading: false, refreshing: false });
    }
  },

  selectWorkspace: (path) => get().load(path),
  refresh: () => get().load(undefined, { force: true }),

  selectFile: async (path: string) => {
    const workspace = root;
    if (!workspace?.path) return;
    const revision = ++fileRevision;
    set({ selectedPath: path, file: null, fileError: null, fileLoading: true });
    let timer: number | undefined;
    try {
      const timeout = new Promise<never>((_, reject) => {
        timer = window.setTimeout(() => reject(new Error('读取文件超时')), 10_000);
      });
      const content = await Promise.race([workspaceApi.text(workspace.path, path), timeout]);
      if (mounted && revision === fileRevision && root?.path === workspace.path) {
        set({ file: { id: `disk:${workspace.path}:${path}`, path, content, source: 'disk' } });
      }
    } catch (reason) {
      if (mounted && revision === fileRevision) set({ fileError: errorMessage(reason) });
    } finally {
      if (timer) window.clearTimeout(timer);
      // Always settle the spinner for the latest request, even if the root
      // changed mid-read; otherwise the right pane can hang on "loading".
      if (mounted && revision === fileRevision) set({ fileLoading: false });
    }
  },

  disconnect: () => {
    mounted = false;
    initialized = false;
    treeRevision++;
    fileRevision++;
    root = null;
    cached = null;
    set({
      info: null, tree: [], loading: false, refreshing: false, error: null,
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
