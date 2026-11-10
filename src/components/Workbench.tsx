import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from 'react';
import * as DropdownMenu from '@radix-ui/react-dropdown-menu';
import * as Popover from '@radix-ui/react-popover';
import * as Tooltip from '@radix-ui/react-tooltip';
import { Archive, ArchiveRestore, ArrowDown, ArrowUp, ArrowUpRight, Bot, BrainCircuit, ChartNoAxesColumnIncreasing, Check, ChevronDown, Clock3, Code2, Command, Copy, FolderGit2, GitBranch, HelpCircle, ListChecks, Monitor, Moon, MoreHorizontal, PanelRightClose, PanelRightOpen, Paperclip, Plus, Puzzle, Settings2, ShieldCheck, Sparkles, Square, Sun, WandSparkles, Zap } from 'lucide-react';
import type { Mode, PlanCard as Plan, PlanDecision, ProjectView, RunStatus, RunView, SessionDetail, SessionView, Snapshot, RunMetrics } from '../api/types';
import CollapsibleBlock from './CollapsibleBlock';
import PlanCard from './PlanCard';
import ToolCallBlocks from './ToolCallBlocks';
import type { DiffViewState } from './DiffPanel';
import { Choice, IconButton } from './ui';
import WorkbenchLayout from './WorkbenchLayout';
import ProjectSessions from './ProjectSessions';
import { createMockDashboard } from './activityMock';
import type { WorkbenchProject } from './workspace';
import SettingsPanel from './SettingsPanel';
import { NewProjectDialog, DeleteProjectDialog } from './NewProjectDialog';
import ErrorToast from './ErrorToast';
import ComposerControls from './ComposerControls';
import RightPanel from './RightPanel';
import type { ExplorerViewState } from './ExplorerPanel';
import ExtensionsView from './ExtensionsView';
import useExtensions, { type ExtensionsController } from '../hooks/useExtensions';
import { formatShortcut, labels, readUiPreferences, saveUiPreferences, shortcutChord, type ShortcutAction, type UiPreferences, type UiView } from '../uiPreferences';
import { useWorkspace } from '../hooks/useWorkspace';
import CommandCenter, { type PaletteCommand } from './CommandCenter';
import { flattenFileTree } from '../commandCenter';
import { readPinnedSessions, savePinnedSessions } from '../sessionGroups';
import { mockExplorerFiles } from './workbenchMock';
import { pickDirectory } from '../api/dialog';
import { useAppearance } from '../useAppearance';
import MarkdownMessage from './MarkdownMessage';
import LocalAttachments, { type LocalAttachment } from './LocalAttachments';
import SessionStats from './SessionStats';
import { resolveCapability } from '../modelCatalog';
import type { ProviderId } from '../modelCapabilities';

const ActivityDashboard = lazy(() => import('./ActivityDashboard'));

export interface WorkbenchController {
  state: Snapshot; sessions: SessionView[]; projects?: ProjectView[]; selectedSessionId: number | null;
  liveTools?: Record<number, import('../api/types').ToolCallView[]>;
  liveReasoning?: Record<number, string>;
  detail: SessionDetail | null; runs: RunView[]; loadingSession: boolean; pending: boolean; error: string | null;
  metrics?: Record<number, RunMetrics>;
  selectSession: (id: number | null) => void;
  selectProject?: (id: number | null) => void;
  createSession: (projectId?: number) => unknown;
  createProject?: (name: string, path?: string) => Promise<{ project?: ProjectView; error?: string }>;
  setProjectArchived?: (projectId: number, archived: boolean) => unknown;
  deleteProject?: (projectId: number) => Promise<{ error?: string } | undefined>;
  updateProjectPath?: (projectId: number, path: string) => Promise<unknown>;
  renameProject?: (projectId: number, name: string) => Promise<unknown>;
  setSessionArchived?: (sessionId: number, archived: boolean) => unknown;
  renameSession?: (sessionId: number, title: string) => unknown;
  deleteSession?: (sessionId: number) => unknown;
  start: (prompt: string, images?: string[]) => Promise<number | undefined>;
  cancel: (id: number) => unknown;
  setMode: (mode: Mode) => unknown;
  decidePlan: (plan: Plan, decision: PlanDecision) => unknown;
}

const statusLabels: Record<RunStatus, string> = { running: '正在生成', completed: '已完成', cancelled: '已停止', failed: '失败' };

