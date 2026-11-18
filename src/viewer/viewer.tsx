/// Lazy chunk behind `CodeViewer`: the @pierre/diffs viewer + its worker pool.
/// Mirrors ZCode's `DiffsWorkerPoolProvider` + `code-viewer.tsx` essentials —
/// shiki-wasm highlighting in a worker pool so the webview main thread only
/// mounts already-tokenized rows.
import { useEffect, useMemo, useState, type CSSProperties, type ReactNode } from 'react';
import {
  File,
  WorkerPoolContextProvider,
  type FileContents,
  type FileOptions,
  type WorkerInitializationRenderOptions,
} from '@pierre/diffs/react';
import type { HighlighterTypes } from '@pierre/diffs';
import type { BundledTheme } from 'shiki';

import type { ExplorerFile } from '../components/workbenchMock';
import { inferCodeLanguage } from '../lib/codeLanguage';

/// ZCode `DIFFS_PREFERRED_HIGHLIGHTER`: the oniguruma WASM engine keeps
/// TextMate regexes out of the V8 code space (the shiki-js engine was the root
/// cause of renderer OOM under long sessions).
const PREFERRED_HIGHLIGHTER: HighlighterTypes = 'shiki-wasm';
const LIGHT_THEME: BundledTheme = 'github-light';
const DARK_THEME: BundledTheme = 'github-dark';

const HIGHLIGHTER_OPTIONS: WorkerInitializationRenderOptions = {
  theme: { light: LIGHT_THEME, dark: DARK_THEME },
  lineDiffType: 'word-alt',
  maxLineDiffLength: 1_000,
  tokenizeMaxLineLength: 1_000,
  useTokenTransformer: false,
  preferredHighlighter: PREFERRED_HIGHLIGHTER,
};

function createDiffsWorker(): Worker {
  return new Worker(new URL('../workers/diffs.worker.ts', import.meta.url), {
    type: 'module',
    name: 'maestro-diffs-worker',
  });
}

function resolveWorkerPoolSize(): number {
  if (typeof navigator === 'undefined') return 2;
  const cores = navigator.hardwareConcurrency;
  if (!Number.isFinite(cores) || cores <= 0) return 2;
  return Math.max(1, Math.min(4, Math.floor(cores / 2)));
}

/// Worker pool for highlight/render work. Without `Worker` (tests, SSR) the
/// children render on the main thread — the library falls back by itself.
export function ViewerPool({ children }: { children: ReactNode }) {
  const poolSize = useMemo(resolveWorkerPoolSize, []);
  if (typeof window === 'undefined' || typeof Worker === 'undefined') {
    return <>{children}</>;
  }
  return (
    <WorkerPoolContextProvider
      poolOptions={{ workerFactory: createDiffsWorker, poolSize }}
      highlighterOptions={HIGHLIGHTER_OPTIONS}
    >
      {children}
    </WorkerPoolContextProvider>
  );
}

function currentTheme(): BundledTheme {
  return document.documentElement.dataset.theme === 'light' ? LIGHT_THEME : DARK_THEME;
}

function cacheKey(file: ExplorerFile): string {
  const content = file.content;
  return `${file.path}:${content.length}:${content.slice(0, 100)}:${content.slice(-100)}`;
}

/// Read-only file surface: virtualized text mount, worker-side highlighting.
export function ViewerBody({ file, fontSize, fontFamily }: { file: ExplorerFile; fontSize: number; fontFamily: string }) {
  const [theme, setTheme] = useState<BundledTheme>(currentTheme);
  useEffect(() => {
    const sync = () => setTheme(currentTheme());
    window.addEventListener('maestro-theme', sync);
    return () => window.removeEventListener('maestro-theme', sync);
  }, []);

  const contents = useMemo<FileContents>(
    () => ({ name: file.path, contents: file.content, lang: inferCodeLanguage(file.path, file.content), cacheKey: cacheKey(file) }),
    [file],
  );
  const options = useMemo<FileOptions<never, never>>(
    () => ({
      disableFileHeader: true,
      overflow: 'scroll',
      theme,
      preferredHighlighter: PREFERRED_HIGHLIGHTER,
    }),
    [theme],
  );
  const style = useMemo<CSSProperties>(
    () => ({
      colorScheme: theme === DARK_THEME ? 'dark' : 'light',
      // @pierre/diffs renders inside Shadow DOM; these custom properties are
      // the supported way to carry app tokens through the shadow root.
      ['--diffs-bg' as never]: 'var(--color-background)',
      ['--diffs-light-bg' as never]: 'var(--color-background)',
      ['--diffs-dark-bg' as never]: 'var(--color-background)',
      ['--diffs-font-family' as never]: `"${fontFamily}", Consolas, monospace`,
      ['--diffs-font-size' as never]: `${fontSize}px`,
    }),
    [fontFamily, fontSize, theme],
  );

  return (
    <File
      key={contents.cacheKey}
      file={contents}
      options={options}
      className="viewer-file"
      style={style}
    />
  );
}
