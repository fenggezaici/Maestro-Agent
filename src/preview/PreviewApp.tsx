import { useRef, useState } from 'react';
import Workbench from '../components/Workbench';
import { usePreviewExtensions } from '../components/workbenchMock';
import { previewDetail, previewProjects, previewSnapshot } from './fixtures';

// Explicit browser-only visual preview; deliberately no useKernel/invoke/listen imports.
export default function PreviewApp() {
  const [selectedSessionId, setSelectedSessionId] = useState<number | null>(101);
  const [state, setState] = useState(() => ({ ...previewSnapshot }));
  const [projects, setProjects] = useState(() => structuredClone(previewProjects));
  const nextId = useRef(106);
  const nextProjectId = useRef(4);
  const detail = previewDetail(selectedSessionId, projects);
  const presentationProjects = projects.filter((project) => !project.archived).map((project) => ({ ...project,
    sessions: project.sessions.map((item) => item.session).filter((item) => !item.archived),
    archivedSessions: project.sessions.map((item) => item.session).filter((item) => item.archived),
  }));
  const mutateSession = (sessionId: number, update: (session: { id: number; title: string; updatedAt: number; runCount: number; projectId: number; archived: boolean }) => { id: number; title: string; updatedAt: number; runCount: number; projectId: number; archived: boolean }) => {
    setProjects((items) => items.map((project) => ({ ...project,
      sessions: project.sessions.map((item) => item.session.id === sessionId ? { ...item, session: update(item.session) } : item),
    })));
  };
  const extensions = usePreviewExtensions();
  return <Workbench source="preview" projects={presentationProjects} extensions={extensions} onCreateProjectSession={(projectId) => {
    if (!projects.some((project) => project.id === projectId)) return;
    const id = nextId.current++;
    const numeric = Number(projectId);
    setProjects((items) => items.map((project) => project.id !== projectId ? project : { ...project, sessions: [...project.sessions,
      { session: { id, title: '新会话', updatedAt: 1, runCount: 0, projectId: Number.isFinite(numeric) ? numeric : 1, archived: false }, runs: [], plans: [], metrics: [] },
    ] }));
    setSelectedSessionId(id);
  }} sessionActions={{
    rename: (sessionId, title) => mutateSession(sessionId, (session) => ({ ...session, title })),
    archive: (sessionId, archived) => mutateSession(sessionId, (session) => ({ ...session, archived })),
    archiveProject: (projectId, archived) => setProjects((items) => items.map((project) => project.id === projectId ? { ...project, archived } : project)),
    deleteProject: (projectId) => setProjects((items) => items.filter((project) => project.id !== projectId)),
    createProject: async (name, path) => {
      const id = `p${nextProjectId.current++}`;
      setProjects((items) => [...items, { id, name, path: path ?? null, sessions: [] }]);
      return null;
    },
  }} controller={{
    state, sessions: projects.flatMap((project) => project.sessions.map((item) => item.session)), selectedSessionId, detail, runs: detail?.runs ?? [],
    loadingSession: false, pending: false, error: null,
    selectSession: setSelectedSessionId,
    createSession: () => undefined, start: async () => undefined, cancel: () => undefined,
    setMode: (mode) => setState((current) => ({ ...current, mode })), decidePlan: () => undefined,
  }} />;
}
