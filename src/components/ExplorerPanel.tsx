import { ChevronDown, FileCode2, Folder } from 'lucide-react';
import { useMemo } from 'react';
import { useT } from '../i18n';
import MonacoCode from './MonacoCode';
import type { ExplorerFile } from './workbenchMock';

export interface ExplorerViewState { selected: string | null; closedFolders: string[] }
interface TreeNode { id: string; name: string; children: TreeNode[]; file?: ExplorerFile }

export function fileTree(files: ExplorerFile[], scope: string): TreeNode[] {
  const roots: TreeNode[] = [];
  for (const file of files) {
    const parts = file.path.split('/');
    if (parts.some((part) => !part || part === '..' || part === '.')) continue;
    let children = roots;
    for (let index = 0; index < parts.length; index++) {
      const id = `${scope}:${parts.slice(0, index + 1).join('/')}`;
      let node = children.find((item) => item.id === id);
      if (!node) { node = { id, name: parts[index], children: [] }; children.push(node); }
      if (index === parts.length - 1) node.file = file;
      children = node.children;
    }
  }
  const sort = (nodes: TreeNode[]) => { nodes.sort((a, b) => Number(!!a.file) - Number(!!b.file) || a.name.localeCompare(b.name)); nodes.forEach((node) => sort(node.children)); };
  sort(roots);
  return roots;
}

export default function ExplorerPanel({ files, scope, view, onViewChange, fontSize, fontFamily }: {
  files: ExplorerFile[]; scope: string; view: ExplorerViewState; onViewChange: (value: ExplorerViewState) => void; fontSize: number; fontFamily: string;
}) {
  const tree = useMemo(() => fileTree(files, scope), [files, scope]);
  const t = useT();
  const selected = files.find((file) => file.id === view.selected) ?? files[0];
  function nodes(items: TreeNode[], depth = 0) {
    return <ul>{items.map((node) => <li key={node.id}>
      {node.file ? <button type="button" className="explorer-file" aria-pressed={selected?.id === node.file.id} style={{ paddingLeft: 12 + depth * 13 }} onClick={() => onViewChange({ ...view, selected: node.file!.id })}><FileCode2 size={13} />{node.name}<span>{node.file.source === 'mock' ? 'M' : 'P'}</span></button>
        : <><button type="button" className="explorer-folder" aria-label={`文件夹 ${node.name}`} aria-expanded={!view.closedFolders.includes(node.id)} style={{ paddingLeft: 10 + depth * 13 }} onClick={() => onViewChange({ ...view, closedFolders: view.closedFolders.includes(node.id) ? view.closedFolders.filter((id) => id !== node.id) : [...view.closedFolders, node.id] })}><ChevronDown size={11} /><Folder size={13} />{node.name}</button><div hidden={view.closedFolders.includes(node.id)}>{nodes(node.children, depth + 1)}</div></>}
    </li>)}</ul>;
  }
  return <section aria-label="文件浏览器" className="explorer-pane">
    <div className="explorer-summary"><strong>{t.workspaceFiles}</strong><span className="mock-badge">{t.virtual}</span></div>
    <nav aria-label="虚拟文件树" className="explorer-tree">{nodes(tree)}</nav>
    {selected ? <><div className="explorer-file-heading"><FileCode2 size={13} /><span>{selected.path}</span><small>{selected.source === 'mock' ? 'Mock' : 'Proposal'}</small></div><MonacoCode file={selected} fontSize={fontSize} fontFamily={fontFamily} /></>
      : <div className="editor-empty">没有可预览的文件。</div>}
    <p className="review-safety-note">虚拟文件树与提议内容，不代表磁盘读取。不写入文件。</p>
  </section>;
}
