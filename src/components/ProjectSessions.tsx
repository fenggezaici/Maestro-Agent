import { useState } from 'react';
import * as DropdownMenu from '@radix-ui/react-dropdown-menu';
import { Archive, ArchiveRestore, ChevronRight, FolderGit2, MoreHorizontal, Pencil, Pin, PinOff, Plus, Search, Settings2, Trash2 } from 'lucide-react';
import { IconButton } from './ui';
import { groupSessionsByDate } from '../sessionGroups';
import { relativeSessionAge, type WorkbenchProject } from './workspace';
import type { SessionView } from '../api/types';

interface Props {
  projects: WorkbenchProject[];
  currentProjectId: string;
  selectedSessionId: number | null;
  activeSessionId?: number;
  source: 'preview' | 'live';
  canCreate: boolean;
  pinnedIds?: number[];
  onSelect: (id: number) => void;
  onCreate: (projectId: string) => void;
  onTogglePin?: (sessionId: number, pinned: boolean) => void;
  onRenameSession?: (sessionId: number, title: string) => void;
  onArchiveSession?: (sessionId: number, archived: boolean) => void;
  onArchiveProject?: (projectId: string, archived: boolean) => void;
  onDeleteProject?: (projectId: string) => void;
  onDeleteSession?: (sessionId: number) => void;
  onSelectProject?: (projectId: string) => void;
  onNewProject?: () => void;
  onRenameProject?: (projectId: number, name: string) => void;
  onTogglePinProject?: (projectId: string) => void;
  pinnedProjectIds?: string[];
  onManageProject?: () => void;
}

function SessionMenu({ session, pinned, onRename, onArchive, onTogglePin, onDelete }: {
  session: SessionView;
  pinned: boolean;
  onRename?: (session: SessionView) => void;
  onArchive?: (sessionId: number, archived: boolean) => void;
  onTogglePin?: (sessionId: number, pinned: boolean) => void;
  onDelete?: (sessionId: number) => void;
}) {
  if (!onRename && !onArchive && !onTogglePin && !onDelete) return null;
  return <DropdownMenu.Root>
    <DropdownMenu.Trigger className="item-menu" aria-label={`会话 ${session.id} 操作`} onClick={(event) => event.stopPropagation()}><MoreHorizontal size={13} /></DropdownMenu.Trigger>
    <DropdownMenu.Portal><DropdownMenu.Content sideOffset={4} align="end" className="dropdown-content">
      {onTogglePin && <DropdownMenu.Item className="dropdown-item" onSelect={() => onTogglePin(session.id, !pinned)}>
        <span className="choice-item-main">{pinned ? <PinOff size={13} className="item-menu-icon" /> : <Pin size={13} className="item-menu-icon" />}{pinned ? '取消置顶' : '置顶'}</span>
      </DropdownMenu.Item>}
      {!session.archived && onRename && <DropdownMenu.Item className="dropdown-item" onSelect={() => onRename(session)}><span className="choice-item-main"><Pencil size={13} className="item-menu-icon" />重命名</span></DropdownMenu.Item>}
      {onArchive && <DropdownMenu.Item className="dropdown-item" onSelect={() => onArchive(session.id, !session.archived)}>
        <span className="choice-item-main">{session.archived ? <ArchiveRestore size={13} className="item-menu-icon" /> : <Archive size={13} className="item-menu-icon" />}{session.archived ? '恢复' : '归档'}</span>
      </DropdownMenu.Item>}
      {onDelete && <DropdownMenu.Item className="dropdown-item dropdown-item-danger" onSelect={() => {
        if (window.confirm(`删除会话「${session.title || session.id}」及其全部消息记录？此操作不可撤销。`)) onDelete(session.id);
      }}><span className="choice-item-main"><Trash2 size={13} className="item-menu-icon" />删除</span></DropdownMenu.Item>}
    </DropdownMenu.Content></DropdownMenu.Portal>
  </DropdownMenu.Root>;
}

