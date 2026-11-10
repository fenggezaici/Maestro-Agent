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
import ChatStream from './ChatStream';
import Composer from './Composer';
import { resolveCapability } from '../modelCatalog';
import type { ProviderId } from '../modelCapabilities';

const ActivityDashboard = lazy(() => import('./ActivityDashboard'));

export interface WorkbenchController {
  state: Snapshot; sessions: SessionView[]; projects?: ProjectView[]; selectedSessionId: number | null;
  liveTools?: Record<number, import('../api/types').ToolCallView[]>;
  liveReasoning?: Record<number, string>;
  liveReasoningMs?: Record<number, number>;
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
            <ChatStream historyRef={history} runs={c.runs} loadingSession={c.loadingSession}
              detailTools={c.detail?.tools} liveTools={c.liveTools} liveReasoning={c.liveReasoning} liveReasoningMs={c.liveReasoningMs}
              plans={plans} reasoningDisplay={preferences.reasoningDisplay} theme={theme} preview={preview}
              messageAttachments={messageAttachments} pending={c.pending}
              onCopyError={setToastError} onReviewPlan={reviewPlan} onDecidePlan={(plan, decision) => void c.decidePlan(plan, decision)}
              onScroll={() => { const element = history.current; if (element) { historyScrollTop.current = element.scrollTop; const near = element.scrollHeight - element.scrollTop - element.clientHeight < 96; autoScroll.current = near; setAtBottom(near); } }} />
            <Composer atBottom={atBottom} onScrollBottom={() => { autoScroll.current = true; setAtBottom(true); history.current?.scrollTo({ top: history.current.scrollHeight, behavior: 'smooth' }); }}
              onSubmit={(event) => void submit(event)} attachments={attachments} onAttachmentsChange={setAttachments}
              blocked={blocked} prompt={prompt} onPromptChange={setPrompt} promptBytes={promptBytes}
              preview={preview} mode={c.state.mode} onSetMode={(mode) => void c.setMode(mode)}
              currentProject={currentProject} projects={projects}
              onSelectProject={(id) => c.selectProject?.(id)}
              onSetProjectPath={c.updateProjectPath ? (id, dir) => c.updateProjectPath!(id, dir) : undefined}
              onBrowse={preview ? undefined : pickDirectory}
              onNewProject={() => setNewProjectOpen(true)}
              preferences={preferences} onPreferences={updatePreferences} settingsVersion={settingsVersion}
              onError={setToastError} onCapability={updateCapability}
              running={!!c.state.activeRun && c.state.activeRun.status === 'running'} pending={c.pending}
              onCancel={() => void c.cancel(c.state.activeRun!.id)}
              runs={c.runs} metrics={c.metrics} context={capability.context} modelLabel={capability.model} />
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

