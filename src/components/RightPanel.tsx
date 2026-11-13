import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useT } from '../i18n';
import * as DropdownMenu from '@radix-ui/react-dropdown-menu';
import { Braces, ChevronDown, File, FileCode2, FileJson, FileText, Folder, FolderOpen, GitCompareArrows, Image as ImageIcon, ListTodo, LockKeyhole, Plus, RefreshCw, X } from 'lucide-react';
import SplitDivider, { STORAGE_KEY } from './SplitDivider';
import type { FileNode } from '../api/workspace';
import { pickDirectory } from '../api/dialog';
import type { PlanCard, PlanStep } from '../api/types';
import type { ReviewFile } from '../lib/conversation';
import type { WorkspaceController } from '../hooks/useWorkspace';
import { useKernelStore } from '../stores/useKernelStore';
import { mockExplorerFiles, type ExplorerFile } from './workbenchMock';
import type { ExplorerViewState } from './ExplorerPanel';
import { countChanges } from './MiniDiff';
import LightweightDiffPreview from './LightweightDiffPreview';
import MonacoCode from './MonacoCode';
import TaskPlanPanel from './TaskPlanPanel';
import { preloadMonaco } from '../editor/loader';

export type ListTab = 'explorer' | 'changes' | 'tasks';
export interface ActiveFile { kind: 'workspace' | 'change'; path: string }

interface ListNode { id: string; name: string; children: ListNode[]; leaf?: { path: string; file?: ExplorerFile } }

const CODE_EXTENSIONS = new Set(['ts', 'tsx', 'js', 'jsx', 'rs', 'py', 'cpp', 'cc', 'c', 'h', 'hpp', 'cs', 'go', 'java', 'rb', 'php', 'swift', 'kt', 'sh', 'sql']);
const DATA_EXTENSIONS = new Set(['json', 'toml', 'yaml', 'yml']);
const TEXT_EXTENSIONS = new Set(['md', 'txt', 'log', 'csv']);
const IMAGE_EXTENSIONS = new Set(['png', 'jpg', 'jpeg', 'webp', 'gif', 'svg', 'ico']);

function FileIcon({ path, size = 13, className }: { path: string; size?: number; className?: string }) {
  const extension = path.split('.').pop()?.toLowerCase() ?? '';
  if (CODE_EXTENSIONS.has(extension)) return <FileCode2 size={size} className={className} />;
  if (DATA_EXTENSIONS.has(extension)) return <FileJson size={size} className={className} />;
  if (TEXT_EXTENSIONS.has(extension)) return <FileText size={size} className={className} />;
  if (IMAGE_EXTENSIONS.has(extension)) return <ImageIcon size={size} className={className} />;
  return <File size={size} className={className} />;
}

function liveTree(nodes: FileNode[]): ListNode[] {
  return nodes.map((node) => ({
    id: node.path,
    name: node.name,
    leaf: node.is_dir ? undefined : { path: node.path },
    children: node.children ? liveTree(node.children) : [],
  }));
}

/// Flat path list → folder tree (dirs first, then files, alphabetical).
function flatTree(files: ExplorerFile[], scope: string): ListNode[] {
  const roots: ListNode[] = [];
  const folders = new Map<string, ListNode>();
  for (const file of files) {
    const parts = file.path.split('/');
    let level = roots, prefix = '';
    for (const [index, part] of parts.entries()) {
      prefix = prefix ? `${prefix}/${part}` : part;
      let node = level.find((item) => item.name === part && (index === parts.length - 1) === !!item.leaf);
      if (!node) {
        node = { id: index === parts.length - 1 ? `${scope}:${file.id}` : `${scope}:dir:${prefix}`, name: part, children: [] };
        if (index !== parts.length - 1) folders.set(prefix, node);
        level.push(node);
      }
      if (index === parts.length - 1) node.leaf = { path: file.path, file };
      level = node.children;
    }
  }
  const sort = (list: ListNode[]) => { list.sort((a, b) => Number(!!a.leaf) - Number(!!b.leaf) || a.name.localeCompare(b.name)); list.forEach((item) => sort(item.children)); };
  sort(roots);
  return roots;
}

