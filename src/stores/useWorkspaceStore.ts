/// Workspace store (zustand): read-only explorer state — selected root, tree,
/// open file. All IPC goes through `api/workspace`; the store only manages
/// state and request revisions.
import { create } from 'zustand';

import { workspaceApi, type FileNode, type WorkspaceInfo } from '../api/workspace';
import type { ExplorerFile } from '../components/workbenchMock';
import { errorMessage } from '../lib/conversation';

const SAVED_PATH = 'maestro.workspace-path.v1';

/// A file read must settle even when the IPC call never answers; otherwise the
/// preview pane can spin forever with no way to retry.
const FILE_READ_TIMEOUT_MS = 30_000;

/// Recently opened files, so clicking the same path again is instant instead of
/// round-tripping through IPC. Bounded by count and bytes; only small text files
/// are cached (the backend already caps reads at 1 MiB).
const FILE_CACHE_LIMIT = 20;
const FILE_CACHE_MAX_CHARS = 2 * 1024 * 1024;
const FILE_CACHE_MAX_FILE_CHARS = 256 * 1024;
const fileCache = new Map<string, string>();
let fileCacheChars = 0;

const cacheKey = (rootPath: string, path: string) => `${rootPath}:${path}`;

function readCachedFile(rootPath: string, path: string): string | undefined {
  const key = cacheKey(rootPath, path);
  const hit = fileCache.get(key);
  if (hit === undefined) return undefined;
  // LRU touch: re-insert so the newest entry is last.
  fileCache.delete(key);
  fileCache.set(key, hit);
  return hit;
}

function writeCachedFile(rootPath: string, path: string, content: string): void {
  if (content.length > FILE_CACHE_MAX_FILE_CHARS) return;
  const key = cacheKey(rootPath, path);
  const existing = fileCache.get(key);
  if (existing !== undefined) {
    fileCacheChars -= existing.length;
    fileCache.delete(key);
  }
  fileCache.set(key, content);
  fileCacheChars += content.length;
  while (fileCache.size > FILE_CACHE_LIMIT || fileCacheChars > FILE_CACHE_MAX_CHARS) {
    const oldest = fileCache.entries().next().value as [string, string] | undefined;
    if (!oldest) break;
    fileCache.delete(oldest[0]);
    fileCacheChars -= oldest[1].length;
  }
}

/// Test/teardown hook: drop every cached file.
export function clearWorkspaceFileCache(): void {
  fileCache.clear();
  fileCacheChars = 0;
}

function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('读取超时，请重试')), ms);
    promise.then(
      (value) => { clearTimeout(timer); resolve(value); },
      (reason) => { clearTimeout(timer); reject(reason); },
    );
  });
}

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
  selectFile: (path: string, fallbackRoot?: string) => Promise<void>;
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
/// A directory scan can take seconds on a large root. Refresh bursts during an
/// agent write loop used to stack overlapping scans (up to the backend gate
/// limit); serialize loads and replay only the newest queued request once the
/// in-flight load finishes. The flag must flip before the first await or
/// requests arriving during the select/info round-trip would slip through.
let loadInFlight = false;
let pendingLoadPath: { path?: string } | null = null;
/// Same-path file reads share one promise so a double click (or a queued
/// retry) cannot issue a second IPC round-trip for identical work.
const fileReadsInFlight = new Map<string, Promise<string>>();

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
    if (loadInFlight) {
      // A load/scan is already running — keep only the newest request; the
      // in-flight load replays it from its finally block.
      pendingLoadPath = { path };
      return;
    }
    loadInFlight = true;
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
      loadInFlight = false;
      if (pendingLoadPath) {
        const pending = pendingLoadPath;
        pendingLoadPath = null;
        void get().load(pending.path, { force: true });
      }
    }
  },

  selectWorkspace: (path) => get().load(path),
  refresh: () => get().load(undefined, { force: true }),

  selectFile: async (path: string, fallbackRoot?: string) => {
    let workspace = root;
    if (!workspace?.path && fallbackRoot) {
      // A page reload (vite dev refresh) wipes this module state; the
      // mount-time selectWorkspace round-trip may not have landed yet when
      // the user clicks a file. Load the known project root instead of
      // failing the first click outright.
      await get().load(fallbackRoot);
      workspace = root;
    }
    if (!workspace?.path) {
      // Never leave the pane spinning on a workspace that was never loaded.
      set({ selectedPath: path, file: null, fileLoading: false, fileError: '工作区尚未加载，请先选择项目目录' });
      return;
    }
    const cachedContent = readCachedFile(workspace.path, path);
    if (cachedContent !== undefined) {
      set({
        selectedPath: path,
        file: { id: `disk:${workspace.path}:${path}`, path, content: cachedContent, source: 'disk' },
        fileError: null,
        fileLoading: false,
      });
      return;
    }
    const revision = ++fileRevision;
    // Keep the previously opened file on screen while the new one loads; the
    // right pane shows a thin progress bar instead of blanking out.
    set({ selectedPath: path, fileError: null, fileLoading: true });
    try {
      // No artificial deadline: the scoped reader is bounded to 1 MiB and the
      // backend owns real errors. A 10s race used to fire while reads queued
      // behind a directory scan, which looked like a broken file.
      const started = performance.now();
      const readKey = cacheKey(workspace.path, path);
      let read = fileReadsInFlight.get(readKey);
      if (!read) {
        read = withTimeout(workspaceApi.text(workspace.path, path), FILE_READ_TIMEOUT_MS)
          .finally(() => fileReadsInFlight.delete(readKey));
        fileReadsInFlight.set(readKey, read);
      }
      const content = await read;
      const elapsed = Math.round(performance.now() - started);
      if (elapsed > 400) console.info(`[workspace] 读取 ${path} 耗时 ${elapsed}ms`);
      writeCachedFile(workspace.path, path, content);
      if (revision === fileRevision && root?.path === workspace.path) {
        set({ file: { id: `disk:${workspace.path}:${path}`, path, content, source: 'disk' } });
      }
    } catch (reason) {
      if (revision === fileRevision) set({ fileError: errorMessage(reason) });
    } finally {
      // Always settle the spinner for the latest request. `mounted` is a
      // component-lifetime flag; using it here could leave the pane spinning
      // forever after a remount, and a zustand `set` after unmount is harmless.
      if (revision === fileRevision) set({ fileLoading: false });
    }
  },

  disconnect: () => {
    mounted = false;
    clearWorkspaceFileCache();
    fileReadsInFlight.clear();
    pendingLoadPath = null;
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
