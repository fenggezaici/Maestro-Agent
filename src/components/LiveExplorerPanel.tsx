import { useEffect, useState } from 'react';
import { ChevronDown, FileCode2, Folder, FolderOpen, RefreshCw } from 'lucide-react';
import type { FileNode } from '../workspaceApi';
import type { WorkspaceController } from '../useWorkspace';
import type { ExplorerViewState } from './ExplorerPanel';
import MonacoCode from './MonacoCode';

export default function LiveExplorerPanel({ workspace, view, onViewChange, fontSize, fontFamily }: {
  workspace: WorkspaceController; view: ExplorerViewState; onViewChange: (view: ExplorerViewState) => void; fontSize: number; fontFamily: string;
}) {
  const [path, setPath] = useState('');
  useEffect(() => { if (workspace.info?.path) setPath(workspace.info.path); }, [workspace.info?.path]);
  const scope = `disk:${workspace.info?.path ?? ''}`;
  function nodes(items: FileNode[], depth = 0) {
    return <ul>{items.map((node) => {
      const id = `${scope}:${node.path}`;
      const closed = view.closedFolders.includes(id);
      return <li key={node.path}>
        {node.is_dir ? <><button type="button" className="explorer-folder" aria-label={`文件夹 ${node.name}`} aria-expanded={!closed} style={{ paddingLeft: 10 + depth * 13 }}
          onClick={() => onViewChange({ ...view, closedFolders: closed ? view.closedFolders.filter((value) => value !== id) : [...view.closedFolders, id] })}><ChevronDown size={11} /><Folder size={13} />{node.name}</button>
          <div hidden={closed}>{node.children.length ? nodes(node.children, depth + 1) : <p className="explorer-empty-folder" style={{ paddingLeft: 24 + depth * 13 }}>Empty folder</p>}</div></>
          : <button type="button" aria-label={`文件 ${node.path}`} className="explorer-file" aria-pressed={workspace.selectedPath === node.path} style={{ paddingLeft: 12 + depth * 13 }} onClick={() => void workspace.selectFile(node.path)}><FileCode2 size={13} />{node.name}</button>}
      </li>;
    })}</ul>;
  }
  return <section aria-label="文件浏览器" className="explorer-pane">
    <div className="explorer-summary"><strong>{workspace.info?.name ?? 'WORKSPACE'}</strong><span className="mock-badge">Local / Read only</span></div>
    <form className="workspace-path-bar" onSubmit={(event) => { event.preventDefault(); if (path.trim()) void workspace.selectWorkspace(path.trim()); }}>
      <input aria-label="本地工作区路径" value={path} onChange={(event) => setPath(event.target.value)} placeholder="Absolute project folder…" disabled={workspace.loading} />
      <button type="submit" className="icon-button" aria-label="打开工作区" disabled={workspace.loading || !path.trim()}><FolderOpen size={14} /></button>
      <button type="button" className="icon-button" aria-label="刷新文件树" disabled={workspace.loading} onClick={() => void workspace.refresh()}><RefreshCw size={13} /></button>
    </form>
    {workspace.loading && <p className="workspace-status" role="status">正在读取本地文件树…</p>}
    {workspace.error && <p className="workspace-read-error" role="alert">{workspace.error}</p>}
    <nav aria-label="本地文件树" className="explorer-tree">{nodes(workspace.tree)}</nav>
    {!workspace.loading && !workspace.error && workspace.tree.length === 0 && <p className="workspace-status">{workspace.info?.path ? '该目录为空，或只有已忽略的构建目录。' : '输入项目绝对路径后打开工作区。'}</p>}
    {workspace.fileLoading && <div className="editor-empty" role="status">正在读取文件内容…</div>}
    {workspace.fileError && <div className="editor-empty" role="alert">{workspace.fileError}</div>}
    {workspace.file && <><div className="explorer-file-heading"><FileCode2 size={13} /><span>{workspace.file.path}</span><small>Disk</small></div><MonacoCode file={workspace.file} fontSize={fontSize} fontFamily={fontFamily} /></>}
    {!workspace.file && !workspace.fileLoading && !workspace.fileError && <div className="editor-empty"><FileCode2 size={25} /><p>选择一个本地 UTF-8 文件以只读预览。</p></div>}
    <p className="review-safety-note">实际磁盘内容 · 只读。不自动加入模型上下文，不应用 Review 提议。</p>
  </section>;
}