function ListNodes({ nodes, depth, closed, onToggle, activePath, onPick }: {
  nodes: ListNode[]; depth: number; closed: Set<string>; onToggle: (id: string) => void;
  activePath: string | null; onPick: (path: string, file?: ExplorerFile) => void;
}) {
  return <>{nodes.map((node) => node.leaf
    ? <button key={node.id} type="button" className="explorer-file" aria-label={`文件 ${node.leaf.path}`} aria-pressed={activePath === node.leaf.path}
        style={{ paddingLeft: 12 + depth * 13 }} onClick={() => onPick(node.leaf!.path, node.leaf!.file)}>
        <FileIcon path={node.leaf.path} />{node.name}</button>
    : <div key={node.id}>
        <button type="button" className="explorer-folder" aria-label={`文件夹 ${node.name}`} aria-expanded={!closed.has(node.id)}
          style={{ paddingLeft: 10 + depth * 13 }} onClick={() => onToggle(node.id)}>
          {closed.has(node.id) ? <Folder size={13} /> : <FolderOpen size={13} />}{node.name}<ChevronDown size={11} className={closed.has(node.id) ? 'folder-chevron closed' : 'folder-chevron'} />
        </button>
        <div hidden={closed.has(node.id)}>
          {node.children.length ? <ListNodes nodes={node.children} depth={depth + 1} closed={closed} onToggle={onToggle} activePath={activePath} onPick={onPick} /> : <p className="explorer-empty-folder" style={{ paddingLeft: 24 + depth * 13 }}>空文件夹</p>}
        </div>
      </div>)}</>;
}

const STATUS_LABEL: Record<ReviewFile['status'], string> = { pending: '待批准', applied: '已应用', proposal: '提议' };

function ChangesList({ files, activePath, onPick }: { files: ReviewFile[]; activePath: string | null; onPick: (path: string) => void }) {
  if (!files.length) return <p className="review-files-empty">暂无变更</p>;
  return <>{files.map((item) => {
    const { added, removed } = countChanges(item.before, item.after);
    const slash = item.path.lastIndexOf('/');
    const basename = slash >= 0 ? item.path.slice(slash + 1) : item.path;
    const dir = slash >= 0 ? item.path.slice(0, slash + 1) : '';
    return <button key={`${item.status}-${item.path}`} type="button" aria-label={item.path}
      className={`review-file ${activePath === item.path ? 'active' : ''}`} aria-pressed={activePath === item.path}
      onClick={() => onPick(item.path)}>
      <FileIcon path={item.path} size={12} className="review-file-icon" />
      <span className="review-file-name"><strong>{basename}</strong>{dir && <span className="review-file-dir">{dir}</span>}</span>
      <em className={`review-file-mark ${item.status}`}>{STATUS_LABEL[item.status]}</em>
      <span className="tool-call-delta"><em className="add">+{added}</em><em className="del">−{removed}</em></span>
    </button>;
  })}</>;
}

