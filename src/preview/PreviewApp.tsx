import { useRef, useState } from 'react';
import Workbench from '../components/Workbench';
import { previewDetail, previewProjects, previewSnapshot } from './fixtures';

// Explicit browser-only visual preview; deliberately no useKernel/invoke/listen imports.
export default function PreviewApp() {
  const [selectedSessionId, setSelectedSessionId] = useState<number | null>(101);
  const [projects, setProjects] = useState(() => structuredClone(previewProjects));
  const nextId = useRef(106);
  const detail = previewDetail(selectedSessionId, projects);
  const presentationProjects = projects.map((project) => ({ ...project, sessions: project.sessions.map((item) => item.session) }));
  return <Workbench source="preview" projects={presentationProjects} onCreateProjectSession={(projectId) => {
    if (!projects.some((project) => project.id === projectId)) return;
    const id = nextId.current++;
    setProjects((items) => items.map((project) => project.id !== projectId ? project : { ...project, sessions: [...project.sessions,
      { session: { id, title: '新会话', updatedAt: 1, runCount: 0 }, runs: [], plans: [] },
    ] }));
    setSelectedSessionId(id);
  }} controller={{
    state: previewSnapshot, sessions: projects.flatMap((project) => project.sessions.map((item) => item.session)), selectedSessionId, detail, runs: detail?.runs ?? [],
    loadingSession: false, pending: false, error: null,
    selectSession: setSelectedSessionId,
    createSession: () => undefined, start: async () => undefined, cancel: () => undefined,
    setMode: () => undefined, decidePlan: () => undefined,
  }} />;
}
