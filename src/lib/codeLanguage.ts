/// Language inference for the code viewer, aligned with ZCode
/// (`packages/ui/src/lib/codeViewer.ts`): extension map first, then shebang
/// and diff-shape hints, with `log` as the plain-text fallback.
import type { BundledLanguage } from 'shiki';

const EXTENSION_TO_LANGUAGE: Record<string, BundledLanguage> = {
  bash: 'bash',
  c: 'c',
  cc: 'cpp',
  cpp: 'cpp',
  cs: 'csharp',
  css: 'css',
  diff: 'diff',
  go: 'go',
  h: 'c',
  hpp: 'cpp',
  htm: 'html',
  html: 'html',
  java: 'java',
  js: 'javascript',
  json: 'json',
  jsx: 'jsx',
  kt: 'kotlin',
  md: 'markdown',
  mermaid: 'mermaid',
  mjs: 'javascript',
  php: 'php',
  py: 'python',
  rb: 'ruby',
  rs: 'rust',
  scss: 'scss',
  sh: 'bash',
  sql: 'sql',
  svg: 'xml',
  swift: 'swift',
  text: 'log',
  toml: 'toml',
  ts: 'typescript',
  tsx: 'tsx',
  txt: 'log',
  xml: 'xml',
  yaml: 'yaml',
  yml: 'yaml',
};

function getPathLeaf(path: string): string {
  const normalized = path.replace(/\\/g, '/');
  const slash = normalized.lastIndexOf('/');
  return slash >= 0 ? normalized.slice(slash + 1) : normalized;
}

function looksLikeDiff(text: string): boolean {
  return /^(diff --git|--- |\+\+\+ |@@ )/m.test(text);
}

export function inferCodeLanguage(path?: string, contentHint?: string): BundledLanguage {
  if (path) {
    const leaf = getPathLeaf(path);
    const extension = leaf.includes('.') ? leaf.split('.').pop()?.toLowerCase() : undefined;
    if (extension && EXTENSION_TO_LANGUAGE[extension]) {
      return EXTENSION_TO_LANGUAGE[extension];
    }
  }
  if (contentHint?.startsWith('#!/')) {
    if (contentHint.includes('python')) return 'python';
    if (contentHint.includes('bash') || contentHint.includes('sh')) return 'bash';
  }
  if (contentHint && looksLikeDiff(contentHint)) {
    return 'diff';
  }
  return 'log';
}
