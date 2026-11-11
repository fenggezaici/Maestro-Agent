import { useMemo } from 'react';

export interface DiffRow { type: 'same' | 'add' | 'del' | 'fold'; text: string; oldNo: number | null; newNo: number | null; fold?: number }

/// Line-level LCS diff — inputs are already bounded (≤8KB each) by the kernel.
export function diffLines(before: string, after: string): DiffRow[] {
  const a = before === '' ? [] : before.split('\n');
  const b = after === '' ? [] : after.split('\n');
  const n = a.length, m = b.length;
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const rows: DiffRow[] = [];
  let i = 0, j = 0, oldNo = 1, newNo = 1;
  while (i < n && j < m) {
    if (a[i] === b[j]) { rows.push({ type: 'same', text: a[i], oldNo: oldNo++, newNo: newNo++ }); i++; j++; }
    else if (dp[i + 1][j] >= dp[i][j + 1]) { rows.push({ type: 'del', text: a[i], oldNo: oldNo++, newNo: null }); i++; }
    else { rows.push({ type: 'add', text: b[j], oldNo: null, newNo: newNo++ }); j++; }
  }
  while (i < n) { rows.push({ type: 'del', text: a[i], oldNo: oldNo++, newNo: null }); i++; }
  while (j < m) { rows.push({ type: 'add', text: b[j], oldNo: null, newNo: newNo++ }); j++; }
  return rows;
}

export function countChanges(before: string, after: string): { added: number; removed: number } {
  let added = 0, removed = 0;
  for (const row of diffLines(before, after)) {
    if (row.type === 'add') added++;
    else if (row.type === 'del') removed++;
  }
  return { added, removed };
}

/// Keep context tight: collapse runs of >7 unchanged rows to first 3 + last 3.
function collapse(rows: DiffRow[]): DiffRow[] {
  const out: DiffRow[] = [];
  let i = 0;
  while (i < rows.length) {
    if (rows[i].type !== 'same') { out.push(rows[i++]); continue; }
    let end = i;
    while (end < rows.length && rows[end].type === 'same') end++;
    const run = end - i;
    if (run <= 7) { out.push(...rows.slice(i, end)); }
    else {
      out.push(...rows.slice(i, i + 3), { type: 'fold', text: '', oldNo: null, newNo: null, fold: run - 6 }, ...rows.slice(end - 3, end));
    }
    i = end;
  }
  return out;
}

/// Compact inline diff: line numbers + red/green rows, ZCode-style.
export default function MiniDiff({ before, after }: { before: string; after: string }) {
  const rows = useMemo(() => collapse(diffLines(before, after)), [before, after]);
  return <div className="mini-diff" role="table" aria-label="代码差异">
    {rows.map((row, index) => row.type === 'fold'
      ? <div key={index} className="mini-diff-fold">··· {row.fold} 行未变更 ···</div>
      : <div key={index} className={`mini-diff-row mini-diff-${row.type}`}>
          <span className="mini-diff-no">{row.oldNo ?? ''}</span>
          <span className="mini-diff-no">{row.newNo ?? ''}</span>
          <code className="mini-diff-code">{row.text === '' ? ' ' : row.text}</code>
        </div>)}
  </div>;
}
