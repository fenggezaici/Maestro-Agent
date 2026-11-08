import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from 'react';
import * as DropdownMenu from '@radix-ui/react-dropdown-menu';
import * as Tooltip from '@radix-ui/react-tooltip';
import { ArrowUp, ArrowUpRight, Bot, ChartNoAxesColumnIncreasing, Check, ChevronDown, Clock3, Code2, Command, FolderGit2, GitBranch, HelpCircle, ListChecks, MoreHorizontal, PanelRightClose, PanelRightOpen, Paperclip, Puzzle, Settings2, ShieldCheck, Sparkles, Square, WandSparkles, Zap } from 'lucide-react';
import type { Mode, PlanCard as Plan, PlanDecision, RunStatus, RunView, SessionDetail, SessionView, Snapshot } from '../ipc';
import PlanCard from './PlanCard';
import type { DiffViewState } from './DiffPanel';
import { Choice, IconButton } from './ui';
import WorkbenchLayout from './WorkbenchLayout';
import ProjectSessions from './ProjectSessions';
import { createMockDashboard } from './activityMock';
import type { WorkbenchProject } from './workspace';
import SettingsPanel from './SettingsPanel';
import ErrorToast from './ErrorToast';
import ComposerControls from './ComposerControls';
import RightPanel from './RightPanel';
import type { ExplorerViewState } from './ExplorerPanel';
import ExtensionsView from './ExtensionsView';
import { changeMockExtension, mockExtensions } from './workbenchMock';
import { labels, readUiPreferences, saveUiPreferences, shortcutChord, type ShortcutAction, type UiPreferences, type UiView } from '../uiPreferences';
import { useWorkspace } from '../useWorkspace';

const ActivityDashboard = lazy(() => import('./ActivityDashboard'));

export interface WorkbenchController {
  state: Snapshot; sessions: SessionView[]; selectedSessionId: number | null;
  detail: SessionDetail | null; runs: RunView[]; loadingSession: boolean; pending: boolean; error: string | null;
  selectSession: (id: number | null) => void;
  createSession: () => unknown;
  start: (prompt: string) => Promise<number | undefined>;
  cancel: (id: number) => unknown;
  setMode: (mode: Mode) => unknown;
  decidePlan: (plan: Plan, decision: PlanDecision) => unknown;
}

const statusLabels: Record<RunStatus, string> = { running: '正在生成', completed: '已完成', cancelled: '已停止', failed: '失败' };

