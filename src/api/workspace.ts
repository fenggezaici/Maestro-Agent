import { invoke } from '@tauri-apps/api/core';

export interface FileNode { name: string; path: string; is_dir: boolean; children: FileNode[] }
export interface WorkspaceInfo { name: string; path: string | null }

export const workspaceApi = {
  info: () => invoke<WorkspaceInfo>('get_workspace_info'),
  select: (workspacePath: string) => invoke<WorkspaceInfo>('select_workspace', { workspacePath }),
  tree: (workspacePath: string) => invoke<FileNode[]>('read_directory_tree', { workspacePath }),
  text: (workspacePath: string, path: string) => invoke<string>('read_workspace_text', { workspacePath, path }),
};
