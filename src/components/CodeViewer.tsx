import { useEffect, useState } from 'react';

import type { ExplorerFile } from './workbenchMock';
import { loadViewer } from '../viewer/loader';

type ViewerModule = typeof import('../viewer/viewer');

/// Read-only file preview backed by @pierre/diffs (ZCode parity): virtualized
/// mount renders text immediately; shiki-wasm highlighting runs in a worker
/// pool off the main thread. The module stays behind one dynamic import so
/// the preview code never touches app startup.
export default function CodeViewer({ file, fontSize, fontFamily }: { file: ExplorerFile; fontSize: number; fontFamily: string }) {
  const [module, setModule] = useState<ViewerModule | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let disposed = false;
    setFailed(false);
    void loadViewer()
      .then((loaded) => { if (!disposed) setModule(loaded); })
      .catch(() => { if (!disposed) setFailed(true); });
    return () => { disposed = true; };
  }, []);

  if (failed) {
    return <section className="viewer-container" aria-label={`文件预览 ${file.path}`}><pre className="editor-fallback">{file.content}</pre></section>;
  }
  if (!module) {
    return (
      <section className="viewer-container" aria-label={`文件预览 ${file.path}`}>
        <div className="editor-loading">Loading readonly preview…</div>
      </section>
    );
  }
  return (
    <section className="viewer-container" aria-label={`文件预览 ${file.path}`}>
      <module.ViewerPool>
        <module.ViewerBody file={file} fontSize={fontSize} fontFamily={fontFamily} />
      </module.ViewerPool>
    </section>
  );
}
