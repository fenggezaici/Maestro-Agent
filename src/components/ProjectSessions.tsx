import { useEffect, useRef, useState } from 'react';
import * as Accordion from '@radix-ui/react-accordion';
import { ChevronDown, Folder, Plus, Search } from 'lucide-react';
import { IconButton } from './ui';
import { readExpandedProjects, saveExpandedProjects, type WorkbenchProject } from './workspace';

interface Props {
  projects: WorkbenchProject[];
  currentProjectId: string;
  selectedSessionId: number | null;
  activeSessionId?: number;
  source: 'preview' | 'live';
  canCreate: boolean;
  onSelect: (id: number) => void;
  onCreate: (projectId: string) => void;
}

export default function ProjectSessions({ projects, currentProjectId, selectedSessionId, activeSessionId, source, canCreate, onSelect, onCreate }: Props) {
  const key = `maestro.projects.v1.${source}`;
  const [expanded, setExpanded] = useState(() => readExpandedProjects(key, projects));
  const [search, setSearch] = useState('');
  const previousSelection = useRef(selectedSessionId);
  const query = search.trim().toLowerCase();
  const filtered = projects.map((project) => ({ ...project,
    visibleSessions: project.name.toLowerCase().includes(query) ? project.sessions : project.sessions.filter((session) => session.title.toLowerCase().includes(query)),
  })).filter((project) => !query || project.visibleSessions.length > 0);
  const current = projects.find((project) => project.id === currentProjectId) ?? projects[0];

  function changeExpanded(ids: string[]) {
    setExpanded(ids);
    saveExpandedProjects(key, ids);
  }

  useEffect(() => {
    if (previousSelection.current === selectedSessionId) return;
    previousSelection.current = selectedSessionId;
    const projectId = projects.find((project) => project.sessions.some((session) => session.id === selectedSessionId))?.id;
    if (projectId) setExpanded((ids) => {
      const next = ids.includes(projectId) ? ids : [...ids, projectId];
      saveExpandedProjects(key, next);
      return next;
    });
  }, [selectedSessionId, projects, key]);

  function create() {
    if (!canCreate || !current) return;
    changeExpanded(expanded.includes(current.id) ? expanded : [...expanded, current.id]);
    onCreate(current.id);
  }

  return <div className="project-sessions">
    <div className="session-section-heading"><span>WORKSPACES</span><IconButton label="新建会话" disabled={!canCreate} onClick={create}><Plus size={16} /></IconButton></div>
    <label className="session-search"><Search size={13} /><input aria-label="搜索会话" value={search} onChange={(event) => setSearch(event.target.value)} placeholder="Search projects & agents…" /></label>
    <nav className="project-tree" aria-label="选择会话">
      <Accordion.Root type="multiple" value={query ? filtered.map((project) => project.id) : expanded}
        onValueChange={(ids) => { if (!query) changeExpanded(ids); }}>
        {filtered.map((project) => <Accordion.Item key={project.id} value={project.id} className="project-group">
          <Accordion.Header className="project-heading"><Accordion.Trigger className="project-trigger" aria-label={`项目 ${project.name}`}>
            <ChevronDown size={12} className="project-chevron" /><Folder size={14} strokeWidth={1.5} />
            <span className="project-name">{project.name}</span><span className="project-count">{project.sessions.length}</span>
          </Accordion.Trigger></Accordion.Header>
          <Accordion.Content className="project-content">
            {project.visibleSessions.map((session) => <button type="button" key={session.id} className="session-item" aria-pressed={session.id === selectedSessionId} onClick={() => onSelect(session.id)}>
              <span className={`session-indicator ${activeSessionId === session.id ? 'working' : ''}`} />
              <span className="session-copy"><span className="session-title">{session.title}</span><span className="session-subtitle">{session.runCount} 轮对话<span>#{session.id}</span></span></span>
            </button>)}
            {project.visibleSessions.length === 0 && <p className="sidebar-empty">此项目暂无会话</p>}
          </Accordion.Content>
        </Accordion.Item>)}
      </Accordion.Root>
      {filtered.length === 0 && <p className="sidebar-empty">没有匹配的项目或会话</p>}
    </nav>
  </div>;
}
