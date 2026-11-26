import { invoke } from '@tauri-apps/api/core';

export interface GitSummary {
  /** false 时其余字段无意义——UI 显示"非 git 仓库"占位。 */
  repo: boolean;
  gitMissing: boolean;
  root: string | null;
  branch: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  staged: number;
  unstaged: number;
  untracked: number;
  conflicted: number;
  userName: string | null;
  userEmail: string | null;
}

export interface GitFileEntry {
  path: string;
  /** porcelain XY 状态码（如 "M."、".M"、"A."）；untracked 为 "?"。 */
  xy: string;
  origPath: string | null;
  added: number | null;
  removed: number | null;
}

export interface GitStatus {
  staged: GitFileEntry[];
  unstaged: GitFileEntry[];
  untracked: GitFileEntry[];
  conflicted: GitFileEntry[];
}

export interface GitCommit {
  hash: string;
  parents: string[];
  author: string;
  email: string;
  timestamp: number;
  subject: string;
  refs: string[];
}

export interface GitDiff {
  diff: string;
  truncated: boolean;
  binary: boolean;
}

/** source：'unstaged' | 'staged' | 'commit:<sha>' | 'branch:<base>'。 */
export type GitDiffSource = string;

export const gitApi = {
  summary: () => invoke<GitSummary>('git_summary'),
  /** 非 git 仓库时返回 null。 */
  status: () => invoke<GitStatus | null>('git_status'),
  diff: (path: string, source: GitDiffSource) =>
    invoke<GitDiff | null>('git_diff', { path, source }),
  /** branch:<base> / commit:<sha> 的逐文件清单；unstaged/staged 走 status。 */
  filesOf: (source: GitDiffSource) =>
    invoke<GitFileEntry[] | null>('git_files_of', { source }),
  log: (limit?: number, skip?: number) =>
    invoke<GitCommit[] | null>('git_log', { limit, skip }),
};