export default function Workbench({ controller: c, source = 'live', projects: suppliedProjects, onCreateProjectSession, sessionActions, extensions: suppliedExtensions }: {
  controller: WorkbenchController; source?: 'live' | 'preview'; projects?: WorkbenchProject[];
  extensions?: ExtensionsController;
  onCreateProjectSession?: (projectId: string) => void;
  sessionActions?: {
    rename?: (sessionId: number, title: string) => void;
    archive?: (sessionId: number, archived: boolean) => void;
    archiveProject?: (projectId: string, archived: boolean) => void;
    deleteProject?: (projectId: string) => void;
    deleteSession?: (sessionId: number) => void;
    createProject?: (name: string, path?: string) => Promise<string | null>;
  };
}) {
  const preview = source === 'preview';
  const [preferences, setPreferences] = useState(() => readUiPreferences(source));
  const updatePreferences = useCallback((value: UiPreferences) => { setPreferences(value); saveUiPreferences(source, value); }, [source]);
  const text = labels(preferences.language);
  const theme = useAppearance(preferences);
  const [capability, setCapability] = useState<{ model: string; provider: ProviderId | (string & {}); context: number | null }>({ model: '', provider: 'openai', context: null });
  const updateCapability = useCallback((model: string, provider: ProviderId | (string & {}), context: number | null) => setCapability({ model, provider, context }), []);
  const [attachments, setAttachments] = useState<LocalAttachment[]>([]);
  const [messageAttachments, setMessageAttachments] = useState<Record<number, LocalAttachment[]>>({});
  const attachmentUrls = useRef(new Set<string>());
  const fileInput = useRef<HTMLInputElement>(null);
  useEffect(() => () => { for (const url of attachmentUrls.current) URL.revokeObjectURL(url); attachmentUrls.current.clear(); }, []);
  const [prompt, setPrompt] = useState('');
  const [navigation, setNavigation] = useState<UiView>(() => preferences.startupView);
  const [selectedPlanId, setSelectedPlanId] = useState<number | null>(null);
  const [reviewOpen, setReviewOpen] = useState(true);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsVersion, setSettingsVersion] = useState(0);
  const [newProjectOpen, setNewProjectOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<WorkbenchProject | null>(null);
  const [rightTab, setRightTab] = useState<'review' | 'explorer'>('review');
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [pinnedIds, setPinnedIds] = useState<number[]>(() => readPinnedSessions(source));
  const workspace = useWorkspace(!preview && ((reviewOpen && rightTab === 'explorer') || paletteOpen));
  const [explorerView, setExplorerView] = useState<ExplorerViewState>({ selected: null, closedFolders: [] });
  const liveExtensions = useExtensions(!preview && navigation === 'extensions');
  const extensions = suppliedExtensions ?? liveExtensions;
  const [toastError, setToastError] = useState<string | null>(null);
  const dismissToast = useCallback(() => setToastError(null), []);
  const [diffView, setDiffView] = useState<DiffViewState>({ file: null, split: true });
  const togglePin = useCallback((sessionId: number, pinned: boolean) => {
    setPinnedIds((current) => {
      const next = pinned ? [...current, sessionId] : current.filter((id) => id !== sessionId);
      savePinnedSessions(source, next);
      return next;
    });
  }, [source]);
  const paletteFiles = useMemo(() => preview
    ? mockExplorerFiles.map((file) => ({ path: file.path, name: file.path.split('/').pop() ?? file.path }))
    : flattenFileTree(workspace.tree), [preview, workspace.tree]);
  const history = useRef<HTMLDivElement>(null);
  const historyScrollTop = useRef(0);
  const autoScroll = useRef(true);
  const [atBottom, setAtBottom] = useState(true);
  const [pinnedProjects, setPinnedProjects] = useState<string[]>(() => { try { return JSON.parse(localStorage.getItem('maestro.ui.pinnedProjects') ?? '[]'); } catch { return []; } });
  useEffect(() => { try { localStorage.setItem('maestro.ui.pinnedProjects', JSON.stringify(pinnedProjects)); } catch { /* quota */ } }, [pinnedProjects]);
  const lastSession = useRef(c.selectedSessionId);
  const promptBytes = new TextEncoder().encode(prompt.trim()).length;
  const plans = c.detail?.plans ?? [];
  const session = c.sessions.find((item) => item.id === c.selectedSessionId);
  const projects: WorkbenchProject[] = suppliedProjects ?? (c.projects ?? []).filter((project) => !project.archived).map((project) => ({
    id: String(project.id), name: project.name, path: project.path,
    sessions: c.sessions.filter((item) => item.projectId === project.id && !item.archived),
    archivedSessions: c.sessions.filter((item) => item.projectId === project.id && item.archived),
  }));
  const archivedProjects = (suppliedProjects ?? []).filter((project) => project.archived).map((project) => ({ key: project.id, numeric: null as number | null, name: project.name }))
    .concat(suppliedProjects ? [] : (c.projects ?? []).filter((project) => project.archived).map((project) => ({ key: String(project.id), numeric: project.id, name: project.name })));
  const currentProject = projects.find((project) => project.sessions.some((item) => item.id === c.selectedSessionId)) ?? projects[0] ?? { id: 'local', name: 'Maestro-Agent', sessions: [] };
  const dashboardData = useMemo(() => createMockDashboard(currentProject), [currentProject.id, currentProject.today?.changedLines, currentProject.today?.completedPlans]);
  const blocked = preview || !c.state.ready || c.pending || !!c.state.activeRun;

  useEffect(() => { if (c.error) setToastError(c.error); }, [c.error]);

  const paletteCommands = useMemo<PaletteCommand[]>(() => {
    const themeIcon = preferences.theme === 'light' ? <Sun size={14} /> : preferences.theme === 'system' ? <Monitor size={14} /> : <Moon size={14} />;
    const cycleTheme = () => updatePreferences({ ...preferences, theme: preferences.theme === 'dark' ? 'light' : preferences.theme === 'light' ? 'system' : 'dark' });
    const newSession = () => {
      setNavigation('agents');
      const numeric = Number(currentProject.id);
      if (preview) onCreateProjectSession?.(currentProject.id);
      else void c.createSession(Number.isFinite(numeric) ? numeric : undefined);
    };
    return [
      { id: 'new-session', label: '新建会话', icon: <Plus size={14} />, disabled: preview ? !onCreateProjectSession : !c.state.ready || c.pending, run: newSession },
      { id: 'new-project', label: '新建项目…', icon: <FolderGit2 size={14} />, run: () => setNewProjectOpen(true) },
      { id: 'agents', label: 'Go to Agents', icon: <Bot size={14} />, run: () => setNavigation('agents') },
      { id: 'activity', label: 'Go to Activity', hint: formatShortcut(preferences.shortcuts.activity), icon: <ChartNoAxesColumnIncreasing size={14} />, run: () => setNavigation('activity') },
      { id: 'extensions', label: 'Go to Extensions', hint: formatShortcut(preferences.shortcuts.extensions), icon: <Puzzle size={14} />, run: () => setNavigation('extensions') },
      { id: 'automations', label: 'Go to Automations', icon: <Zap size={14} />, run: () => setNavigation('automations') },
      { id: 'review', label: 'Toggle Review Panel', hint: formatShortcut(preferences.shortcuts.review), icon: <PanelRightOpen size={14} />, run: () => setReviewOpen((value) => !value) },
      { id: 'settings', label: 'Open Settings', hint: formatShortcut(preferences.shortcuts.settings), icon: <Settings2 size={14} />, run: () => setSettingsOpen(true) },
      { id: 'theme', label: `Theme: ${preferences.theme} (cycle)`, icon: themeIcon, run: cycleTheme },
      { id: 'mode-plan', label: 'Switch to Plan mode', icon: <ListChecks size={14} />, disabled: c.state.mode === 'plan', run: () => void c.setMode('plan') },
      { id: 'mode-code', label: 'Switch to Code mode', icon: <Code2 size={14} />, disabled: c.state.mode === 'code', run: () => void c.setMode('code') },
      { id: 'mode-confirm', label: 'Switch to Confirm mode', icon: <ShieldCheck size={14} />, disabled: c.state.mode === 'confirm', run: () => void c.setMode('confirm') },
      { id: 'mode-auto', label: 'Switch to Auto mode', icon: <Zap size={14} />, disabled: c.state.mode === 'auto', run: () => void c.setMode('auto') },
    ];
  }, [preferences, updatePreferences, preview, onCreateProjectSession, currentProject.id, c]);

  useEffect(() => {
    if (!preview && currentProject.path) void workspace.selectWorkspace(currentProject.path);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- workspace object identity changes every render; only the project context matters.
  }, [preview, currentProject.id, currentProject.path]);

  useEffect(() => {
    if (lastSession.current !== c.selectedSessionId) {
      lastSession.current = c.selectedSessionId;
      autoScroll.current = true;
      historyScrollTop.current = 0;
    }
    if (navigation === 'agents' && history.current) {
      history.current.scrollTop = !preview && autoScroll.current ? history.current.scrollHeight : historyScrollTop.current;
    }
  }, [preview, navigation, c.selectedSessionId, c.runs.at(-1)?.output, plans.length]);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (blocked || promptBytes === 0 || promptBytes > 8192) return;
    autoScroll.current = true;
    const submitted = prompt;
    const sentAttachments = attachments;
    const images = sentAttachments.map((item) => item.dataUrl).filter((url): url is string => Boolean(url));
    const texts = sentAttachments.filter((item) => item.text).map((item) => `

\`\`\`${item.name}
${item.text}\`\`\``).join('');
    if (images.length && !preview) {
      const vision = resolveCapability(capability.model, capability.provider).vision;
      if (!vision) { setToastError('当前模型不支持视觉输入，请先移除图片附件'); return; }
    }
    setPrompt('');
    const runId = await c.start(submitted + texts, images.length ? images : undefined);
    if (runId === undefined) setPrompt((current) => current || submitted);
    else { setMessageAttachments((current) => ({ ...current, [runId]: sentAttachments })); setAttachments([]); }
  }

  /// Strip tool-call narration leaked into persisted output (pre-fix runs) or