/// Unified workspace panel: fixed left list (Explorer tree OR Changes list,
/// toggled by a compact segmented header) + a right preview column that shows
/// MonacoCode for workspace files, MonacoDiff for changed files, and a slim
/// approval bar while a call is suspended.
export default function RightPanel({ listTab, onListTabChange, openTabs, onAddTab, onCloseTab, activeFile, onActiveFile, reviewFiles, plans, planSteps, planRunning, preview, workspace, explorerView, onExplorerViewChange, scope, fontSize, fontFamily }: {
  listTab: ListTab; onListTabChange: (tab: ListTab) => void;
  openTabs: ListTab[]; onAddTab: (tab: ListTab) => void; onCloseTab: (tab: ListTab) => void;
  activeFile: ActiveFile | null; onActiveFile: (file: ActiveFile | null) => void;
  reviewFiles: ReviewFile[]; plans: PlanCard[];
  planSteps: PlanStep[]; planRunning: boolean;
  preview: boolean; workspace: WorkspaceController;
  explorerView: ExplorerViewState; onExplorerViewChange: (view: ExplorerViewState) => void;
  scope: string; fontSize: number; fontFamily: string;
}) {
  const t = useT();
  // Agent write/command side effects bump this tick — coalesce bursts into one
  // refresh so a long tool loop cannot keep the tree permanently "loading".
  const tick = useKernelStore((s) => s.workspaceTick);
  useEffect(() => {
    if (tick === 0) return;
    const timer = window.setTimeout(() => {
      if (workspace.info) void workspace.refresh();
    }, 400);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only a new tick should schedule a refresh.
  }, [tick]);

  const closed = useMemo(() => new Set(explorerView.closedFolders), [explorerView.closedFolders]);
  const toggleFolder = (id: string) => {
    const next = new Set(closed);
    next.has(id) ? next.delete(id) : next.add(id);
    onExplorerViewChange({ ...explorerView, closedFolders: [...next] });
  };

  // Preview-mode explorer entries: mock files overlaid with plan proposals.
  const previewFiles = useMemo(() => {
    if (!preview) return [];
    const items = new Map(mockExplorerFiles.map((file) => [file.path, { ...file, id: `${scope}:mock:${file.id}` }]));
    for (const plan of plans) {
      for (const diff of plan.draft.diffs) {
        items.set(diff.path, { id: `${scope}:proposal:${plan.id}:${diff.path}`, path: diff.path, content: diff.after, source: 'proposal' });
      }
    }
    return [...items.values()];
  }, [scope, plans, preview]);

  const tree = useMemo<ListNode[]>(() => preview ? flatTree(previewFiles, scope) : liveTree(workspace.tree), [preview, previewFiles, scope, workspace.tree]);

  const workspacePath = activeFile?.kind === 'workspace' ? activeFile.path : null;
  const change = activeFile?.kind === 'change'
    ? reviewFiles.find((item) => item.path === activeFile.path)
    : (activeFile ? undefined : reviewFiles[0]);
  const previewFile = preview && workspacePath ? previewFiles.find((item) => item.path === workspacePath) : undefined;
  const liveFile = !preview && workspacePath ? (workspace.file?.path === workspacePath ? workspace.file : undefined) : undefined;
  const previewPath = change?.path ?? workspacePath ?? '';


  const pickWorkspace = (picked: string, file?: ExplorerFile) => {
    onActiveFile({ kind: 'workspace', path: picked });
    if (!preview) void workspace.selectFile(picked);
  };

  const TAB_META: Record<ListTab, { label: string; icon: ReactNode }> = {
    explorer: { label: '文件', icon: <Folder size={12} /> },
    changes: { label: '变更', icon: <GitCompareArrows size={12} /> },
    tasks: { label: '任务', icon: <ListTodo size={12} /> },
  };
  const noTabs = openTabs.length === 0;
  const [listWidth, setListWidth] = useState(() => { try { return Number(localStorage.getItem(STORAGE_KEY)) || 240; } catch { return 240; } });
  const splitRef = useRef<HTMLDivElement>(null);
  const commitWidth = (width: number) => { setListWidth(width); try { localStorage.setItem(STORAGE_KEY, String(width)); } catch { /* quota */ } };
  useEffect(() => { splitRef.current?.style.setProperty('--right-list-width', `${listWidth}px`); }, [listWidth]);

  return <aside aria-label="右侧工作区" className="right-pane" onMouseEnter={preloadMonaco} onFocusCapture={preloadMonaco}>
        <div className="right-tabs-toolbar">
          <div className="right-tabs-shell">
            <div className="right-tabs" role="tablist" aria-label="右侧面板视图">
              {openTabs.map((tab) => <div key={tab} className="right-tab-wrap">
                <button type="button" role="tab" aria-selected={listTab === tab}
                  className="right-tab" onClick={() => onListTabChange(tab)}>{TAB_META[tab].icon}<span>{TAB_META[tab].label}</span></button>
                <button type="button" className="right-tab-close" aria-label={`关闭 ${TAB_META[tab].label}`}
                  onClick={() => onCloseTab(tab)}><X size={11} /></button>
              </div>)}
            </div>
            <DropdownMenu.Root>
              <DropdownMenu.Trigger asChild>
                <button type="button" className="right-tab-add" aria-label="添加视图"><Plus size={14} /></button>
              </DropdownMenu.Trigger>
              <DropdownMenu.Portal>
                <DropdownMenu.Content className="dropdown-content" align="end" sideOffset={4}>
                  {(Object.keys(TAB_META) as ListTab[]).map((tab) => <DropdownMenu.Item key={tab} className="dropdown-item"
                    disabled={openTabs.includes(tab)} onSelect={() => onAddTab(tab)}>
                    <span className="choice-item-main">{TAB_META[tab].icon}{TAB_META[tab].label}</span>
                    {openTabs.includes(tab) && <span className="right-tab-open-tag">已打开</span>}
                  </DropdownMenu.Item>)}
                </DropdownMenu.Content>
              </DropdownMenu.Portal>
            </DropdownMenu.Root>
          </div>
        </div>
    <div ref={splitRef} className="right-split">
      <div className="right-list-pane">
        <div className="list-body">
          {noTabs ? <div className="right-tabs-empty"><p>点击右侧 + 添加视图</p></div> : listTab === 'explorer' ? <>
            {!preview && <>
              <div className="list-workspace">
                <span className="list-workspace-name">{workspace.info?.name ?? t.workspace}</span>
                <span className="list-workspace-meta">{workspace.info?.path ? '本地 / 只读' : '未选择工作区'}</span>
                <button type="button" className="icon-button" aria-label="打开工作区" disabled={workspace.loading || workspace.refreshing}
                  onClick={async () => { const picked = await pickDirectory(); if (picked) void workspace.selectWorkspace(picked); }}><FolderOpen size={12} /></button>
                <button type="button" className="icon-button" aria-label="刷新文件树" disabled={workspace.loading || workspace.refreshing} onClick={() => void workspace.refresh()}><RefreshCw size={12} /></button>
              </div>
              {workspace.loading && workspace.tree.length === 0 && <p className="workspace-status" role="status">正在读取本地文件树…</p>}
              {workspace.error && <p className="workspace-read-error" role="alert">{workspace.error}</p>}
            </>}
            <nav aria-label="文件浏览器" className="explorer-tree">
              <ListNodes nodes={tree} depth={0} closed={closed} onToggle={toggleFolder} activePath={workspacePath} onPick={pickWorkspace} />
              {!preview && !workspace.loading && !workspace.error && workspace.tree.length === 0 && <p className="workspace-status">{workspace.info?.path ? '该目录为空，或只有已忽略的构建目录。' : '输入项目绝对路径后打开工作区。'}</p>}
            </nav>
          </> : listTab === 'tasks' ? <div className="task-plan-list-pane">
            {planSteps.length > 0
              ? <TaskPlanPanel steps={planSteps} running={planRunning} variant="list" />
              : <p className="review-files-empty">暂无任务计划</p>}
          </div> : <nav aria-label="变更文件列表" className="review-files">
            <ChangesList files={reviewFiles} activePath={change?.path ?? null} onPick={(picked) => onActiveFile({ kind: 'change', path: picked })} />
          </nav>}
        </div>
      </div>
      <SplitDivider containerRef={splitRef} onCommit={commitWidth} />
      <div className="right-preview-pane">
        <div className="preview-stack">
          <div className="preview-toolbar">
            <span className="preview-toolbar-path" title={previewPath}>
              {noTabs ? '' : listTab === 'tasks' ? '任务计划' : previewPath ? previewPath.split('/').map((part, index, all) => <span key={`${part}-${index}`}
                className={index === all.length - 1 ? 'preview-path-leaf' : 'preview-path-dir'}>{part}{index < all.length - 1 ? ' / ' : ''}</span>) : ''}
            </span>
            {!noTabs && listTab !== 'tasks' && <span className="readonly-label"><LockKeyhole size={11} />只读</span>}
          </div>
          {noTabs
            ? <div className="editor-empty"><Braces size={34} strokeWidth={1} /><h3>没有打开的视图</h3><p>点击右侧 + 添加文件、变更或任务。</p></div>
            : listTab === 'tasks'
            ? <div className="editor-empty"><ListTodo size={34} strokeWidth={1} /><h3>任务计划</h3><p>左侧列出当前会话的步骤与进度。</p></div>
            : (() => {
              if (change) return <LightweightDiffPreview key={`${change.path}:${change.before}:${change.after}`} path={change.path} before={change.before} after={change.after} fontSize={fontSize} />;
              if (previewFile) return <MonacoCode file={previewFile} fontSize={fontSize} fontFamily={fontFamily} />;
              if (liveFile) return <MonacoCode file={liveFile} fontSize={fontSize} fontFamily={fontFamily} />;
              if (!preview && workspacePath) {
                if (workspace.fileLoading) return <div className="editor-empty" role="status">正在读取文件内容…</div>;
                if (workspace.fileError) return <div className="editor-empty" role="alert">{workspace.fileError}</div>;
                return <div className="editor-empty">选择一个本地 UTF-8 文件以只读预览。</div>;
              }
              return <div className="editor-empty"><Braces size={34} strokeWidth={1} /><h3>你的下一次变更，从这里开始</h3><p>选择左侧文件以查看内容或差异。</p></div>;
            })()}
        </div>
      </div>
    </div>
  </aside>;
}
