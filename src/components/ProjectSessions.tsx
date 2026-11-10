import { useState } from 'react';
import * as DropdownMenu from '@radix-ui/react-dropdown-menu';
import { Archive, ArchiveRestore, ChevronRight, MoreHorizontal, Pencil, Pin, PinOff, Plus, Search } from 'lucide-react';
import { IconButton } from './ui';
import { groupSessionsByDate } from '../sessionGroups';
import { relativeSessionAge, type WorkbenchProject } from './workspace';
import type { SessionView } from '../ipc';

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
}

function SessionMenu({ session, pinned, onRename, onArchive, onTogglePin }: {
  session: SessionView;
  pinned: boolean;
  onRename?: (session: SessionView) => void;
  onArchive?: (sessionId: number, archived: boolean) => void;
  onTogglePin?: (sessionId: number, pinned: boolean) => void;
}) {
  if (!onRename && !onArchive && !onTogglePin) return null;
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
    </DropdownMenu.Content></DropdownMenu.Portal>
  </DropdownMenu.Root>;
}

export default function ProjectSessions({ projects, currentProjectId, selectedSessionId, activeSessionId, source, canCreate, pinnedIds = [], onSelect, onCreate, onTogglePin, onRenameSession, onArchiveSession }: Props) {
  const [search, setSearch] = useState('');
  const [renamingId, setRenamingId] = useState<number | null>(null);
  const [renameDraft, setRenameDraft] = useState('');
  const [archivedOpen, setArchivedOpen] = useState(false);
  const query = search.trim().toLowerCase();
  const current = projects.find((project) => project.id === currentProjectId) ?? projects[0];
  const visibleSessions = current
    ? query ? current.sessions.filter((session) => session.title.toLowerCase().includes(query)) : current.sessions
    : [];
  const groups = query ? [] : groupSessionsByDate(visibleSessions, pinnedIds, Date.now(), source === 'preview');
  const archivedSessions = current?.archivedSessions ?? [];

  function create() {
    if (!canCreate || !current) return;
    onCreate(current.id);
  }

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
      <SessionMenu session={session} pinned={pinned} onTogglePin={archived ? undefined : onTogglePin} onRename={onRenameSession ? startRename : undefined} onArchive={onArchiveSession} />
    </div>;
  }

  return <div className="project-sessions">
    <div className="session-section-heading"><span>WORKSPACES</span><IconButton label="新建会话" disabled={!canCreate} onClick={create}><Plus size={16} /></IconButton></div>
    <label className="session-search"><Search size={13} /><input aria-label="搜索会话" value={search} onChange={(event) => setSearch(event.target.value)} placeholder="Search projects & agents…" /></label>
    <nav className="project-tree" aria-label="选择会话">
      {groups.length > 0
        ? groups.map((group) => <div key={group.key} className="session-group">
          {group.label && <p className="session-group-label">{group.label}</p>}
          {group.sessions.map((session) => sessionRow(session, false))}
        </div>)
        : visibleSessions.map((session) => sessionRow(session, false))}
      {visibleSessions.length === 0 && <p className="sidebar-empty">{query ? '没有匹配的会话' : '此项目暂无会话'}</p>}
      {!query && archivedSessions.length > 0 && <>
        <button type="button" className="archived-toggle" data-state={archivedOpen ? 'open' : 'closed'} aria-label="已归档会话"
          onClick={() => setArchivedOpen((open) => !open)}>
          <ChevronRight size={11} /><Archive size={11} />已归档 ({archivedSessions.length})
        </button>
        {archivedOpen && archivedSessions.map((session) => sessionRow(session, true))}
      </>}
    </nav>
  </div>;
}