/// emitted by non-function-calling models. Keeps the intentional
/// `> 工具调用预算已用尽` note.
/// Models without `reasoning_content` emit inline <think>…</think> blocks —
/// pull them into the reasoning pane instead of the answer body.
function extractReasoning(text: string): { reasoning: string; content: string } {
  const parts: string[] = [];
  const content = text.replace(/<think>([\s\S]*?)(?:<\/think>|$)/g, (_m, inner: string) => { parts.push(inner); return ''; });
  return { reasoning: parts.join('\n'), content };
}

function cleanRunOutput(text: string): string {
  return text
    .replace(/^\s*>\s*调用工具[^\n]*\n?/gm, '')
    .replace(/^\s*调用工具[^\n]*\n?/gm, '')
    .replace(/\n{3,}/g, '\n\n');
}

function reviewPlan(id: number) { setSelectedPlanId(id); setRightTab('review'); setReviewOpen(true); }

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || settingsOpen || paletteOpen) return;
      const chord = shortcutChord(event);
      const action = (Object.keys(preferences.shortcuts) as ShortcutAction[]).find((id) => preferences.shortcuts[id] === chord);
      if (!action) return;
      // The command palette is a global gesture (works from any input); other
      // shortcuts stay suspended while typing to avoid background navigation.
      if (action !== 'search' && event.target instanceof Element && event.target.closest('input,textarea,[contenteditable="true"],.monaco-editor')) return;
      event.preventDefault();
      if (action === 'settings') setSettingsOpen(true);
      if (action === 'activity') setNavigation('activity');
      if (action === 'extensions') setNavigation('extensions');
      if (action === 'review') setReviewOpen((value) => !value);
      if (action === 'search') setPaletteOpen(true);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [preferences.shortcuts, settingsOpen, paletteOpen]);

  return <Tooltip.Provider delayDuration={250}>
    <div className={`workbench ${preview ? 'is-preview' : ''} ${!reviewOpen ? 'review-hidden' : ''}`} style={{ zoom: `${Number(preferences.fontScale) / 100}` }}>
      <WorkbenchLayout source={source} sidebar={
      <aside aria-label="会话列表" className="sessions-pane">
        <div className="brand"><span className="brand-mark"><Command size={19} strokeWidth={1.5} /></span><h1>Maestro-Agent</h1><span className="brand-version">beta</span></div>
        {archivedProjects.length > 0 && <div className="archived-projects" aria-label="已归档项目">
          <p className="session-group-label">已归档项目</p>
          {archivedProjects.map((project) => <button key={project.key} type="button" className="archived-project-row" disabled={!c.setProjectArchived && !sessionActions?.archiveProject}
            onClick={() => { if (preview) sessionActions?.archiveProject?.(project.key, false); else if (project.numeric !== null) void c.setProjectArchived?.(project.numeric, false); }}>
            <Archive size={12} /><span className="archived-project-name">{project.name}</span><ArchiveRestore size={11} /></button>)}
        </div>}
        <nav className="primary-navigation" aria-label="核心导航">
          <button className="navigation-item" aria-current={navigation === 'agents' ? 'page' : undefined} onClick={() => setNavigation('agents')}><Bot size={17} strokeWidth={1.5} /><span>{text.agents}</span><span className="navigation-count">{c.sessions.length}</span></button>
          <button className="navigation-item" aria-current={navigation === 'automations' ? 'page' : undefined} onClick={() => setNavigation('automations')}><Zap size={17} strokeWidth={1.5} /><span>{text.automations}</span><span className="soon-label">Soon</span></button>
          <button className="navigation-item" aria-current={navigation === 'extensions' ? 'page' : undefined} onClick={() => setNavigation('extensions')}><Puzzle size={17} strokeWidth={1.5} /><span>{text.extensions}</span><span className="soon-label">Mock</span></button>
        </nav>
        <button type="button" className="navigation-item activity-navigation" aria-label="Activity" aria-current={navigation === 'activity' ? 'page' : undefined} onClick={() => setNavigation('activity')}>
          <ChartNoAxesColumnIncreasing size={16} strokeWidth={1.5} /><span>Activity</span><span className="activity-navigation-summary">{dashboardData.todayLines} lines today</span>
        </button>
        <ProjectSessions projects={projects} currentProjectId={currentProject.id} source={source} selectedSessionId={c.selectedSessionId} activeSessionId={c.state.activeRun?.sessionId}
          pinnedIds={pinnedIds} onTogglePin={togglePin}
          canCreate={preview ? !!onCreateProjectSession : c.state.ready && !c.pending}
          onSelect={(id) => { setNavigation('agents'); c.selectSession(id); }}
          onSelectProject={(projectId) => { const numeric = Number(projectId); if (Number.isFinite(numeric)) { setNavigation('agents'); c.selectProject?.(numeric); } }}
          onNewProject={() => setNewProjectOpen(true)}
          onRenameProject={c.renameProject ? (id, name) => void c.renameProject!(id, name) : undefined}
          onTogglePinProject={(projectId) => setPinnedProjects((value) => value.includes(projectId) ? value.filter((item) => item !== projectId) : [...value, projectId])}
          pinnedProjectIds={pinnedProjects}
          onManageProject={() => setSettingsOpen(true)}
          onCreate={(projectId) => { setNavigation('agents'); const numeric = Number(projectId); if (preview) onCreateProjectSession?.(projectId); else void c.createSession(Number.isFinite(numeric) ? numeric : undefined); }}
          onRenameSession={preview ? sessionActions?.rename : c.renameSession ? (id, title) => void c.renameSession!(id, title) : undefined}
          onArchiveSession={preview ? sessionActions?.archive : c.setSessionArchived ? (id, archived) => void c.setSessionArchived!(id, archived) : undefined}
          onDeleteSession={preview ? sessionActions?.deleteSession : c.deleteSession ? (id) => void c.deleteSession!(id) : undefined}
          onArchiveProject={preview ? sessionActions?.archiveProject : c.setProjectArchived ? (id, archived) => void c.setProjectArchived!(Number(id), archived) : undefined}
          onDeleteProject={preview ? sessionActions?.deleteProject : c.deleteProject ? (id) => setDeleteTarget(projects.find((project) => project.id === id) ?? null) : undefined} />
        <div className="sidebar-bottom">
          <div className="connection-status"><span className={`status-dot ${c.state.ready && !preview ? 'connected' : ''}`} /><span role="status">{preview ? 'Frontend preview' : c.state.ready ? 'Rust 内核已就绪' : '正在连接 Rust 内核…'}</span><span className="local-tag">{preview ? 'MOCK' : 'LOCAL'}</span></div>
          <button type="button" className="navigation-item sidebar-settings-button" onClick={() => setSettingsOpen(true)}><Settings2 size={16} strokeWidth={1.5} /><span>{text.settings}</span></button>
        </div>
      </aside>
      } conversation={
      <main aria-label={navigation === 'activity' ? 'Activity 工作区' : 'Agent 对话'} className="conversation-pane">
        <header className="conversation-header">
          <div className="conversation-title"><span className="eyebrow">{navigation === 'agents' ? 'AGENT SESSION' : 'WORKSPACE'}</span><h2>{navigation === 'agents' ? session?.title ?? 'New agent' : navigation === 'activity' ? 'Workspace overview' : navigation === 'extensions' ? text.extensions : text.automations}</h2></div>
          <div className="header-actions"><IconButton label={reviewOpen ? '隐藏代码审查' : '显示代码审查'} onClick={() => setReviewOpen(!reviewOpen)}>{reviewOpen ? <PanelRightClose size={16} /> : <PanelRightOpen size={16} />}</IconButton>
            <IconButton label="会话信息" onClick={() => setSettingsOpen(true)}><MoreHorizontal size={17} /></IconButton></div>
        </header>
        {navigation === 'activity'
          ? <div className="session-context-strip"><span><ChartNoAxesColumnIncreasing size={12} />Mock analytics</span><span>{dashboardData.start} — {dashboardData.end}</span></div>
          : <div className="session-context-strip"><span><GitBranch size={12} />{preview ? 'refactor/session-lifecycle' : 'local / planning'}</span><span><ShieldCheck size={12} />{c.state.mode === 'plan' ? 'Plan mode' : 'Execute mode'}</span>{preview && <span className="preview-tag">示例数据 · 无 Rust IPC</span>}</div>}
        {c.error && <div className="error-message">{c.error}</div>}
        {navigation === 'activity' ? <Suspense fallback={<div role="status" className="dashboard-loading">Loading dashboard…</div>}>
          <ActivityDashboard data={dashboardData} projectName={currentProject.name} onBack={() => setNavigation('agents')} />
        </Suspense> : navigation === 'extensions' ? <ExtensionsView controller={extensions} /> : navigation === 'automations' ? <div className="automation-page"><span className="welcome-icon"><Zap size={27} strokeWidth={1.25} /></span><h3>Your agents, on autopilot.</h3><p>定时执行重复性任务，让专注留给更重要的工作。</p><div className="automation-placeholder"><Clock3 size={19} /><div><strong>Scheduled agents</strong><p>自动任务尚未接入后端，本次重构仅展示入口。</p></div><span className="soon-label">Coming soon</span></div><button className="button-ghost" onClick={() => setNavigation('agents')}>返回 Agents<ArrowUpRight size={14} /></button></div>
          : <>
            <div ref={history} className="chat-history" aria-label="对话消息" onScroll={() => { const element = history.current; if (element) { historyScrollTop.current = element.scrollTop; const near = element.scrollHeight - element.scrollTop - element.clientHeight < 96; autoScroll.current = near; setAtBottom(near); } }}>
              <div className="chat-history-inner">
                {c.runs.length === 0 && <div className="conversation-welcome"><span className="welcome-icon"><WandSparkles size={27} strokeWidth={1.2} /></span><h3>What should we build?</h3><p>描述你的任务。Agent 会先规划，再提交可审查的变更。</p><div className="welcome-suggestions">重构模块<span>·</span>排查问题<span>·</span>审查代码</div></div>}
                {c.loadingSession && c.runs.length === 0 && <p className="loading-note">正在加载会话…</p>}
                {c.runs.map((run) => <section key={run.id} className="conversation-turn" aria-label={`对话轮次 ${run.id}`}>
                  <article className="user-message"><div className="message-meta"><span className="user-avatar">Y</span><strong>You</strong><button type="button" className="user-copy-button" aria-label={`复制用户消息 ${run.id}`} onClick={async () => { try { await navigator.clipboard.writeText(run.prompt); } catch { setToastError('Cannot access clipboard'); } }}><Copy size={12} /></button><span>{preview ? '11:42 AM' : `#${run.id}`}</span></div><p>{run.prompt}</p><LocalAttachments items={messageAttachments[run.id] ?? []} sent /></article>
                  {(() => { const calls = [...(c.detail?.tools ?? []), ...(c.liveTools?.[run.id] ?? [])].filter((call) => call.runId === run.id);
                    return calls.length > 0 && <ToolCallBlocks calls={calls} />; })()}
                  {(() => {
                    const { reasoning: inlineReasoning, content } = extractReasoning(run.output || '');
                    const reasoning = (c.liveReasoning?.[run.id] ?? '') + inlineReasoning;
                    return <>
                      {reasoning && preferences.reasoningDisplay !== 'hidden' &&
                        <CollapsibleBlock title="思考过程" icon={<BrainCircuit size={13} />} subtitle="思考" defaultOpen={preferences.reasoningDisplay === 'summary'}
                          contentClass={`reasoning-content ${preferences.reasoningDisplay === 'summary' ? 'reasoning-clamp' : ''}`}><p>{reasoning}</p></CollapsibleBlock>}
                      <div aria-label={`回复内容 ${run.id}`} aria-busy={run.status === 'running'} className="agent-copy"><MarkdownMessage content={cleanRunOutput(content) || (run.status === 'running' ? '思考中...' : '无输出')} light={theme === 'light'} />{run.status === 'running' && <span aria-hidden="true" className="typing-cursor" />}</div>
                    </>;
                  })()}
                  <article aria-label={`Agent 回复 ${run.id}`} className="agent-message"><div className="message-meta"><span className="agent-avatar"><Sparkles size={14} strokeWidth={1.5} /></span><strong>Maestro</strong><span className="message-model">Agent</span><button type="button" className="user-copy-button" aria-label={`复制回复 ${run.id}`} onClick={async () => { try { await navigator.clipboard.writeText(run.output); } catch { setToastError('Cannot access clipboard'); } }}><Copy size={12} /></button><span>{statusLabels[run.status]}</span></div>
                    {run.error && <p className="error-message">{run.error}</p>}
                    {plans.filter((plan) => plan.runId === run.id).map((plan) => <PlanCard key={plan.id} plan={plan} preview={preview} canDecide={!preview && run.status === 'completed'} pending={c.pending} onReview={reviewPlan} onDecision={(_id, decision) => { if (!preview) void c.decidePlan(plan, decision); }} />)}
                    <div className="message-footnote"><ShieldCheck size={12} />{preview ? '示例对话 · 不执行任何操作' : '模型回复 · 不执行工具或应用修改'}</div>
                  </article>
                </section>)}
              </div>
            </div>
            <div className="composer-area"><button type="button" className={`scroll-bottom-fab ${atBottom ? '' : 'visible'}`} aria-label="回到底部" onClick={() => { autoScroll.current = true; setAtBottom(true); history.current?.scrollTo({ top: history.current.scrollHeight, behavior: 'smooth' }); }}><ArrowDown size={16} /></button><form onSubmit={(event) => void submit(event)} className="composer">
              <LocalAttachments items={attachments} onRemove={(id) => setAttachments((items) => items.filter((item) => { if (item.id === id) { URL.revokeObjectURL(item.url); attachmentUrls.current.delete(item.url); return false; } return true; }))} />
              <input ref={fileInput} type="file" hidden multiple aria-label="选择本地媒体附件" accept="image/png,image/jpeg,image/webp,image/gif,video/mp4,video/webm,.md,.txt,.py,.ts,.tsx,.js,.jsx,.json,.rs,.toml,.yaml,.yml,.log,.sql,.sh" onChange={(event) => {
                const files = Array.from(event.target.files ?? []);
                const added: LocalAttachment[] = [];
                for (const file of files.slice(0, Math.max(0, 6 - attachments.length))) {
                  if (file.size > 20 * 1024 * 1024) { setToastError('附件超过 20 MiB 上限'); continue; }
                  const url = URL.createObjectURL(file); attachmentUrls.current.add(url);
                  const type: LocalAttachment['type'] = file.type.startsWith('image/') ? 'image' : file.type.startsWith('video/') ? 'video' : 'text';
                  const entry: LocalAttachment = { id: crypto.randomUUID(), name: file.name, type, url };
                  if (type === 'image' && file.size <= 3 * 1024 * 1024) void new Promise<string | undefined>((resolve) => {
                    const reader = new FileReader(); reader.onload = () => resolve(typeof reader.result === 'string' ? reader.result : undefined); reader.onerror = () => resolve(undefined); reader.readAsDataURL(file);
                  }).then((dataUrl) => { if (dataUrl) setAttachments((items) => items.map((item) => item.id === entry.id ? { ...item, dataUrl } : item)); });
                  else if (type === 'text' && file.size <= 64 * 1024) void file.text().then((text) => setAttachments((items) => items.map((item) => item.id === entry.id ? { ...item, text } : item))).catch(() => undefined);
                  added.push(entry);
                }
                setAttachments((items) => [...items, ...added]); event.target.value = '';
              }} />
              <textarea aria-label="发送消息" rows={3} value={prompt} onChange={(event) => setPrompt(event.target.value)} placeholder="描述你的任务... Ctrl+Enter 发送"
                disabled={blocked} onKeyDown={(event) => { if ((event.ctrlKey || event.metaKey) && event.key === 'Enter' && !event.nativeEvent.isComposing) { event.preventDefault(); event.currentTarget.form?.requestSubmit(); } }} />
              <div className="composer-toolbar"><IconButton label="添加图片或视频" disabled={!preview && (!!c.state.activeRun || c.pending)} onClick={() => fileInput.current?.click()}><Paperclip size={17} /></IconButton>

                <Choice label="由 Rust 强制执行" value={c.state.mode} disabled={!preview && blocked} onChange={(value) => void c.setMode(value as Mode)} options={[
                  { value: 'plan', label: <span className="choice-item-stack"><span className="choice-item-main"><ListChecks size={13} />Plan</span><span className="choice-item-desc">只读分析，不修改文件</span></span> },
                  { value: 'code', label: <span className="choice-item-stack"><span className="choice-item-main"><Code2 size={13} />Code</span><span className="choice-item-desc">每次 Diff 必须批准；未开放写入执行器</span></span> },
                  { value: 'confirm', label: <span className="choice-item-stack"><span className="choice-item-main"><ShieldCheck size={13} />Confirm</span><span className="choice-item-desc">每次工具/命令须确认；未开放执行器</span></span> },
                  { value: 'auto', label: <span className="choice-item-stack"><span className="choice-item-main"><Zap size={13} />Auto</span><span className="choice-item-desc">自动策略仅适用于已实现的能力；当前不执行文件/命令操作</span></span> },
                ]} />
                <Popover.Root>
                  <Popover.Trigger asChild><button type="button" className="composer-project-chip" disabled={!preview && blocked} aria-label="当前项目"><FolderGit2 size={13} />{currentProject.name}{!currentProject.path && !preview && <small className="project-picker-no-path"> 未设目录</small>}<ChevronDown size={11} /></button></Popover.Trigger>
                  <Popover.Portal><Popover.Content className="model-picker project-picker" side="top" align="start" sideOffset={8}>
                    <div className="model-picker-header"><FolderGit2 size={14} />项目<small className="model-picker-api">workspace</small></div>
                    <div className="model-picker-list" role="listbox" aria-label="项目列表">
                      {projects.map((project) => {
                        const numeric = Number(project.id);
                        return <div key={project.id} className="model-picker-row project-picker-row" role="option" aria-selected={currentProject.id === project.id}
                          onClick={() => { if (!preview && Number.isFinite(numeric)) c.selectProject?.(numeric); }}>
                          <span className="model-picker-id">{project.name}{!project.path && <small className="project-picker-no-path"> 未设目录</small>}</span>
                          {!preview && !project.path && Number.isFinite(numeric) && c.updateProjectPath &&
                            <button type="button" className="project-picker-setpath" aria-label={`设置目录 ${project.name}`} onClick={async (event) => {
                              event.stopPropagation();
                              const dir = await pickDirectory();
                              if (dir) await c.updateProjectPath?.(numeric, dir);
                            }}>设置目录</button>}
                          {project.path && <span className="model-picker-context project-picker-path" title={project.path}>{project.path.split(/[\/]/).pop()}</span>}
                        </div>;
                      })}
                    </div>
                    <button type="button" className="model-picker-level" onClick={() => setNewProjectOpen(true)}>＋ 新建项目</button>
                  </Popover.Content></Popover.Portal>
                </Popover.Root>
                <div className="flex-1" /><ComposerControls preview={preview} disabled={blocked} preferences={preferences} onPreferences={updatePreferences} settingsVersion={settingsVersion} onError={setToastError} onCapability={updateCapability} />
                <span className={`input-count ${promptBytes > 8192 ? 'invalid' : ''}`}>{promptBytes > 0 ? `${promptBytes} / 8192` : 'Ctrl ↵'}</span>
                {c.state.activeRun?.status === 'running' && !preview ? <IconButton label="停止生成" className="stop-button" disabled={c.pending} onClick={() => void c.cancel(c.state.activeRun!.id)}><Square size={13} /></IconButton> : <button type="submit" aria-label="发送" className="send-button" disabled={blocked || promptBytes === 0 || promptBytes > 8192}><ArrowUp size={19} strokeWidth={1.8} /></button>}
              </div>
            </form><SessionStats runs={c.runs} metrics={c.metrics} context={capability.context} /><div className="composer-caption"><span><ShieldCheck size={11} />{preview ? '预览不调用模型；媒体仅本地显示' : '图片/文本附件随消息发送（图片需视觉模型）；视频仅本地预览。'}</span><span>{capability.model || 'Maestro Agent'}</span></div></div>
          </>}
      </main>
      } review={reviewOpen ? <RightPanel tab={rightTab} onTabChange={setRightTab} plans={plans} selectedId={selectedPlanId} onSelect={setSelectedPlanId} diffView={diffView} onDiffViewChange={setDiffView} explorerView={explorerView} onExplorerViewChange={setExplorerView} scope={`${currentProject.id}:${c.selectedSessionId}`} workspaceName={currentProject.name} fontSize={preferences.editorFontSize} fontFamily={preferences.editorFont} preview={preview} workspace={workspace} /> : null} />
    </div>
    <SettingsPanel open={settingsOpen} onOpenChange={setSettingsOpen} preview={preview} preferences={preferences} onPreferences={updatePreferences} onModelsChanged={() => setSettingsVersion((value) => value + 1)} />
    <CommandCenter open={paletteOpen} onOpenChange={setPaletteOpen} source={source} projects={projects} commands={paletteCommands}
      files={paletteFiles} filesLoading={!preview && paletteOpen && workspace.loading}
      onSelectSession={(id) => { setNavigation('agents'); c.selectSession(id); }}
      onSelectFile={(path) => {
        setReviewOpen(true); setRightTab('explorer');
        if (preview) {
          const file = mockExplorerFiles.find((item) => item.path === path);
          if (file) setExplorerView((view) => ({ ...view, selected: `${currentProject.id}:${c.selectedSessionId}:mock:${file.id}` }));
        } else void workspace.selectFile(path);
      }} />
    <NewProjectDialog open={newProjectOpen} onOpenChange={setNewProjectOpen} disabled={!preview && (!c.state.ready || c.pending)}
      onBrowse={preview ? undefined : pickDirectory}
      onCreate={async (name, path) => {
        if (preview) return sessionActions?.createProject ? sessionActions.createProject(name, path) : null;
        if (!c.createProject) return '后端不支持创建项目';
        const result = await c.createProject(name, path);
        return result.error ?? null;
      }} />
    <DeleteProjectDialog project={deleteTarget} onOpenChange={(open) => { if (!open) setDeleteTarget(null); }}
      onConfirm={async (projectId) => {
        if (!c.deleteProject) return '后端不支持删除项目';
        const result = await c.deleteProject(Number(projectId));
        return result?.error ?? null;
      }} />
    <ErrorToast message={toastError} onDismiss={dismissToast} />
  </Tooltip.Provider>;
}