export default function Workbench({ controller: c, source = 'live', projects: suppliedProjects, onCreateProjectSession }: {
  controller: WorkbenchController; source?: 'live' | 'preview'; projects?: WorkbenchProject[];
  onCreateProjectSession?: (projectId: string) => void;
}) {
  const preview = source === 'preview';
  const [preferences, setPreferences] = useState(() => readUiPreferences(source));
  const updatePreferences = useCallback((value: UiPreferences) => { setPreferences(value); saveUiPreferences(source, value); }, [source]);
  const text = labels(preferences.language);
  const [prompt, setPrompt] = useState('');
  const [navigation, setNavigation] = useState<UiView>(() => preferences.startupView);
  const [selectedPlanId, setSelectedPlanId] = useState<number | null>(null);
  const [reviewOpen, setReviewOpen] = useState(true);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsVersion, setSettingsVersion] = useState(0);
  const [rightTab, setRightTab] = useState<'review' | 'explorer'>('review');
  const workspace = useWorkspace(!preview && reviewOpen && rightTab === 'explorer');
  const [explorerView, setExplorerView] = useState<ExplorerViewState>({ selected: null, closedFolders: [] });
  const [extensionItems, setExtensionItems] = useState(() => structuredClone(mockExtensions));
  const [toastError, setToastError] = useState<string | null>(null);
  const dismissToast = useCallback(() => setToastError(null), []);
  const [diffView, setDiffView] = useState<DiffViewState>({ file: null, split: true });
  const history = useRef<HTMLDivElement>(null);
  const historyScrollTop = useRef(0);
  const autoScroll = useRef(true);
  const lastSession = useRef(c.selectedSessionId);
  const promptBytes = new TextEncoder().encode(prompt.trim()).length;
  const plans = c.detail?.plans ?? [];
  const session = c.sessions.find((item) => item.id === c.selectedSessionId);
  const projects = suppliedProjects ?? [{ id: 'local', name: 'Maestro-Agent', sessions: c.sessions }];
  const currentProject = projects.find((project) => project.sessions.some((item) => item.id === c.selectedSessionId)) ?? projects[0];
  const dashboardData = useMemo(() => createMockDashboard(currentProject), [currentProject.id, currentProject.today?.changedLines, currentProject.today?.completedPlans]);
  const blocked = preview || !c.state.ready || c.pending || !!c.state.activeRun;

  useEffect(() => { if (c.error) setToastError(c.error); }, [c.error]);

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
    setPrompt('');
    if (await c.start(submitted) === undefined) setPrompt((current) => current || submitted);
  }

  function reviewPlan(id: number) { setSelectedPlanId(id); setRightTab('review'); setReviewOpen(true); }

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || settingsOpen || (event.target instanceof Element && event.target.closest('input,textarea,[contenteditable="true"],.monaco-editor'))) return;
      const chord = shortcutChord(event);
      const action = (Object.keys(preferences.shortcuts) as ShortcutAction[]).find((id) => preferences.shortcuts[id] === chord);
      if (!action) return;
      event.preventDefault();
      if (action === 'settings') setSettingsOpen(true);
      if (action === 'activity') setNavigation('activity');
      if (action === 'extensions') setNavigation('extensions');
      if (action === 'review') setReviewOpen((value) => !value);
      if (action === 'search') document.querySelector<HTMLInputElement>('input[aria-label="搜索会话"]')?.focus();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [preferences.shortcuts, settingsOpen]);

  return <Tooltip.Provider delayDuration={250}>
    <div className={`workbench ${preview ? 'is-preview' : ''} ${!reviewOpen ? 'review-hidden' : ''}`}>
      <WorkbenchLayout source={source} sidebar={
      <aside aria-label="会话列表" className="sessions-pane">
        <div className="brand"><span className="brand-mark"><Command size={19} strokeWidth={1.5} /></span><h1>Maestro-Agent</h1><span className="brand-version">beta</span></div>
        <DropdownMenu.Root>
          <DropdownMenu.Trigger className="project-switcher"><span className="project-icon"><FolderGit2 size={17} strokeWidth={1.5} /></span><span><strong>{currentProject.name}</strong><small>{preview ? 'Preview workspace' : 'Local workspace'}</small></span><ChevronDown size={13} /></DropdownMenu.Trigger>
          <DropdownMenu.Portal><DropdownMenu.Content sideOffset={6} className="dropdown-content">
            <DropdownMenu.Label>当前项目</DropdownMenu.Label>
            {projects.map((project) => <DropdownMenu.Item key={project.id} className="dropdown-item" disabled={project.sessions.length === 0}
              onSelect={() => { if (preview && project.sessions[0]) { setNavigation('agents'); c.selectSession(project.sessions[0].id); } }}><FolderGit2 size={14} />{project.name}{project.id === currentProject.id && <Check size={13} className="ml-auto" />}</DropdownMenu.Item>)}
            <DropdownMenu.Separator className="subtle-separator" />
            <DropdownMenu.Item disabled className="dropdown-item">其他工作区尚未接入</DropdownMenu.Item>
          </DropdownMenu.Content></DropdownMenu.Portal>
        </DropdownMenu.Root>
        <nav className="primary-navigation" aria-label="核心导航">
          <button className="navigation-item" aria-current={navigation === 'agents' ? 'page' : undefined} onClick={() => setNavigation('agents')}><Bot size={17} strokeWidth={1.5} /><span>{text.agents}</span><span className="navigation-count">{c.sessions.length}</span></button>
          <button className="navigation-item" aria-current={navigation === 'automations' ? 'page' : undefined} onClick={() => setNavigation('automations')}><Zap size={17} strokeWidth={1.5} /><span>{text.automations}</span><span className="soon-label">Soon</span></button>
          <button className="navigation-item" aria-current={navigation === 'extensions' ? 'page' : undefined} onClick={() => setNavigation('extensions')}><Puzzle size={17} strokeWidth={1.5} /><span>{text.extensions}</span><span className="soon-label">Mock</span></button>
        </nav>
        <button type="button" className="navigation-item activity-navigation" aria-label="Activity" aria-current={navigation === 'activity' ? 'page' : undefined} onClick={() => setNavigation('activity')}>
          <ChartNoAxesColumnIncreasing size={16} strokeWidth={1.5} /><span>Activity</span><span className="activity-navigation-summary">{dashboardData.todayLines} lines today</span>
        </button>
        <ProjectSessions projects={projects} currentProjectId={currentProject.id} source={source} selectedSessionId={c.selectedSessionId} activeSessionId={c.state.activeRun?.sessionId}
          canCreate={preview ? !!onCreateProjectSession : c.state.ready && !c.pending}
          onSelect={(id) => { setNavigation('agents'); c.selectSession(id); }}
          onCreate={(projectId) => { setNavigation('agents'); if (preview) onCreateProjectSession?.(projectId); else void c.createSession(); }} />
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
        </Suspense> : navigation === 'extensions' ? <ExtensionsView items={extensionItems} onAction={(id, action) => setExtensionItems((items) => changeMockExtension(items, id, action))} /> : navigation === 'automations' ? <div className="automation-page"><span className="welcome-icon"><Zap size={27} strokeWidth={1.25} /></span><h3>Your agents, on autopilot.</h3><p>定时执行重复性任务，让专注留给更重要的工作。</p><div className="automation-placeholder"><Clock3 size={19} /><div><strong>Scheduled agents</strong><p>自动任务尚未接入后端，本次重构仅展示入口。</p></div><span className="soon-label">Coming soon</span></div><button className="button-ghost" onClick={() => setNavigation('agents')}>返回 Agents<ArrowUpRight size={14} /></button></div>
          : <>
            <div ref={history} className="chat-history" aria-label="对话消息" onScroll={() => { const element = history.current; if (element) { historyScrollTop.current = element.scrollTop; autoScroll.current = element.scrollHeight - element.scrollTop - element.clientHeight < 96; } }}>
              <div className="chat-history-inner">
                {c.runs.length === 0 && <div className="conversation-welcome"><span className="welcome-icon"><WandSparkles size={27} strokeWidth={1.2} /></span><h3>What should we build?</h3><p>描述你的任务。Agent 会先规划，再提交可审查的变更。</p><div className="welcome-suggestions">重构模块<span>·</span>排查问题<span>·</span>审查代码</div></div>}
                {c.loadingSession && c.runs.length === 0 && <p className="loading-note">正在加载会话…</p>}
                {c.runs.map((run) => <section key={run.id} className="conversation-turn" aria-label={`对话轮次 ${run.id}`}>
                  <article className="user-message"><div className="message-meta"><span className="user-avatar">Y</span><strong>You</strong><span>{preview ? '11:42 AM' : `#${run.id}`}</span></div><p>{run.prompt}</p></article>
                  <article aria-label={`Agent 回复 ${run.id}`} className="agent-message"><div className="message-meta"><span className="agent-avatar"><Sparkles size={14} strokeWidth={1.5} /></span><strong>Maestro</strong><span className="message-model">Agent</span><span className="ml-auto">{statusLabels[run.status]}</span></div>
                    <div aria-label={`回复内容 ${run.id}`} aria-busy={run.status === 'running'} className="agent-copy">{run.output || (run.status === 'running' ? '思考中...' : '无输出')}{run.status === 'running' && <span aria-hidden="true" className="typing-cursor" />}</div>
                    {run.error && <p className="error-message">{run.error}</p>}
                    {plans.filter((plan) => plan.runId === run.id).map((plan) => <PlanCard key={plan.id} plan={plan} preview={preview} canDecide={!preview && run.status === 'completed'} pending={c.pending} onReview={reviewPlan} onDecision={(_id, decision) => { if (!preview) void c.decidePlan(plan, decision); }} />)}
                    <div className="message-footnote"><ShieldCheck size={12} />{preview ? '示例对话 · 不执行任何操作' : '模型回复 · 不执行工具或应用修改'}</div>
                  </article>
                </section>)}
              </div>
            </div>
            <div className="composer-area"><form onSubmit={(event) => void submit(event)} className="composer">
              <textarea aria-label="发送消息" rows={3} value={prompt} onChange={(event) => setPrompt(event.target.value)} placeholder="描述你的任务... Ctrl+Enter 发送"
                disabled={blocked} onKeyDown={(event) => { if ((event.ctrlKey || event.metaKey) && event.key === 'Enter' && !event.nativeEvent.isComposing) { event.preventDefault(); event.currentTarget.form?.requestSubmit(); } }} />
              <div className="composer-toolbar"><IconButton label="附件暂未接入" disabled><Paperclip size={17} /></IconButton>
                <Choice label="由 Rust 强制执行" value={c.state.mode} disabled={blocked} onChange={(value) => { if (!preview) void c.setMode(value as Mode); }} options={[{ value: 'plan', label: <span className="flex items-center gap-1.5"><ListChecks size={13} />Plan</span> }, { value: 'execute', label: <span className="flex items-center gap-1.5"><Code2 size={13} />Execute</span> }]} />
                <div className="flex-1" /><span className={`input-count ${promptBytes > 8192 ? 'invalid' : ''}`}>{promptBytes > 0 ? `${promptBytes} / 8192` : 'Ctrl ↵'}</span>
                <ComposerControls preview={preview} disabled={blocked} preferences={preferences} onPreferences={updatePreferences} settingsVersion={settingsVersion} onError={setToastError} />
                {c.state.activeRun?.status === 'running' && !preview ? <IconButton label="停止生成" className="stop-button" disabled={c.pending} onClick={() => void c.cancel(c.state.activeRun!.id)}><Square size={13} /></IconButton> : <button type="submit" aria-label="发送" className="send-button" disabled={blocked || promptBytes === 0 || promptBytes > 8192}><ArrowUp size={19} strokeWidth={1.8} /></button>}
              </div>
            </form><div className="composer-caption"><span><ShieldCheck size={11} />{preview ? '静态预览 · 发送与审批已禁用' : '先审查，再决定。批准计划不会自动应用修改。'}</span><span>Maestro Agent</span></div></div>
          </>}
      </main>
      } review={reviewOpen ? <RightPanel tab={rightTab} onTabChange={setRightTab} plans={plans} selectedId={selectedPlanId} onSelect={setSelectedPlanId} diffView={diffView} onDiffViewChange={setDiffView} explorerView={explorerView} onExplorerViewChange={setExplorerView} scope={`${currentProject.id}:${c.selectedSessionId}`} workspaceName={currentProject.name} fontSize={preferences.editorFontSize} fontFamily={preferences.editorFont} preview={preview} workspace={workspace} /> : null} />
    </div>
    <SettingsPanel open={settingsOpen} onOpenChange={setSettingsOpen} preview={preview} preferences={preferences} onPreferences={updatePreferences} onModelsChanged={() => setSettingsVersion((value) => value + 1)} />
    <ErrorToast message={toastError} onDismiss={dismissToast} />
  </Tooltip.Provider>;
}

