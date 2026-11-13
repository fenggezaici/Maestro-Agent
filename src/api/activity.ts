import { invoke } from '@tauri-apps/api/core';
import type { DashboardData } from '../components/activityMock';

export const activitySnapshotApi = (projectId?: number) =>
  invoke<DashboardData>('activity_snapshot', { project_id: projectId ?? null });
