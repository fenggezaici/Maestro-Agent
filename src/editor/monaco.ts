import * as monaco from 'monaco-editor/editor/editor.api.js';
import 'monaco-editor/editor/browser/widget/diffEditor/diffEditor.contribution.js';
import 'monaco-editor/editor/browser/coreCommands.js';
import 'monaco-editor/editor/contrib/folding/browser/folding.js';
import 'monaco-editor/languages/definitions/cpp/register.js';
import 'monaco-editor/languages/definitions/typescript/register.js';
import 'monaco-editor/languages/definitions/python/register.js';
import 'monaco-editor/languages/definitions/markdown/register.js';
import '../../node_modules/monaco-editor/esm/vs/base/browser/ui/codicons/codicon/codicon.css';
import '../../node_modules/monaco-editor/esm/vs/base/browser/ui/codicons/codicon/codicon-modifiers.css';
import EditorWorker from 'monaco-editor/editor/editor.worker.js?worker';

// Emitted same-origin worker files: no CDN, blob worker, eval, or external runtime.
self.MonacoEnvironment = { getWorker: () => new EditorWorker() };

monaco.editor.defineTheme('maestro-dark', {
  base: 'vs-dark', inherit: true,
  rules: [
    { token: 'comment', foreground: '757575', fontStyle: 'italic' },
    { token: 'keyword', foreground: 'B5A1CE' },
    { token: 'string', foreground: 'B4BE9B' },
    { token: 'number', foreground: 'C7A98D' },
    { token: 'type.identifier', foreground: 'B1C0CE' },
  ],
  colors: {
    'editor.background': '#1B1B1B', 'editor.foreground': '#D4D4D4',
    'editorLineNumber.foreground': '#555555', 'editorLineNumber.activeForeground': '#9C9C9C',
    'editor.selectionBackground': '#8B5CF626', 'editor.lineHighlightBackground': '#FFFFFF03',
    'editorCursor.foreground': '#A78BFA', 'editorGutter.background': '#1B1B1B',
    'diffEditor.insertedTextBackground': '#739D6628', 'diffEditor.removedTextBackground': '#BC6C6C24',
    'diffEditor.insertedLineBackground': '#739D6610', 'diffEditor.removedLineBackground': '#BC6C6C0F',
    'diffEditorGutter.insertedLineBackground': '#739D6624', 'diffEditorGutter.removedLineBackground': '#BC6C6C20',
    'diffEditor.diagonalFill': '#232323', 'diffEditor.border': '#FFFFFF06',
    'scrollbarSlider.background': '#FFFFFF10', 'scrollbarSlider.hoverBackground': '#FFFFFF18',
    'editorOverviewRuler.border': '#00000000',
  },
});

export { monaco };

monaco.editor.defineTheme('maestro-light', {
  base: 'vs', inherit: true, rules: [], colors: {
    'editor.background': '#FFFFFF', 'editor.foreground': '#303030', 'editorGutter.background': '#FFFFFF',
    'editorLineNumber.foreground': '#A0A0A0', 'editor.selectionBackground': '#8B5CF622',
    'diffEditor.insertedLineBackground': '#6DAB6515', 'diffEditor.removedLineBackground': '#C46A6A12',
    'diffEditor.insertedTextBackground': '#6DAB652A', 'diffEditor.removedTextBackground': '#C46A6A24',
    'diffEditor.diagonalFill': '#F1F1F1', 'diffEditor.border': '#00000008',
  },
});
const applyTheme = () => monaco.editor.setTheme(document.documentElement.dataset.theme === 'light' ? 'maestro-light' : 'maestro-dark');
window.addEventListener('maestro-theme', applyTheme);
applyTheme();
if (import.meta.hot) import.meta.hot.dispose(() => window.removeEventListener('maestro-theme', applyTheme));

