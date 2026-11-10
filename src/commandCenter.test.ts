import { beforeEach, describe, expect, it } from 'vitest';
import { clearPaletteHistory, flattenFileTree, fuzzyScore, pushPaletteHistory, readPaletteHistory } from './commandCenter';
import type { FileNode } from './api/workspace';

beforeEach(() => localStorage.clear());

describe('fuzzyScore', () => {
  it('matches subsequences and misses non-subsequences', () => {
    expect(fuzzyScore('st', 'Open Settings')).not.toBeNull();
    expect(fuzzyScore('ose', 'Open Settings')).not.toBeNull();
    expect(fuzzyScore('xyz', 'Open Settings')).toBeNull();
    expect(fuzzyScore('', 'anything')).toBe(0);
  });

  it('prefers tighter earlier matches', () => {
    const tight = fuzzyScore('set', 'Settings');
    const loose = fuzzyScore('set', 'some long title with s e t far apart');
    expect(tight!).toBeGreaterThan(loose!);
  });
});

describe('flattenFileTree', () => {
  it('flattens nested directories into file paths', () => {
    const tree: FileNode[] = [
      { name: 'src', path: 'src', is_dir: true, children: [
        { name: 'a.ts', path: 'src/a.ts', is_dir: false, children: [] },
        { name: 'lib', path: 'src/lib', is_dir: true, children: [
          { name: 'b.ts', path: 'src/lib/b.ts', is_dir: false, children: [] },
        ] },
      ] },
      { name: 'README.md', path: 'README.md', is_dir: false, children: [] },
    ];
    expect(flattenFileTree(tree)).toEqual([
      { path: 'src/a.ts', name: 'a.ts' },
      { path: 'src/lib/b.ts', name: 'b.ts' },
      { path: 'README.md', name: 'README.md' },
    ]);
  });

  it('respects the limit', () => {
    const tree: FileNode[] = Array.from({ length: 5 }, (_, index) => ({ name: `f${index}`, path: `f${index}`, is_dir: false, children: [] }));
    expect(flattenFileTree(tree, 3)).toHaveLength(3);
  });
});

describe('palette history', () => {
  it('pushes most recent first, dedupes, and caps the list', () => {
    for (let index = 0; index < 12; index++) pushPaletteHistory('live', { label: `q${index}` });
    pushPaletteHistory('live', { label: 'q5' });
    const history = readPaletteHistory('live');
    expect(history[0].label).toBe('q5');
    expect(history.filter((entry) => entry.label === 'q5')).toHaveLength(1);
    expect(history.length).toBeLessThanOrEqual(8);
  });

  it('isolates history per source and clears it', () => {
    pushPaletteHistory('live', { label: 'a' });
    pushPaletteHistory('preview', { label: 'b' });
    expect(readPaletteHistory('live')[0].label).toBe('a');
    clearPaletteHistory('live');
    expect(readPaletteHistory('live')).toEqual([]);
    expect(readPaletteHistory('preview')).toHaveLength(1);
  });
});
