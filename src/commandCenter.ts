import type { FileNode } from './api/workspace';

/// Subsequence match scoring for the command palette. Returns null when the
/// query is not a subsequence; higher scores mean tighter, earlier matches.
export function fuzzyScore(query: string, text: string): number | null {
  const needle = query.trim().toLowerCase();
  if (!needle) return 0;
  const haystack = text.toLowerCase();
  let score = 0;
  let position = 0;
  let streak = 0;
  for (const char of needle) {
    const found = haystack.indexOf(char, position);
    if (found === -1) return null;
    streak = found === position ? streak + 1 : 1;
    score += 2 + streak * 3 + (found === 0 || /[\s\-_/#.]/.test(haystack[found - 1] ?? '') ? 8 : 0) - Math.min(found, 40) * 0.1;
    position = found + 1;
  }
  return score - (haystack.length - needle.length) * 0.01;
}

export interface FlatFile {
  path: string;
  name: string;
}

export function flattenFileTree(nodes: FileNode[], limit = 2000): FlatFile[] {
  const files: FlatFile[] = [];
  const walk = (items: FileNode[]) => {
    for (const node of items) {
      if (files.length >= limit) return;
      if (node.is_dir) walk(node.children);
      else files.push({ path: node.path, name: node.name });
    }
  };
  walk(nodes);
  return files;
}

export interface PaletteHistoryEntry {
  label: string;
  detail?: string;
}

const HISTORY_LIMIT = 8;
const key = (source: 'live' | 'preview') => `maestro.command-center.v1.${source}`;

export function readPaletteHistory(source: 'live' | 'preview'): PaletteHistoryEntry[] {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(key(source)) ?? 'null');
    if (!Array.isArray(value)) return [];
    return value.filter((entry): entry is PaletteHistoryEntry =>
      !!entry && typeof entry === 'object' && typeof (entry as PaletteHistoryEntry).label === 'string').slice(0, HISTORY_LIMIT);
  } catch { return []; }
}

export function pushPaletteHistory(source: 'live' | 'preview', entry: PaletteHistoryEntry) {
  try {
    const next = [entry, ...readPaletteHistory(source).filter((item) => item.label !== entry.label)].slice(0, HISTORY_LIMIT);
    localStorage.setItem(key(source), JSON.stringify(next));
  } catch { /* History is best-effort UI state. */ }
}

export function clearPaletteHistory(source: 'live' | 'preview') {
  try { localStorage.removeItem(key(source)); } catch { /* Best effort. */ }
}
