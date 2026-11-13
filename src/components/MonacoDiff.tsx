import { useEffect, useRef, useState } from 'react';
import type { ProposedDiff } from '../api/types';
import type { editor } from 'monaco-editor';
import { loadMonaco } from '../editor/loader';

function language(path: string) {
  if (/\.(h|hpp|cpp|cc|c)$/.test(path)) return 'cpp';
  if (/\.(ts|tsx|js|jsx)$/.test(path)) return 'typescript';
  if (/\.py$/.test(path)) return 'python';
  if (/\.md$/.test(path)) return 'markdown';
  return 'plaintext';
}

/// One diff editor per preview pane; diff changes only swap models.
export default function MonacoDiff({ diff, split, fontSize = 12, fontFamily = 'Cascadia Code' }: { diff: ProposedDiff; split: boolean; fontSize?: number; fontFamily?: string }) {
  const host = useRef<HTMLDivElement>(null);
  const instance = useRef<editor.IStandaloneDiffEditor | null>(null);
  const original = useRef<editor.ITextModel | null>(null);
  const modified = useRef<editor.ITextModel | null>(null);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState(false);

  useEffect(() => () => {
    instance.current?.dispose();
    instance.current = null;
    original.current?.dispose();
    modified.current?.dispose();
    original.current = null;
    modified.current = null;
  }, []);

  useEffect(() => {
    let disposed = false;
    setError(false);
    void loadMonaco().then(({ monaco }) => {
      if (disposed || !host.current) return;
      if (!instance.current) {
        const created = monaco.editor.createDiffEditor(host.current, {
          theme: document.documentElement.dataset.theme === 'light' ? 'maestro-light' : 'maestro-dark',
          readOnly: true, originalEditable: false, renderSideBySide: split, automaticLayout: true,
          minimap: { enabled: false }, fontFamily: `"${fontFamily}", Consolas, monospace`,
          fontSize, lineHeight: 23, padding: { top: 20, bottom: 24 },
          scrollBeyondLastLine: false, renderLineHighlight: 'none',
          overviewRulerLanes: 0, renderOverviewRuler: false, diffCodeLens: false,
          hideUnchangedRegions: { enabled: false }, wordWrap: 'off',
          enableSplitViewResizing: true, renderIndicators: true, folding: false, diffAlgorithm: 'advanced',
          links: false, contextmenu: false, stickyScroll: { enabled: false },
          accessibilitySupport: 'on', originalAriaLabel: `原始代码 ${diff.path}`,
          modifiedAriaLabel: `提议代码 ${diff.path}`,
        });
        created.onDidUpdateDiff(() => { if (!disposed) setReady(true); });
        instance.current = created;
      }
      const before = monaco.editor.createModel(diff.before, language(diff.path));
      const after = monaco.editor.createModel(diff.after, language(diff.path));
      instance.current.updateOptions({
        renderSideBySide: split, fontSize, fontFamily: `"${fontFamily}", Consolas, monospace`,
      });
      instance.current.setModel({ original: before, modified: after });
      original.current?.dispose();
      modified.current?.dispose();
      original.current = before;
      modified.current = after;
      setReady(true);
    }).catch(() => { if (!disposed) setError(true); });
    return () => { disposed = true; };
  }, [diff.path, diff.before, diff.after, split, fontSize, fontFamily]);

  return <section aria-label={`Diff ${diff.path}`} className="monaco-container" data-editor-ready={ready}>
    <div ref={host} className="monaco-host" />
    {!ready && !error && <div className="editor-loading">正在准备代码审查…</div>}
    {error && <div className="editor-fallback" role="status">
      <p>代码编辑器暂不可用，显示只读文本差异。</p>
      <div className="fallback-columns"><pre aria-label="原始代码">{diff.before || '// Empty file'}</pre><pre aria-label="提议代码">{diff.after}</pre></div>
    </div>}
  </section>;
}
