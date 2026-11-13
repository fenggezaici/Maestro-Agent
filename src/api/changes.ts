import { invoke } from '@tauri-apps/api/core';

export interface RunFileChange { path: string; added: number; removed: number; before: string; after: string; createdAt: number }
export interface RunChangeSummary { files: number; additions: number; deletions: number; state: 'applied' | 'reverted' }
export interface RevertIssue { path: string; reason: string }
export interface RevertPreview { safeFiles: string[]; unsafeFiles: RevertIssue[]; ignoredFiles: RevertIssue[] }
export interface RevertResult { reverted: string[]; conflicts: string[] }

export const runChangeSummaryApi = (runId: number) => invoke<RunChangeSummary>('run_change_summary', { run_id: runId });
export const runChangesApi = (runId: number) => invoke<RunFileChange[]>('run_changes', { run_id: runId });
export const previewRevertRunChangesApi = (runId: number) => invoke<RevertPreview>('preview_revert_run_changes', { run_id: runId });
export const revertRunChangesApi = (runId: number, sessionId: number) =>
  invoke<RevertResult>('revert_run_changes', { run_id: runId, session_id: sessionId });
