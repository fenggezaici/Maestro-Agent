import { useEffect, useMemo, useState } from 'react';
import { useT } from '../i18n';
import * as Tabs from '@radix-ui/react-tabs';
import * as DropdownMenu from '@radix-ui/react-dropdown-menu';
import { Braces, Check, ChevronDown, FileCode2, Folder, FolderOpen, GitCompareArrows, ListTodo, LockKeyhole, PanelsTopLeft, RefreshCw, Rows3 } from 'lucide-react';
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from 'react-resizable-panels';
import type { FileNode } from '../api/workspace';
import { pickDirectory } from '../api/dialog';
import type { PlanCard, PlanStep } from '../api/types';
import type { ReviewFile } from '../lib/conversation';
import type { WorkspaceController } from '../hooks/useWorkspace';
import { useKernelStore } from '../stores/useKernelStore';
import { mockExplorerFiles, type ExplorerFile } from './workbenchMock';
import type { ExplorerViewState } from './ExplorerPanel';
import { countChanges } from './MiniDiff';
import MonacoDiff from './MonacoDiff';
import MonacoCode from './MonacoCode';
import TaskPlanPanel from './TaskPlanPanel';

export type ListTab = 'explorer' | 'changes' | 'tasks';
export interface ActiveFile { kind: 'workspace' | 'change'; path: string }

interface ListNode { id: string; name: string; children: ListNode[]; leaf?: { path: string; file?: ExplorerFile } }

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
        <FileCode2 size={13} />{node.name}</button>
    : <div key={node.id}>
        <button type="button" className="explorer-folder" aria-label={`文件夹 ${node.name}`} aria-expanded={!closed.has(node.id)}
          style={{ paddingLeft: 10 + depth * 13 }} onClick={() => onToggle(node.id)}>
          {closed.has(node.id) ? <Folder size={13} /> : <FolderOpen size={13} />}{node.name}<ChevronDown size={11} className={closed.has(node.id) ? 'folder-chevron closed' : 'folder-chevron'} />
        </button>
        <div hidden={closed.has(node.id)}>
          {node.children.length ? <ListNodes nodes={node.children} depth={depth + 1} closed={closed} onToggle={onToggle} activePath={activePath} onPick={onPick} /> : <p className="explorer-empty-folder" style={{ paddingLeft: 24 + depth * 13 }}>Empty folder</p>}
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
      <FileCode2 size={12} className="review-file-icon" />
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
export default function RightPanel({ listTab, onListTabChange, activeFile, onActiveFile, reviewFiles, plans, planSteps, planRunning, preview, workspace, explorerView, onExplorerViewChange, scope, fontSize, fontFamily }: {
  listTab: ListTab; onListTabChange: (tab: ListTab) => void;
  activeFile: ActiveFile | null; onActiveFile: (file: ActiveFile | null) => void;
  reviewFiles: ReviewFile[]; plans: PlanCard[];
  planSteps: PlanStep[]; planRunning: boolean;
  preview: boolean; workspace: WorkspaceController;
  explorerView: ExplorerViewState; onExplorerViewChange: (view: ExplorerViewState) => void;
  scope: string; fontSize: number; fontFamily: string;
}) {
  const [split, setSplit] = useState(true);
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


  const pickWorkspace = (picked: string, file?: ExplorerFile) => {
    onActiveFile({ kind: 'workspace', path: picked });
    if (!preview) void workspace.selectFile(picked);
  };

  return <aside aria-label="右侧工作区" className="right-pane">
    <PanelGroup orientation="horizontal" className="right-split">
      <Panel defaultSize={240} minSize={160} maxSize="50%" className="right-list-pane">
        <div className="list-header">
          <DropdownMenu.Root>
            <DropdownMenu.Trigger asChild>
              <button type="button" className="list-filter" aria-label="文件列表视图">
                {listTab === 'explorer' ? <Folder size={12} /> : listTab === 'tasks' ? <ListTodo size={12} /> : <GitCompareArrows size={12} />}
                <span>{listTab === 'explorer' ? 'Explorer' : listTab === 'tasks' ? 'Tasks' : 'Changes'}</span>
                <ChevronDown size={12} className="list-filter-chevron" />
              </button>
            </DropdownMenu.Trigger>
            <DropdownMenu.Portal>
              <DropdownMenu.Content className="dropdown-content" align="start" sideOffset={4}>
                <DropdownMenu.Item className="dropdown-item" onSelect={() => onListTabChange('changes')}><span><GitCompareArrows size={12} className="item-menu-icon" /> Changes</span>{listTab === 'changes' && <Check size={12} />}</DropdownMenu.Item>
                <DropdownMenu.Item className="dropdown-item" onSelect={() => onListTabChange('explorer')}><span><Folder size={12} className="item-menu-icon" /> Explorer</span>{listTab === 'explorer' && <Check size={12} />}</DropdownMenu.Item>
                {(planSteps.length > 0 || listTab === 'tasks') && <DropdownMenu.Item className="dropdown-item" onSelect={() => onListTabChange('tasks')}><span><ListTodo size={12} className="item-menu-icon" /> Tasks</span>{listTab === 'tasks' && <Check size={12} />}</DropdownMenu.Item>}
              </DropdownMenu.Content>
            </DropdownMenu.Portal>
          </DropdownMenu.Root>
        </div>
        <div className="list-body">
          {listTab === 'explorer' ? <>
            {!preview && <>
              <div className="list-workspace">
                <span className="list-workspace-name">{workspace.info?.name ?? t.workspace}</span>
                <span className="list-workspace-meta">{workspace.info?.path ? t.localReadOnly : t.noWorkspace}</span>
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
      </Panel>
      <PanelResizeHandle aria-label="调整文件列表宽度" className="resizable-handle"><span /></PanelResizeHandle>
      <Panel minSize={200} className="right-preview-pane">
        <div className="preview-stack">
          <div className="preview-toolbar">
            <span className="preview-toolbar-path">{change?.path ?? workspacePath ?? ''}</span>
            {change && <Tabs.Root value={split ? 'split' : 'inline'} onValueChange={(value) => setSplit(value === 'split')}>
              <Tabs.List aria-label="Diff 布局" className="segmented-tabs">
                <Tabs.Trigger value="split" aria-label="并排差异" className="segmented-tab"><PanelsTopLeft size={13} /></Tabs.Trigger>
                <Tabs.Trigger value="inline" aria-label="行内差异" className="segmented-tab"><Rows3 size={13} /></Tabs.Trigger>
              </Tabs.List>
            </Tabs.Root>}
            <span className="readonly-label"><LockKeyhole size={11} />Read only</span>
          </div>
          {(() => {
            if (change) return <MonacoDiff diff={{ path: change.path, before: change.before, after: change.after }} split={split} fontSize={fontSize} fontFamily={fontFamily} />;
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
      </Panel>
    </PanelGroup>
  </aside>;
}
