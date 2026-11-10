/// Session/project/plan CRUD API — thin pass-throughs over Tauri `invoke`.
import { invoke } from '@tauri-apps/api/core';
import type { PlanCard, PlanDecision, ProjectView, SessionDetail, SessionView } from './types';

export const sessionsApi = {
  sessions: () => invoke<SessionView[]>('list_sessions'),
  projects: () => invoke<ProjectView[]>('list_projects'),
  createProject: (name: string, path?: string) =>
    invoke<ProjectView>('create_project', { name, path: path ?? null }),
  setProjectArchived: (projectId: number, archived: boolean) =>
    invoke<ProjectView>('set_project_archived', { projectId, archived }),
  updateProjectName: (projectId: number, name: string) => invoke<void>('update_project_name', { projectId, name }),
  updateProjectPath: (projectId: number, path: string) => invoke<void>('update_project_path', { projectId, path }),
  deleteProject: (projectId: number) => invoke<void>('delete_project', { projectId }),
  createSession: (projectId?: number) =>
    invoke<SessionView>('create_session', { projectId: projectId ?? null }),
  setSessionArchived: (sessionId: number, archived: boolean) =>
    invoke<SessionView>('set_session_archived', { sessionId, archived }),
  renameSession: (sessionId: number, title: string) =>
    invoke<SessionView>('rename_session', { sessionId, title }),
  deleteSession: (sessionId: number) => invoke<void>('delete_session', { sessionId }),
  session: (sessionId: number) => invoke<SessionDetail>('get_session', { sessionId }),
  decidePlan: (sessionId: number, planId: number, decision: PlanDecision) =>
    invoke<PlanCard>('decide_plan', { sessionId, planId, decision }),
};
