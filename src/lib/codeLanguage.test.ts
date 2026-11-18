import { describe, expect, it } from 'vitest';

import { inferCodeLanguage } from './codeLanguage';

describe('inferCodeLanguage', () => {
  it('maps extensions like ZCode (tsx → tsx, txt → log, svg → xml)', () => {
    expect(inferCodeLanguage('src/App.tsx')).toBe('tsx');
    expect(inferCodeLanguage('notes/readme.txt')).toBe('log');
    expect(inferCodeLanguage('assets/icon.svg')).toBe('xml');
    expect(inferCodeLanguage('src\\win\\path.py')).toBe('python');
    expect(inferCodeLanguage('config/settings.yml')).toBe('yaml');
    expect(inferCodeLanguage('Makefile.am')).toBe('log');
  });

  it('falls back through shebang and diff-shape hints', () => {
    expect(inferCodeLanguage('run', '#!/usr/bin/env python3\nprint(1)')).toBe('python');
    expect(inferCodeLanguage('run', '#!/bin/bash\nset -e')).toBe('bash');
    expect(inferCodeLanguage('output', 'diff --git a/x b/x\n@@ -1 +1 @@')).toBe('diff');
    expect(inferCodeLanguage('plain', 'just words')).toBe('log');
    expect(inferCodeLanguage()).toBe('log');
  });
});
