export type UiView = 'agents' | 'activity' | 'automations' | 'extensions' | 'sandbox';
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
  settings: '打开设置', activity: '打开活动', extensions: '打开扩展', review: '切换右侧面板', search: '命令面板',
};
export const defaultUiPreferences: UiPreferences = {
  language: 'zh-CN', startupView: 'agents', theme: 'dark', editorFont: 'Cascadia Code', editorFontSize: 12, fontScale: '100', reasoningDisplay: 'collapsed',
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

const zhDashboard = {
  locale: 'zh-CN',
  actScopeAll: '全部项目',
  actScopeAria: '活动范围',
  actPastYear: '过去一年的工作节奏',
  actLoading: '正在读取本地活动数据…',
  actLoadFailed: '活动数据加载失败',
  actMockNotice: '所有指标均为前端 Mock，不代表真实代码修改、模型用量或账单。',
  actRunCommandNote: '修改行数按每次写入的增删累计（write_file/apply_patch）；run_command 产生的文件改动暂不统计，历史会话没有 before/after 记录。',
  actTotalLines: '累计修改行数',
  actTotalTokens: '累计 Token 数',
  actPeakTokens: '峰值 Token（日）',
  actLongestRun: '最长任务时长',
  actCurrentStreak: '当前连续天数',
  actLongestStreak: '最长连续天数',
  actLinesChanged: '写入工具增删行',
  actModelTokens: '模型消耗 token',
  actPeakDetail: '单日最高消耗',
  actRunDetail: '单个 run 的耗时',
  actStreakDetail: '有代码或 token 活动',
  actContributions: '代码活动',
  actContribSubtitle: '过去一年的代码与计划活动',
  actDaily: '每日',
  actWeekly: '每周',
  actCumulative: '累计',
  actLess: '少',
  actMore: '多',
  actMon: '一',
  actWed: '三',
  actFri: '五',
  actTrendTitle: 'Token 用量趋势',
  actModelUsage: '模型用量',
  actRuns: '次运行',
  actLast7: '近 7 日',
  actLast30: '近 30 日',
  actDayAria: '活跃度日历，可横向滚动',
  actDayUnit: '天',
  actDayTooltip: (lines: number, tokens: number, plans: number) => `修改 ${lines.toLocaleString('zh-CN')} 行代码 · ${tokens.toLocaleString('zh-CN')} tokens · ${plans} 个计划`,
  actTrendCaption: (count: number) => `${count} 个每日样本 · 本地时区 · 来自本地 run_metrics。`,
  actDuration: (ms: number) => {
    const totalMinutes = Math.floor(ms / 60000);
    const days = Math.floor(totalMinutes / 1440);
    const hours = Math.floor((totalMinutes % 1440) / 60);
    const minutes = totalMinutes % 60;
    return days > 0 ? `${days} 天 ${hours} 小时 ${minutes} 分` : hours > 0 ? `${hours} 小时 ${minutes} 分` : `${minutes} 分钟`;
  },
};

const enDashboard: typeof zhDashboard = {
  locale: 'en-US',
  actScopeAll: 'All projects',
  actScopeAria: 'Activity scope',
  actPastYear: 'Activity over the past year',
  actLoading: 'Loading local activity…',
  actLoadFailed: 'Failed to load activity',
  actMockNotice: 'All metrics are frontend mocks and do not reflect real edits, usage, or billing.',
  actRunCommandNote: 'Line counts accumulate per write (write_file/apply_patch); edits made inside run_command are not tracked, and pre-instrumentation sessions have no before/after.',
  actTotalLines: 'Total lines changed',
  actTotalTokens: 'Total tokens',
  actPeakTokens: 'Peak tokens (day)',
  actLongestRun: 'Longest run',
  actCurrentStreak: 'Current streak',
  actLongestStreak: 'Longest streak',
  actLinesChanged: 'Write-tool added/removed',
  actModelTokens: 'Model tokens consumed',
  actPeakDetail: 'Highest single-day usage',
  actRunDetail: 'Duration of a single run',
  actStreakDetail: 'Days with code or token activity',
  actContributions: 'Code activity',
  actContribSubtitle: 'Code and plan activity over the past year',
  actDaily: 'Daily',
  actWeekly: 'Weekly',
  actCumulative: 'Cumulative',
  actLess: 'Less',
  actMore: 'More',
  actMon: 'Mon',
  actWed: 'Wed',
  actFri: 'Fri',
  actTrendTitle: 'Token usage trend',
  actModelUsage: 'Model usage',
  actRuns: 'runs',
  actLast7: 'Last 7 days',
  actLast30: 'Last 30 days',
  actDayAria: 'Activity calendar, scroll horizontally',
  actDayUnit: 'd',
  actDayTooltip: (lines, tokens, plans) => `${lines.toLocaleString('en-US')} lines changed · ${tokens.toLocaleString('en-US')} tokens · ${plans} plans`,
  actTrendCaption: (count) => `${count} daily samples · local timezone · from local run_metrics.`,
  actDuration: (ms) => {
    const totalMinutes = Math.floor(ms / 60000);
    const days = Math.floor(totalMinutes / 1440);
    const hours = Math.floor((totalMinutes % 1440) / 60);
    const minutes = totalMinutes % 60;
    return days > 0 ? `${days}d ${hours}h ${minutes}m` : hours > 0 ? `${hours}h ${minutes}m` : `${minutes}m`;
  },
};

export function labels(language: UiPreferences['language']) {
  return language === 'zh-CN'
    ? { agents: '会话', automations: '自动任务', extensions: '扩展', sandbox: '架构沙盘', settings: '设置', general: '常规', appearance: '外观', models: '模型', shortcuts: '快捷键', subagents: '子智能体', agentSession: '智能体会话', workspace: '工作区', newAgent: '新会话', workspaceOverview: '工作区概览', backToChat: '返回对话', activityDashboard: '活动面板', workspaceInsights: '工作区洞察', proposedPlan: '建议计划', workspaceFiles: '工作区文件', localReadOnly: '本地 / 只读', noWorkspace: '未选择工作区', contextUsed: '上下文已用', workspaceCapabilities: '工作区能力', virtual: '虚拟', ...zhDashboard }
    : { agents: 'Agents', automations: 'Automations', extensions: 'Extensions', sandbox: 'Architecture Sandbox', settings: 'Settings', general: 'General', appearance: 'Appearance', models: 'Models', shortcuts: 'Shortcuts', subagents: 'Subagents', agentSession: 'AGENT SESSION', workspace: 'WORKSPACE', newAgent: 'New agent', workspaceOverview: 'Workspace overview', backToChat: 'Back to Chat', activityDashboard: 'Activity Dashboard', workspaceInsights: 'WORKSPACE INSIGHTS', proposedPlan: 'PROPOSED PLAN', workspaceFiles: 'WORKSPACE FILES', localReadOnly: 'local / read only', noWorkspace: 'no workspace', contextUsed: 'context used', workspaceCapabilities: 'WORKSPACE CAPABILITIES', virtual: 'Virtual', ...enDashboard };
}
