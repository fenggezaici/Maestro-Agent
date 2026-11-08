import { beforeEach, expect, it } from 'vitest';
import { bindShortcut, defaultUiPreferences, readUiPreferences, saveUiPreferences, shortcutChord } from './uiPreferences';
import { fileTree } from './components/ExplorerPanel';
import { changeMockExtension, mockExtensions } from './components/workbenchMock';

beforeEach(() => localStorage.clear());

it('isolates UI preferences and validates fonts, bounds and shortcut conflicts', () => {
  const changed = bindShortcut(structuredClone(defaultUiPreferences), 'activity', 'Mod+Shift+KeyB');
  saveUiPreferences('preview', { ...changed, editorFontSize: 16 });
  expect(readUiPreferences('preview').editorFontSize).toBe(16);
  expect(readUiPreferences('live').editorFontSize).toBe(12);
  expect(() => bindShortcut(changed, 'activity', 'Mod+Enter')).toThrow();
  expect(() => bindShortcut(changed, 'activity', changed.shortcuts.settings)).toThrow();
  expect(shortcutChord({ code: 'KeyB', ctrlKey: true, metaKey: false, altKey: false, shiftKey: true })).toBe('Mod+Shift+KeyB');
  localStorage.setItem('maestro.ui.v1.preview', JSON.stringify({ editorFontSize: 99, editorFont: 'remote font', temperature: -1 }));
  expect(readUiPreferences('preview')).toMatchObject({ editorFontSize: 12, editorFont: 'Cascadia Code', temperature: 0.7 });
});

it('keeps virtual file identities scoped and never conflates duplicate filenames', () => {
  const tree = fileTree([
    { id: 'first', path: 'src/one/File.ts', content: 'one', source: 'mock' },
    { id: 'second', path: 'src/two/File.ts', content: 'two', source: 'mock' },
  ], 'project-a');
  expect(tree[0].children).toHaveLength(2);
  expect(tree[0].children[0].children[0].file?.content).toBe('one');
  expect(tree[0].children[1].children[0].file?.content).toBe('two');
  expect(tree[0].id).toBe('project-a:src');
});

it('only simulates allowed extension transitions and keeps unavailable entries disabled', () => {
  const unavailable = changeMockExtension(mockExtensions, 'browser', 'toggle');
  expect(unavailable.find((item) => item.id === 'browser')?.enabled).toBe(false);
  const installed = changeMockExtension(mockExtensions, 'tests', 'install');
  expect(installed.find((item) => item.id === 'tests')).toMatchObject({ status: 'installed', enabled: false });
  const enabled = changeMockExtension(installed, 'tests', 'toggle');
  expect(enabled.find((item) => item.id === 'tests')?.enabled).toBe(true);
  expect(changeMockExtension(enabled, 'tests', 'remove').find((item) => item.id === 'tests')).toMatchObject({ status: 'available', enabled: false });
});