export default function ProjectSessions({ projects, currentProjectId, selectedSessionId, activeSessionId, source, canCreate, pinnedIds = [], onSelect, onCreate, onSelectProject, onNewProject, onRenameProject, onTogglePinProject, pinnedProjectIds = [], onManageProject, onTogglePin, onRenameSession, onArchiveSession, onDeleteSession, onDeleteProject }: Props) {
  const [renamingProjectId, setRenamingProjectId] = useState<string | null>(null);
  const [projectDraft, setProjectDraft] = useState('');
  const [search, setSearch] = useState('');
  const [renamingId, setRenamingId] = useState<number | null>(null);
  const [renameDraft, setRenameDraft] = useState('');
  const [archivedOpen, setArchivedOpen] = useState(false);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const query = search.trim().toLowerCase();

  function startRename(session: SessionView) {
    setRenamingId(session.id);
    setRenameDraft(session.title);
  }

  function commitRename() {
    if (renamingId === null) return;
    const title = renameDraft.trim();
    if (title) onRenameSession?.(renamingId, title);
    setRenamingId(null);
  }

  function sessionRow(session: SessionView, archived: boolean) {
    const pinned = pinnedIds.includes(session.id);
    if (renamingId === session.id && !archived) {
      return <div key={session.id} className="session-row">
        <input className="session-rename-input" aria-label={`重命名会话 ${session.id}`} autoFocus value={renameDraft}
          onChange={(event) => setRenameDraft(event.target.value)}
          onBlur={commitRename}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing) return;
            if (event.key === 'Enter') commitRename();
            if (event.key === 'Escape') setRenamingId(null);
          }} />
      </div>;
    }
    return <div key={session.id} className="session-row">
      <button type="button" className={`session-item${archived ? ' archived' : ''}`} aria-pressed={session.id === selectedSessionId} onClick={() => onSelect(session.id)}>
        <span className={`session-indicator ${activeSessionId === session.id ? 'working' : ''}`} />
        <span className="session-copy"><span className="session-title">{session.title}{pinned && <Pin size={10} className="session-pin-icon" />}</span><span className="session-subtitle">{session.runCount} 轮对话 · {relativeSessionAge(session.updatedAt, source)}<span>#{session.id}</span></span></span>
      </button>
      <SessionMenu session={session} pinned={pinned} onTogglePin={archived ? undefined : onTogglePin} onRename={onRenameSession ? startRename : undefined} onArchive={onArchiveSession} onDelete={onDeleteSession} />
    </div>;
  }

  function projectNode(project: WorkbenchProject) {
    const expanded = !collapsed.has(project.id);
    const groups = query ? [] : groupSessionsByDate(project.sessions, pinnedIds, Date.now(), source === 'preview');
    return <div key={project.id} className={`project-node ${project.id === currentProjectId ? 'active' : ''}`}>
      <div className="project-row">
        <button type="button" className="project-toggle" aria-expanded={expanded} aria-label={`项目 ${project.name}`}
          onClick={(event) => {
            if ((event.target as HTMLElement).closest('.project-add-btn')) return;
            setCollapsed((value) => { const next = new Set(value); next.has(project.id) ? next.delete(project.id) : next.add(project.id); return next; });
            if (project.id !== currentProjectId) onSelectProject?.(project.id);
          }}>
          {expanded ? <ChevronRight size={13} className="project-chevron open" /> : <ChevronRight size={13} className="project-chevron" />}
          <FolderGit2 size={14} className="project-folder-icon" />
          <span className="project-name">{project.name}</span>
          <span className="project-count">{project.sessions.length}</span>
        </button>
        {canCreate && <button type="button" className="project-add-btn" aria-label={`在项目 ${project.name} 下新建会话`} onClick={(event) => { event.stopPropagation(); onCreate(project.id); }}><Plus size={12} /></button>}
        <DropdownMenu.Root>
          <DropdownMenu.Trigger className="project-menu-btn" aria-label={`项目 ${project.name} 操作`} onClick={(event) => event.stopPropagation()}><MoreHorizontal size={13} /></DropdownMenu.Trigger>
          <DropdownMenu.Portal><DropdownMenu.Content sideOffset={4} align="end" className="dropdown-content">
            <DropdownMenu.Item className="dropdown-item" onSelect={() => { setRenamingProjectId(project.id); setProjectDraft(project.name); }}><span className="choice-item-main"><Pencil size={13} className="item-menu-icon" />编辑项目</span></DropdownMenu.Item>
            {onTogglePinProject && <DropdownMenu.Item className="dropdown-item" onSelect={() => onTogglePinProject(project.id)}><span className="choice-item-main">{pinnedProjectIds.includes(project.id) ? <PinOff size={13} className="item-menu-icon" /> : <Pin size={13} className="item-menu-icon" />}{pinnedProjectIds.includes(project.id) ? '取消置顶' : '置顶'}</span></DropdownMenu.Item>}
            {onDeleteProject && <DropdownMenu.Item className="dropdown-item dropdown-item-danger" onSelect={() => { if (window.confirm(`删除项目「${project.name}」及其全部会话？此操作不可撤销。`)) onDeleteProject(project.id); }}><span className="choice-item-main"><Trash2 size={13} className="item-menu-icon" />删除项目</span></DropdownMenu.Item>}
            <DropdownMenu.Separator className="subtle-separator" />
            {onManageProject && <DropdownMenu.Item className="dropdown-item" onSelect={onManageProject}><span className="choice-item-main"><Settings2 size={13} className="item-menu-icon" />管理项目…</span></DropdownMenu.Item>}
          </DropdownMenu.Content></DropdownMenu.Portal>
        </DropdownMenu.Root>
      </div>
      {renamingProjectId === project.id && <input className="session-rename-input project-rename-input" aria-label={`重命名项目 ${project.name}`} autoFocus value={projectDraft}
        onChange={(event) => setProjectDraft(event.target.value)}
        onBlur={() => { const name = projectDraft.trim(); const numeric = Number(project.id); if (name && Number.isFinite(numeric)) onRenameProject?.(numeric, name); setRenamingProjectId(null); }}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing) return;
          if (event.key === 'Enter') { const name = projectDraft.trim(); const numeric = Number(project.id); if (name && Number.isFinite(numeric)) onRenameProject?.(numeric, name); setRenamingProjectId(null); }
          if (event.key === 'Escape') setRenamingProjectId(null);
        }} />}
      {expanded && <div className="project-children">
        {groups.length > 0
          ? groups.map((group) => <div key={group.key} className="session-group">
            {group.label && <p className="session-group-label">{group.label}</p>}
            {group.sessions.map((session) => sessionRow(session, false))}
          </div>)
          : <p className="project-empty">{query ? '没有匹配' : '暂无任务'}</p>}
        {!query && (project.archivedSessions?.length ?? 0) > 0 && <>
          <button type="button" className="archived-toggle" data-state={archivedOpen ? 'open' : 'closed'} aria-label="已归档会话"
            onClick={() => setArchivedOpen((open) => !open)}>
            <ChevronRight size={11} /><Archive size={11} />已归档 ({project.archivedSessions!.length})
          </button>
          {archivedOpen && project.archivedSessions!.map((session) => sessionRow(session, true))}
        </>}
      </div>}
    </div>;
  }

  const filtered = query ? projects.flatMap((project) => project.sessions.filter((session) => session.title.toLowerCase().includes(query))) : [];

  return <div className="project-sessions">
    <div className="session-section-heading"><span>项目</span><IconButton label="新建项目" disabled={!canCreate} onClick={() => onNewProject?.()}><Plus size={16} /></IconButton></div>
    <label className="session-search"><Search size={13} /><input aria-label="搜索会话" value={search} onChange={(event) => setSearch(event.target.value)} placeholder="Search projects & agents…" /></label>
    <nav className="project-tree" aria-label="选择会话">
      {query
        ? (filtered.length ? filtered.map((session) => sessionRow(session, false)) : <p className="sidebar-empty">没有匹配的会话</p>)
        : [...projects].sort((a, b) => (pinnedProjectIds.includes(b.id) ? 1 : 0) - (pinnedProjectIds.includes(a.id) ? 1 : 0)).map(projectNode)}
      {!query && projects.length === 0 && <p className="sidebar-empty">暂无项目</p>}
    </nav>
  </div>;
}
