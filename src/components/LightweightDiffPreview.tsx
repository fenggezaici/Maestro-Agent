import { useMemo } from 'react';
import { lineDiff } from '../lib/lineDiff';

/// ZCode lightweight diff: added/removed rows use a 14% tint plus a 3px
/// inset bar; the line-number gutter uses an 18% tint. Unified +/- markers
/// are intentionally not rendered as code content.
export default function LightweightDiffPreview({ before, after, fontSize = 12, path }: { before: string; after: string; fontSize?: number; path?: string }) {
  const lines = useMemo(() => lineDiff(before, after), [before, after]);
  return <section className="lightweight-diff" aria-label={`Diff ${path ?? ''}`} data-lightweight-diff-preview>
    <div className="lightweight-diff-scroll" style={{ fontSize }}>
      {lines.map((line, index) => {
        const added = line.kind === 'added';
        const removed = line.kind === 'removed';
        const color = added ? 'var(--color-diff-added)' : removed ? 'var(--color-diff-removed)' : 'transparent';
        return <div key={`${index}-${line.line}`} className={`lightweight-diff-row ${line.kind}`}
          style={added || removed ? {
            backgroundColor: `color-mix(in srgb, ${color} 14%, transparent)`,
            boxShadow: `inset 3px 0 0 ${color}`,
          } : undefined}>
          <span aria-hidden="true" className={`lightweight-diff-gutter ${line.kind}`}
            style={added || removed ? {
              backgroundColor: `color-mix(in srgb, ${color} 18%, var(--color-background))`,
              boxShadow: `inset 3px 0 0 ${color}`,
              color,
            } : undefined}>{line.line || index + 1}</span>
          <code className="lightweight-diff-code">{line.content || ' '}</code>
        </div>;
      })}
    </div>
  </section>;
}
