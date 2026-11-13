import { useEffect, useRef, useState } from 'react';
import type { editor } from 'monaco-editor';
import type { ExplorerFile } from './workbenchMock';
import { loadMonaco } from '../editor/loader';

function language(path: string) {
  if (/\.(h|cpp|hpp|cc|c)$/.test(path)) return 'cpp';
  if (/\.(ts|tsx|js|jsx)$/.test(path)) return 'typescript';
  if (/\.py$/.test(path)) return 'python';
  if (/\.md$/.test(path)) return 'markdown';
  return 'plaintext';
}

/// One editor instance per preview pane; file changes only swap models.
export default function MonacoCode({ file, fontSize, fontFamily }: { file: ExplorerFile; fontSize: number; fontFamily: string }) {
  const host = useRef<HTMLDivElement>(null);
  const instance = useRef<editor.IStandaloneCodeEditor | null>(null);
  const model = useRef<editor.ITextModel | null>(null);
  const [ready, setReady] = useState(false);
  const [failed, setFailed] = useState(false);

  useEffect(() => () => {
    instance.current?.dispose();
    instance.current = null;
    model.current?.dispose();
    model.current = null;
  }, []);

  useEffect(() => {
    const element = host.current;
    if (!element) return;
    let raf: number | null = null;
    const layout = () => {
      if (document.body.dataset.panelDragging === 'true') return;
      if (raf !== null) cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => { raf = null; instance.current?.layout(); });
    };
    const observer = new ResizeObserver(layout);
    observer.observe(element);
    const settle = () => { if (raf !== null) cancelAnimationFrame(raf); instance.current?.layout(); };
    window.addEventListener('maestro-panel-resize-end', settle);
    return () => { observer.disconnect(); if (raf !== null) cancelAnimationFrame(raf); window.removeEventListener('maestro-panel-resize-end', settle); };
  }, []);

  useEffect(() => {
    let disposed = false;
    setFailed(false);
    void loadMonaco().then(({ monaco }) => {
      if (disposed || !host.current) return;
      if (!instance.current) {
        instance.current = monaco.editor.create(host.current, {
          theme: document.documentElement.dataset.theme === 'light' ? 'maestro-light' : 'maestro-dark',
          readOnly: true, automaticLayout: false, minimap: { enabled: false },
          fontSize, fontFamily: `"${fontFamily}", Consolas, monospace`, lineHeight: 23, folding: true,
          scrollBeyondLastLine: false, links: false, contextmenu: false, wordWrap: 'off',
          padding: { top: 18, bottom: 20 }, accessibilitySupport: 'on',
        });
      }
      const next = monaco.editor.createModel(file.content, language(file.path));
      instance.current.setModel(next);
      instance.current.updateOptions({
        fontSize, fontFamily: `"${fontFamily}", Consolas, monospace`,
        ariaLabel: `只读文件 ${file.path}`,
        theme: document.documentElement.dataset.theme === 'light' ? 'maestro-light' : 'maestro-dark',
      });
      model.current?.dispose();
      model.current = next;
      setReady(true);
    }).catch(() => { if (!disposed) setFailed(true); });
    return () => { disposed = true; };
  }, [file.id, file.path, file.content, fontSize, fontFamily]);

  return <section className="monaco-container" aria-label={`文件预览 ${file.path}`} data-code-ready={ready}>
    <div ref={host} className="monaco-host" />
    {!ready && !failed && <div className="editor-loading">Loading readonly preview…</div>}
    {failed && <pre className="editor-fallback">{file.content}</pre>}
  </section>;
}
