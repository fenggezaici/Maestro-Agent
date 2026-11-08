import { useEffect, useRef, useState } from 'react';
import type { editor } from 'monaco-editor';
import type { ExplorerFile } from './workbenchMock';

function language(path: string) { return /\.(h|cpp|hpp)$/.test(path) ? 'cpp' : /\.(ts|tsx|js)$/.test(path) ? 'typescript' : /\.md$/.test(path) ? 'markdown' : 'plaintext'; }

export default function MonacoCode({ file, fontSize, fontFamily }: { file: ExplorerFile; fontSize: number; fontFamily: string }) {
  const host = useRef<HTMLDivElement>(null);
  const instance = useRef<editor.IStandaloneCodeEditor | null>(null);
  const [ready, setReady] = useState(false);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let disposed = false;
    let model: editor.ITextModel | undefined;
    let editorInstance: editor.IStandaloneCodeEditor | undefined;
    setReady(false); setFailed(false);
    void import('../editor/monaco').then(({ monaco }) => {
      if (disposed || !host.current) return;
      model = monaco.editor.createModel(file.content, language(file.path));
      editorInstance = monaco.editor.create(host.current, {
        model, theme: 'maestro-dark', readOnly: true, automaticLayout: true, minimap: { enabled: false },
        fontSize, fontFamily: `"${fontFamily}", Consolas, monospace`, lineHeight: 23, folding: true,
        scrollBeyondLastLine: false, links: false, contextmenu: false, wordWrap: 'off',
        padding: { top: 18, bottom: 20 }, ariaLabel: `只读文件 ${file.path}`, accessibilitySupport: 'on',
      });
      instance.current = editorInstance;
      setReady(true);
    }).catch(() => { if (!disposed) setFailed(true); });
    return () => { disposed = true; instance.current = null; editorInstance?.dispose(); model?.dispose(); };
  }, [file.id, file.path, file.content]);
  useEffect(() => { instance.current?.updateOptions({ fontSize, fontFamily: `"${fontFamily}", Consolas, monospace` }); }, [fontSize, fontFamily, ready]);
  return <section className="monaco-container" aria-label={`文件预览 ${file.path}`} data-code-ready={ready}>
    <div ref={host} className="monaco-host" />
    {!ready && !failed && <div className="editor-loading">Loading readonly preview…</div>}
    {failed && <pre className="editor-fallback">{file.content}</pre>}
  </section>;
}
