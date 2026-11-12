export type UiView = 'agents' | 'activity' | 'automations' | 'extensions';
export type ShortcutAction = 'settings' | 'activity' | 'extensions' | 'review' | 'search';
export interface UiPreferences {
  language: 'en' | 'zh-CN'; startupView: 'agents' | 'activity' | 'extensions';
  theme: 'dark' | 'light' | 'system'; editorFont: string; editorFontSize: number;
  fontScale: '100' | '125' | '150';
  reasoningDisplay: 'collapsed' | 'summary' | 'hidden';
  contextWindow: 8192 | 32768 | 128000; temperature: number;
  provider: 'openai' | 'deepseek' | 'anthropic' | 'ollama'; previewModel: string;
  shortcuts: Record<ShortcutAction, string>;
}

export const defaultShortcuts: Record<ShortcutAction, string> = {
  settings: 'Mod+Comma', activity: 'Mod+Shift+KeyA', extensions: 'Mod+Shift+KeyX', review: 'Mod+Shift+KeyD', search: 'Mod+KeyK',
};
export const shortcutLabels: Record<ShortcutAction, string> = {
  settings: 'Open Settings', activity: 'Open Activity', extensions: 'Open Extensions', review: 'Toggle Review', search: 'Command Center',
};
export const defaultUiPreferences: UiPreferences = {
  language: 'en', startupView: 'agents', theme: 'dark', editorFont: 'Cascadia Code', editorFontSize: 12, fontScale: '100', reasoningDisplay: 'collapsed',
  contextWindow: 8192, temperature: 0.7, provider: 'openai', previewModel: 'deepseek-chat', shortcuts: defaultShortcuts,
};

export function shortcutChord(event: Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>): string | null {
  if ((!event.ctrlKey && !event.metaKey && !event.altKey) || /^(Control|Alt|Shift|Meta)/.test(event.code)) return null;
  return [event.ctrlKey || event.metaKey ? 'Mod' : '', event.altKey ? 'Alt' : '', event.shiftKey ? 'Shift' : '', event.code].filter(Boolean).join('+');
}

export function bindShortcut(preferences: UiPreferences, action: ShortcutAction, chord: string): UiPreferences {
  if (!/^(Mod\+|Alt\+)(Alt\+)?(Shift\+)?(Key[A-Z]|Digit[0-9]|Comma|Period|Slash|Backslash|Semicolon|Quote|BracketLeft|BracketRight|Minus|Equal|F[1-9][0-2]?)$/.test(chord)
    || Object.entries(preferences.shortcuts).some(([id, value]) => id !== action && value === chord)) {
    throw new Error('快捷键冲突或不支持；Ctrl/Cmd+Enter 保留用于发送。');
  }
  return { ...preferences, shortcuts: { ...preferences.shortcuts, [action]: chord } };
}

export function formatShortcut(chord: string) { return chord.replace('Mod', 'Ctrl').replace(/Key(?=[A-Z])/g, '').replace('Comma', ',').replace('Digit', ''); }

export function readUiPreferences(source: 'live' | 'preview'): UiPreferences {
  const result = structuredClone(defaultUiPreferences);
  try {
    const value = JSON.parse(localStorage.getItem(`maestro.ui.v1.${source}`) ?? 'null');
    if (!value || typeof value !== 'object') return result;
    if (['en', 'zh-CN'].includes(value.language)) result.language = value.language;
    if (['agents', 'activity', 'extensions'].includes(value.startupView)) result.startupView = value.startupView;
    if (['dark', 'light', 'system'].includes(value.theme)) result.theme = value.theme;
    if (['Cascadia Code', 'Consolas', 'JetBrains Mono'].includes(value.editorFont)) result.editorFont = value.editorFont;
    if (Number.isInteger(value.editorFontSize) && value.editorFontSize >= 10 && value.editorFontSize <= 20) result.editorFontSize = value.editorFontSize;
    if (['100', '125', '150'].includes(value.fontScale)) result.fontScale = value.fontScale;
    if (['collapsed', 'summary', 'hidden'].includes(value.reasoningDisplay)) result.reasoningDisplay = value.reasoningDisplay;
    if ([8192, 32768, 128000].includes(value.contextWindow)) result.contextWindow = value.contextWindow;
    if (Number.isFinite(value.temperature) && value.temperature >= 0 && value.temperature <= 2) result.temperature = value.temperature;
    if (['openai', 'deepseek', 'anthropic', 'ollama'].includes(value.provider)) result.provider = value.provider;
    if (typeof value.previewModel === 'string' && value.previewModel.length < 128) result.previewModel = value.previewModel;
    for (const action of Object.keys(defaultShortcuts) as ShortcutAction[]) {
      if (typeof value.shortcuts?.[action] === 'string') {
        try { result.shortcuts = bindShortcut(result, action, value.shortcuts[action]).shortcuts; } catch { /* Ignore bad bindings. */ }
      }
    }
  } catch { /* UI preferences cannot prevent startup. */ }
  return result;
}

export function saveUiPreferences(source: 'live' | 'preview', preferences: UiPreferences) {
  try { localStorage.setItem(`maestro.ui.v1.${source}`, JSON.stringify(preferences)); } catch { /* Presentation only. */ }
}

export function labels(language: UiPreferences['language']) {
  return language === 'zh-CN'
    ? { agents: '会话', automations: '自动任务', extensions: '扩展', settings: '设置', general: '常规', appearance: '外观', models: '模型', shortcuts: '快捷键', agentSession: '智能体会话', workspace: '工作区', newAgent: '新会话', workspaceOverview: '工作区概览', backToChat: '返回对话', activityDashboard: '活动面板', workspaceInsights: '工作区洞察', proposedPlan: '建议计划', workspaceFiles: '工作区文件', localReadOnly: '本地 / 只读', noWorkspace: '未选择工作区', contextUsed: '上下文已用', workspaceCapabilities: '工作区能力', virtual: '虚拟' }
    : { agents: 'Agents', automations: 'Automations', extensions: 'Extensions', settings: 'Settings', general: 'General', appearance: 'Appearance', models: 'Models', shortcuts: 'Shortcuts', agentSession: 'AGENT SESSION', workspace: 'WORKSPACE', newAgent: 'New agent', workspaceOverview: 'Workspace overview', backToChat: 'Back to Chat', activityDashboard: 'Activity Dashboard', workspaceInsights: 'WORKSPACE INSIGHTS', proposedPlan: 'PROPOSED PLAN', workspaceFiles: 'WORKSPACE FILES', localReadOnly: 'local / read only', noWorkspace: 'no workspace', contextUsed: 'context used', workspaceCapabilities: 'WORKSPACE CAPABILITIES', virtual: 'Virtual' };
}
