import { useEffect, useRef, useState } from 'react';
import { workspaceApi, type FileNode, type WorkspaceInfo } from './workspaceApi';
import type { ExplorerFile } from './components/workbenchMock';

const SAVED_PATH = 'maestro.workspace-path.v1';
const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);

export function useWorkspace(enabled: boolean) {
  const [info, setInfo] = useState<WorkspaceInfo | null>(null);
  const [tree, setTree] = useState<FileNode[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [file, setFile] = useState<ExplorerFile | null>(null);
  const [fileLoading, setFileLoading] = useState(false);
  const [fileError, setFileError] = useState<string | null>(null);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const mounted = useRef(false);
  const initialized = useRef(false);
  const root = useRef<WorkspaceInfo | null>(null);
  const treeRevision = useRef(0);
  const fileRevision = useRef(0);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; initialized.current = false; treeRevision.current++; fileRevision.current++; };
  }, []);

  async function load(path?: string) {
    const revision = ++treeRevision.current;
    fileRevision.current++;
    setLoading(true); setError(null); setTree([]); setFile(null); setFileError(null); setFileLoading(false); setSelectedPath(null);
    root.current = null; setInfo(null);
    try {
      const workspace = path ? await workspaceApi.select(path) : await workspaceApi.info();
      if (!mounted.current || revision !== treeRevision.current) return;
      root.current = workspace; setInfo(workspace);
      if (path) { try { localStorage.setItem(SAVED_PATH, workspace.path ?? ''); } catch { /* Selection still succeeded in Rust. */ } }
      if (!workspace.path) return;
      const nodes = await workspaceApi.tree(workspace.path);
      if (mounted.current && revision === treeRevision.current) setTree(nodes);
    } catch (reason) {
      if (mounted.current && revision === treeRevision.current) setError(errorText(reason));
    } finally {
      if (mounted.current && revision === treeRevision.current) setLoading(false);
    }
  }

  useEffect(() => {
    if (!enabled || initialized.current) return;
    initialized.current = true;
    let saved: string | undefined;
    try { saved = localStorage.getItem(SAVED_PATH) || undefined; } catch { /* No saved choice. */ }
    void load(saved);
  }, [enabled]);

  async function selectFile(path: string) {
    const workspace = root.current;
    if (!workspace?.path) return;
    const revision = ++fileRevision.current;
    setSelectedPath(path); setFile(null); setFileError(null); setFileLoading(true);
    try {
      const content = await workspaceApi.text(workspace.path, path);
      if (mounted.current && revision === fileRevision.current && root.current?.path === workspace.path) {
        setFile({ id: `disk:${workspace.path}:${path}`, path, content, source: 'disk' });
      }
    } catch (reason) {
      if (mounted.current && revision === fileRevision.current && root.current?.path === workspace.path) setFileError(errorText(reason));
    } finally {
      if (mounted.current && revision === fileRevision.current && root.current?.path === workspace.path) setFileLoading(false);
    }
  }

  return { info, tree, loading, error, file, fileLoading, fileError, selectedPath,
    selectWorkspace: (path: string) => load(path),
    refresh: () => load(), selectFile,
  };
}

export type WorkspaceController = ReturnType<typeof useWorkspace>;
