import { useMemo } from 'react';

interface DiffRow {
  kind: 'meta' | 'hunk' | 'added' | 'removed' | 'context';
  content: string;
}

/// 统一 diff 文本的行分类：文件头/hunk 头/增删/上下文。
/// 视觉语言沿用 LightweightDiffPreview（14% tint + 3px inset bar），但输入是
/// unified diff 文本而非 before/after 全文——git 给出的就是这种格式。
function classify(text: string): DiffRow[] {
  return text.split('\n').map((line) => {
    if (line.startsWith('diff --git') || line.startsWith('index ') || line.startsWith('---') || line.startsWith('+++')
      || line.startsWith('new file') || line.startsWith('deleted file') || line.startsWith('old mode') || line.startsWith('new mode')
      || line.startsWith('similarity index') || line.startsWith('rename from') || line.startsWith('rename to') || line.startsWith('Binary')) {
      return { kind: 'meta', content: line };
    }
    if (line.startsWith('@@')) return { kind: 'hunk', content: line };
    if (line.startsWith('+')) return { kind: 'added', content: line.slice(1) };
    if (line.startsWith('-')) return { kind: 'removed', content: line.slice(1) };
    return { kind: 'context', content: line.startsWith(' ') ? line.slice(1) : line };
  });
}

export default function GitDiffView({ diff, fontSize = 12 }: { diff: string; fontSize?: number }) {
  const rows = useMemo(() => classify(diff), [diff]);
  return <section className="lightweight-diff" aria-label="git diff" data-git-diff-view>
    <div className="lightweight-diff-scroll" style={{ fontSize }}>
      {rows.map((row, index) => {
        const added = row.kind === 'added';
        const removed = row.kind === 'removed';
        const color = added ? 'var(--color-diff-added)' : removed ? 'var(--color-diff-removed)' : 'transparent';
        if (row.kind === 'meta') {
          return <div key={index} className="git-diff-meta"><code>{row.content}</code></div>;
        }
        if (row.kind === 'hunk') {
          return <div key={index} className="git-diff-hunk"><code>{row.content}</code></div>;
        }
        return <div key={index} className={`lightweight-diff-row ${row.kind}`}
          style={added || removed ? {
            backgroundColor: `color-mix(in srgb, ${color} 14%, transparent)`,
            boxShadow: `inset 3px 0 0 ${color}`,
          } : undefined}>
          <code className="lightweight-diff-code">{row.content || ' '}</code>
        </div>;
      })}
    </div>
  </section>;
}
