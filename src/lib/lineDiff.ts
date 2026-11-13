export type DiffLineKind = 'added' | 'removed' | 'context';
export interface DiffLine { kind: DiffLineKind; content: string; line: number }

/// Line-level LCS diff for the right-panel lightweight preview. Large files
/// fall back to a whole-file replacement so the UI never blocks on O(n*m).
export function lineDiff(before: string, after: string): DiffLine[] {
  const a = before.split('\n');
  const b = after.split('\n');
  if (before.length + after.length > 200_000 || a.length * b.length > 4_000_000) {
    return [...a.map((content) => ({ kind: 'removed' as const, content, line: 0 })), ...b.map((content) => ({ kind: 'added' as const, content, line: 0 }))];
  }
  const rows = a.length + 1;
  const cols = b.length + 1;
  const dp: Uint16Array[] = Array.from({ length: rows }, () => new Uint16Array(cols));
  for (let i = a.length - 1; i >= 0; i -= 1) {
    for (let j = b.length - 1; j >= 0; j -= 1) {
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) { out.push({ kind: 'context', content: a[i], line: j + 1 }); i += 1; j += 1; }
    else if (dp[i + 1][j] >= dp[i][j + 1]) { out.push({ kind: 'removed', content: a[i], line: i + 1 }); i += 1; }
    else { out.push({ kind: 'added', content: b[j], line: j + 1 }); j += 1; }
  }
  while (i < a.length) { out.push({ kind: 'removed', content: a[i], line: i + 1 }); i += 1; }
  while (j < b.length) { out.push({ kind: 'added', content: b[j], line: j + 1 }); j += 1; }
  return out;
}
