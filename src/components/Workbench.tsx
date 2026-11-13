import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from 'react';
import * as DropdownMenu from '@radix-ui/react-dropdown-menu';
import * as Popover from '@radix-ui/react-popover';
import * as Tooltip from '@radix-ui/react-tooltip';
import { Archive, ArchiveRestore, ArrowDown, ArrowUp, ArrowUpRight, Bot, BrainCircuit, ChartNoAxesColumnIncreasing, Check, ChevronDown, Clock3, Code2, Copy, FolderGit2, GitBranch, HelpCircle, ListChecks, ListTodo, Monitor, Moon, PanelRightClose, PanelRightOpen, Paperclip, Plus, Puzzle, Settings2, ShieldCheck, Sparkles, Square, Sun, WandSparkles, Zap } from 'lucide-react';
import type { Mode, PlanCard as Plan, PlanDecision, PlanStep, ProjectView, RunView, SessionDetail, SessionView, Snapshot, RunMetrics } from '../api/types';
import CollapsibleBlock from './CollapsibleBlock';
import MaestroLogo from './MaestroLogo';
import TaskPlanPanel from './TaskPlanPanel';
import { preloadMonaco } from '../editor/loader';
import PlanCard from './PlanCard';
import ToolCallBlocks from './ToolCallBlocks';
import type { ActiveFile, ListTab } from './RightPanel';
import { Choice, IconButton } from './ui';
import WorkbenchLayout from './WorkbenchLayout';
import ProjectSessions from './ProjectSessions';
import { createEmptyDashboard, createMockDashboard, type DashboardData } from './activityMock';
import { activitySnapshotApi } from '../api/activity';
import { previewRevertRunChangesApi, revertRunChangesApi, runChangeSummaryApi, runChangesApi, type RevertPreview, type RunChangeSummary, type RunFileChange } from '../api/changes';
import ChangeSummaryCard from './ChangeSummaryCard';
import RevertChangesDialog from './RevertChangesDialog';
import { lineDiff } from '../lib/lineDiff';
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
import { useStickyScroll } from '../hooks/useStickyScroll';
import { I18nProvider } from '../i18n';
import CommandCenter, { type PaletteCommand } from './CommandCenter';
import { flattenFileTree } from '../commandCenter';
import { readPinnedSessions, savePinnedSessions } from '../sessionGroups';
import { mockExplorerFiles } from './workbenchMock';
import { pickDirectory } from '../api/dialog';
import { agentStatus, pendingApproval, reviewFiles, type TimelineItem } from '../lib/conversation';
import { useAppearance } from '../useAppearance';
import LocalAttachments, { type LocalAttachment } from './LocalAttachments';
import SessionStats from './SessionStats';
import ChatStream from './ChatStream';
import Composer from './Composer';
import { resolveCapability } from '../modelCatalog';
import type { ProviderId } from '../modelCapabilities';

const ActivityDashboard = lazy(() => import('./ActivityDashboard'));
const EMPTY_PLAN: PlanStep[] = [];
const MODE_LABEL: Record<Mode, string> = { plan: 'Plan 模式', code: 'Code 模式', confirm: 'Confirm 模式', auto: 'Auto 模式' };

export interface WorkbenchController {
  state: Snapshot; sessions: SessionView[]; projects?: ProjectView[]; selectedSessionId: number | null;
  liveTools?: Record<number, import('../api/types').ToolCallView[]>;
  liveTimeline?: Record<number, TimelineItem[]>;
  stoppingRunId?: number | null;
  /** Task-tracker steps for the selected session (per-session in the store). */
  currentPlan?: PlanStep[];
  /** Presentation override for the preview build; live derives it from the run. */
  planRunning?: boolean;
  /** Latest applied write path for live diff following. */
  lastChangedPath?: string | null;
  /** Bumped on workspace_changed; refreshes live change summaries. */
  workspaceTick?: number;
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
  queueStart?: (prompt: string, images?: string[]) => void;
  resolveApproval?: (runId: number, approved: boolean) => unknown;
  setMode: (mode: Mode) => unknown;
  decidePlan: (plan: Plan, decision: PlanDecision) => unknown;
}

function fallbackChanges(files: { path: string; before: string; after: string }[]): RunFileChange[] {
  return files.map((file) => {
    const lines = lineDiff(file.before, file.after);
    return {
      path: file.path,
      added: lines.filter((line) => line.kind === 'added').length,
      removed: lines.filter((line) => line.kind === 'removed').length,
      before: file.before,
      after: file.after,
      createdAt: 0,
    };
  });
}

function mergeRunChanges(changes: RunFileChange[]): RunFileChange[] {
  const order: string[] = [];
  const merged = new Map<string, RunFileChange>();
  for (const change of changes) {
    const existing = merged.get(change.path);
    if (existing) {
      existing.added += change.added;
      existing.removed += change.removed;
      existing.after = change.after;
    } else {
      order.push(change.path);
      merged.set(change.path, { ...change });
    }
  }
  return order.map((path) => merged.get(path)!);
}

function summarizeChanges(changes: RunFileChange[]): RunChangeSummary {
  return {
    files: changes.length,
    additions: changes.reduce((total, change) => total + change.added, 0),
    deletions: changes.reduce((total, change) => total + change.removed, 0),
    state: 'applied',
  };
}

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
  const [promptFocus, setPromptFocus] = useState(0);
  const [navigation, setNavigation] = useState<UiView>(() => preferences.startupView);
  const [selectedPlanId, setSelectedPlanId] = useState<number | null>(null);
  const [reviewOpen, setReviewOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsVersion, setSettingsVersion] = useState(0);
  const [newProjectOpen, setNewProjectOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<WorkbenchProject | null>(null);
  const [listTab, setListTab] = useState<ListTab>('changes');
  const [openTabs, setOpenTabs] = useState<ListTab[]>(['explorer', 'changes']);
  const tasksDismissed = useRef(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [pinnedIds, setPinnedIds] = useState<number[]>(() => readPinnedSessions(source));
  const workspace = useWorkspace(!preview && ((reviewOpen && listTab === 'explorer' && openTabs.includes('explorer')) || paletteOpen));
  const [explorerView, setExplorerView] = useState<ExplorerViewState>({ selected: null, closedFolders: [] });
  const liveExtensions = useExtensions(!preview && navigation === 'extensions');
  const extensions = suppliedExtensions ?? liveExtensions;
  const [toastError, setToastError] = useState<string | null>(null);
  const dismissToast = useCallback(() => setToastError(null), []);
  const [activeFile, setActiveFile] = useState<ActiveFile | null>(null);
  const [runSummary, setRunSummary] = useState<RunChangeSummary | null>(null);
  const [runChanges, setRunChanges] = useState<RunFileChange[]>([]);
  const [runChangesLoaded, setRunChangesLoaded] = useState(false);
  const [runChangesLoading, setRunChangesLoading] = useState(false);
  const [runChangesError, setRunChangesError] = useState<string | null>(null);
  const [revertOpen, setRevertOpen] = useState(false);
  const [revertPreview, setRevertPreview] = useState<RevertPreview | null>(null);
  const [revertLoading, setRevertLoading] = useState(false);
  const [reverting, setReverting] = useState(false);
  const [revertError, setRevertError] = useState<string | null>(null);
  const autoFollowChanges = useRef(true);
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
  const [atBottom, setAtBottom] = useState(true);
  const scroll = useStickyScroll(history, {
    onNearBottom: (near) => setAtBottom((current) => (current === near ? current : near)),
  });
  const [pinnedProjects, setPinnedProjects] = useState<string[]>(() => { try { return JSON.parse(localStorage.getItem('maestro.ui.pinnedProjects') ?? '[]'); } catch { return []; } });
  useEffect(() => { try { localStorage.setItem('maestro.ui.pinnedProjects', JSON.stringify(pinnedProjects)); } catch { /* quota */ } }, [pinnedProjects]);
  const lastSession = useRef(c.selectedSessionId);
  const promptBytes = new TextEncoder().encode(prompt.trim()).length;
  const plans = c.detail?.plans ?? [];
  const planSteps = c.currentPlan ?? EMPTY_PLAN;
  const planCompleted = planSteps.filter((step) => step.status === 'completed').length;
  const lastChangedPath = c.lastChangedPath ?? null;
  const stoppingActiveRun = c.stoppingRunId != null;
  const planRunning = !stoppingActiveRun && (c.planRunning ?? (c.state.activeRun?.status === 'running' && c.state.activeRun.sessionId === c.selectedSessionId));
  const approval = useMemo(() => pendingApproval(c.liveTools), [c.liveTools]);
  const reviewFileList = useMemo(() => reviewFiles({ liveTools: c.liveTools, pending: approval, plans }), [c.liveTools, approval, plans]);
  const stepCount = useMemo(() => Math.max(c.detail?.tools?.length ?? 0, Object.values(c.liveTools ?? {}).reduce((total, calls) => total + calls.length, 0)), [c.detail?.tools, c.liveTools]);
  const panelReviewFiles = useMemo(() => reviewFileList.length ? reviewFileList : runChanges.map((change) => ({ path: change.path, before: change.before, after: change.after, status: 'applied' as const, tool: 'write_file' })), [reviewFileList, runChanges]);
  const status = agentStatus({ running: !stoppingActiveRun && (c.state.activeRun?.status === 'running' || (c.runs ?? []).some((run) => run.status === 'running')), liveCalls: c.selectedSessionId != null ? c.liveTools?.[c.selectedSessionId] : undefined, reasoning: Object.values(c.liveTimeline ?? {}).some((items) => items.some((item) => item.kind === 'reasoning' && item.segment.endedAt === undefined)) ? 'x' : '', hasApproval: Boolean(approval) });
  // Surface suspended tool calls in the Review pane the moment they appear —
  // the diff/command preview is what the user actually approves.
  useEffect(() => {
    if (approval) {
      setListTab('changes');
      // Pre-select the pending file without opening the panel — the panel
      // only slides in on explicit user intent (change/stat clicks).
      if (approval.preview?.kind === 'write') setActiveFile({ kind: 'change', path: approval.preview.path });
    }
  }, [approval]);
  // The Tasks list is only meaningful while the selected session has a plan.
  useEffect(() => {
    if (planSteps.length === 0 || tasksDismissed.current) return;
    if (openTabs.length === 0) setListTab('tasks');
    setOpenTabs((current) => current.includes('tasks') ? current : [...current, 'tasks']);
  }, [planSteps.length, openTabs.length]);
  const session = c.sessions.find((item) => item.id === c.selectedSessionId);
  const projects: WorkbenchProject[] = suppliedProjects ?? (c.projects ?? []).filter((project) => !project.archived).map((project) => ({
    id: String(project.id), name: project.name, path: project.path,
    sessions: c.sessions.filter((item) => item.projectId === project.id && !item.archived),
    archivedSessions: c.sessions.filter((item) => item.projectId === project.id && item.archived),
  }));
  const archivedProjects = (suppliedProjects ?? []).filter((project) => project.archived).map((project) => ({ key: project.id, numeric: null as number | null, name: project.name }))
    .concat(suppliedProjects ? [] : (c.projects ?? []).filter((project) => project.archived).map((project) => ({ key: String(project.id), numeric: project.id, name: project.name })));
  const currentProject = projects.find((project) => project.sessions.some((item) => item.id === c.selectedSessionId)) ?? projects[0] ?? { id: 'local', name: 'Maestro-Agent', sessions: [] };
  const [liveDashboard, setLiveDashboard] = useState<DashboardData | null>(null);
  const [activityError, setActivityError] = useState<string | null>(null);
  useEffect(() => {
    if (preview || navigation !== 'activity') return;
    let cancelled = false;
    const numeric = Number(currentProject.id);
    setActivityError(null);
    void activitySnapshotApi(Number.isFinite(numeric) ? numeric : undefined)
      .then((snapshot) => { if (!cancelled) setLiveDashboard(snapshot); })
      .catch((reason) => { if (!cancelled) setActivityError(reason instanceof Error ? reason.message : String(reason)); });
    return () => { cancelled = true; };
  }, [preview, navigation, currentProject.id]);
  const summaryRun = c.state.activeRun ?? c.state.lastRun ?? c.runs.at(-1);
  const summaryRunId = summaryRun?.id ?? null;
  const panelReviewFilesRef = useRef(panelReviewFiles);
  useEffect(() => { panelReviewFilesRef.current = panelReviewFiles; }, [panelReviewFiles]);
  const summaryRunRef = useRef<number | null>(null);
  useEffect(() => {
    if (summaryRunRef.current === summaryRunId) return;
    summaryRunRef.current = summaryRunId;
    setRunSummary(null); setRunChanges([]); setRunChangesLoaded(false); setRunChangesError(null);
  }, [summaryRunId, c.selectedSessionId]);
  useEffect(() => {
    if (preview) return;
    if (!summaryRun || summaryRun.sessionId !== c.selectedSessionId) return;
    let cancelled = false;
    const terminal = summaryRun.status !== 'running';
    const fallback = () => {
      if (cancelled || !terminal) return;
      const rows = fallbackChanges(panelReviewFilesRef.current);
      if (rows.length) {
        setRunSummary(summarizeChanges(rows));
        setRunChanges(rows);
        setRunChangesLoaded(true);
        setRunChangesError('后端未返回变更记录，显示当前会话内存中的文件列表；撤销不可用。');
      }
    };
    void runChangeSummaryApi(summaryRun.id)
      .then((summary) => {
        if (cancelled) return;
        if (summary.files > 0) {
          setRunSummary(summary);
          setRunChangesLoaded(false);
        } else if (terminal) fallback();
      })
      .catch(() => { if (terminal) fallback(); });
    return () => { cancelled = true; };
  }, [preview, summaryRun?.id, summaryRun?.status, summaryRun?.sessionId, c.selectedSessionId, c.lastChangedPath, c.workspaceTick ?? 0]);

  const loadRunChanges = () => {
    if (!summaryRun || runChangesLoaded || runChangesLoading) return;
    setRunChangesLoading(true); setRunChangesError(null);
    void runChangesApi(summaryRun.id)
      .then((rows) => { setRunChanges(mergeRunChanges(rows)); setRunChangesLoaded(true); })
      .catch(() => {
        const rows = fallbackChanges(panelReviewFilesRef.current);
        setRunChanges(rows); setRunChangesLoaded(true);
        setRunChangesError('读取变更明细失败，显示当前会话内存中的文件列表；撤销不可用。');
      })
      .finally(() => setRunChangesLoading(false));
  };

  const previewRevert = () => {
    if (!summaryRun) return;
    setRevertOpen(true); setRevertLoading(true); setRevertError(null); setRevertPreview(null);
    void previewRevertRunChangesApi(summaryRun.id)
      .then(setRevertPreview)
      .catch((reason) => setRevertError(reason instanceof Error ? reason.message : String(reason)))
      .finally(() => setRevertLoading(false));
  };

  const applyRevert = () => {
    if (!summaryRun || c.selectedSessionId == null) return;
    setReverting(true); setRevertError(null);
    void revertRunChangesApi(summaryRun.id, c.selectedSessionId)
      .then((result) => {
        setRunSummary((current) => current ? { ...current, state: 'reverted' } : current);
        setRunChangesLoaded(false);
        if (result.conflicts.length) setToastError(`以下文件未撤销：${result.conflicts.join('、')}`);
        setRevertOpen(false); setRevertPreview(null);
        void workspace.refresh();
      })
      .catch((reason) => setRevertError(reason instanceof Error ? reason.message : String(reason)))
      .finally(() => setReverting(false));
  };

  const dashboardData = useMemo(() => preview ? createMockDashboard(currentProject) : liveDashboard ?? createEmptyDashboard(), [preview, currentProject, liveDashboard]);
  const running = !stoppingActiveRun && c.state.activeRun?.status === 'running';
  const blocked = preview || !c.state.ready || c.pending;

  const addTab = (tab: ListTab) => {
    setOpenTabs((current) => current.includes(tab) ? current : [...current, tab]);
    setListTab(tab);
  };
  const openTasks = () => {
    tasksDismissed.current = false;
    addTab('tasks');
    setReviewOpen(true);
  };
  const closeTab = (tab: ListTab) => {
    const index = openTabs.indexOf(tab);
    const next = openTabs.filter((item) => item !== tab);
    setOpenTabs(next);
    if (listTab === tab) {
      const neighbor = next[index] ?? next[index - 1];
      if (neighbor) setListTab(neighbor);
    }
    if (tab === 'tasks') tasksDismissed.current = true;
  };

  useEffect(() => { if (c.error) setToastError(c.error); }, [c.error]);
  useEffect(() => { autoFollowChanges.current = true; }, [c.state.activeRun?.id]);
  useEffect(() => {
    if (!reviewOpen || listTab !== 'changes' || !autoFollowChanges.current || !lastChangedPath) return;
    setActiveFile({ kind: 'change', path: lastChangedPath });
  }, [lastChangedPath, reviewOpen, listTab]);
  useEffect(() => { const timer = window.setTimeout(preloadMonaco, 1200); return () => window.clearTimeout(timer); }, []);

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
      scroll.reset();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- reset only when the conversation changes.
  }, [c.selectedSessionId]);

  useEffect(() => {
    if (navigation === 'agents') scroll.reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- returning to the chat should follow new output.
  }, [navigation]);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (blocked || running || promptBytes === 0 || promptBytes > 8192) {
      if (running) setToastError('当前任务运行中，请先停止或等待完成');
      return;
    }
    scroll.reset();
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
    if (c.stoppingRunId != null) {
      if (images.length) { setToastError('等待上一轮停止后再发送图片'); return; }
      c.queueStart?.(submitted + texts);
      setPrompt(''); setAttachments([]);
      setToastError('已排队，等待上一轮停止…');
      return;
    }
    setPrompt('');
    const runId = await c.start(submitted + texts, images.length ? images : undefined);
    if (runId === undefined) setPrompt((current) => current || submitted);
    else { setMessageAttachments((current) => ({ ...current, [runId]: sentAttachments })); setAttachments([]); }
  }

function reviewPlan(id: number) { setSelectedPlanId(id); setListTab('changes'); setReviewOpen(true); }
  function selectReviewFile(path: string) { autoFollowChanges.current = false; setListTab('changes'); setReviewOpen(true); setActiveFile({ kind: 'change', path }); }
  function openWorkspaceFile(rawPath: string) {
    const path = rawPath.replace(/\\/g, '/').replace(/^\/+/, '');
    if (!path) return;
    setReviewOpen(true);
    setListTab('explorer');
    setActiveFile({ kind: 'workspace', path });
    if (!preview) void workspace.selectFile(path);
  }

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

  return <I18nProvider language={preferences.language}><Tooltip.Provider delayDuration={250}>
    <div className={`workbench ${preview ? 'is-preview' : ''} ${!reviewOpen ? 'review-hidden' : ''}`} style={{ zoom: `${Number(preferences.fontScale) / 100}` }}>
      <WorkbenchLayout source={source} sidebar={
      <aside aria-label="会话列表" className="sessions-pane">
        <div className="brand"><MaestroLogo size={24} /><h1>Maestro-Agent</h1><span className="brand-version">beta</span></div>
        {archivedProjects.length > 0 && <div className="archived-projects" aria-label="已归档项目">
          <p className="session-group-label">已归档项目</p>
          {archivedProjects.map((project) => <button key={project.key} type="button" className="archived-project-row" disabled={!c.setProjectArchived && !sessionActions?.archiveProject}
            onClick={() => { if (preview) sessionActions?.archiveProject?.(project.key, false); else if (project.numeric !== null) void c.setProjectArchived?.(project.numeric, false); }}>
            <Archive size={12} /><span className="archived-project-name">{project.name}</span><ArchiveRestore size={11} /></button>)}
        </div>}
        <nav className="primary-navigation" aria-label="核心导航">
          <button className="navigation-item" aria-current={navigation === 'agents' ? 'page' : undefined} onClick={() => setNavigation('agents')}><Bot size={17} strokeWidth={1.5} /><span>{text.agents}</span><span className="navigation-count">{c.sessions.length}</span></button>
          <button className="navigation-item" aria-current={navigation === 'automations' ? 'page' : undefined} onClick={() => setNavigation('automations')}><Zap size={17} strokeWidth={1.5} /><span>{text.automations}</span><span className="soon-label">待接入</span></button>
          <button className="navigation-item" aria-current={navigation === 'extensions' ? 'page' : undefined} onClick={() => setNavigation('extensions')}><Puzzle size={17} strokeWidth={1.5} /><span>{text.extensions}</span><span className="soon-label">模拟</span></button>
        </nav>
        <button type="button" className="navigation-item activity-navigation" aria-label="活动" aria-current={navigation === 'activity' ? 'page' : undefined} onClick={() => setNavigation('activity')}>
          <ChartNoAxesColumnIncreasing size={16} strokeWidth={1.5} /><span>活动</span><span className="activity-navigation-summary">今日 {dashboardData.todayLines} 行</span>
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
          <div className="connection-status"><span className={`status-dot ${c.state.ready && !preview ? 'connected' : ''}`} /><span role="status">{preview ? '前端预览' : c.state.ready ? 'Rust 内核已就绪' : '正在连接 Rust 内核…'}</span><span className="local-tag">{preview ? '模拟' : '本地'}</span></div>
          <span className="build-stamp" title={__BUILD_TIME__}>构建 {new Date(__BUILD_TIME__).toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })}</span>
          <button type="button" className="navigation-item sidebar-settings-button" onClick={() => setSettingsOpen(true)}><Settings2 size={16} strokeWidth={1.5} /><span>{text.settings}</span></button>
        </div>
      </aside>
      } conversation={
      <main aria-label={navigation === 'activity' ? 'Activity 工作区' : 'Agent 对话'} className="conversation-pane">
        <header className="conversation-header">
          <div className="conversation-title">{navigation !== 'agents' && <button type="button" className="back-to-chat" onClick={() => setNavigation('agents')}>← {text.backToChat}</button>}<span className="eyebrow">{navigation === 'agents' ? text.agentSession : text.workspace}</span><h2>{navigation === 'agents' ? session?.title ?? text.newAgent : navigation === 'activity' ? text.workspaceOverview : navigation === 'extensions' ? text.extensions : text.automations}</h2></div>
          <div className="header-actions"><IconButton label="切换右侧面板" onClick={() => setReviewOpen(!reviewOpen)}>{reviewOpen ? <PanelRightClose size={16} /> : <PanelRightOpen size={16} />}</IconButton>
            <IconButton label="打开设置" onClick={() => setSettingsOpen(true)}><Settings2 size={17} /></IconButton></div>
        </header>
        {navigation === 'activity'
          ? <div className="session-context-strip"><span><ChartNoAxesColumnIncreasing size={12} />{preview ? 'Mock analytics' : '本地真实聚合'}</span><span>{dashboardData.start} — {dashboardData.end}</span></div>
          : <div className="session-context-strip"><span><GitBranch size={12} />{preview ? 'refactor/session-lifecycle' : 'local / planning'}</span><span><ShieldCheck size={12} />{MODE_LABEL[c.state.mode]}</span>{planSteps.length > 0 && <button type="button" className="task-progress-chip" onClick={openTasks}><ListTodo size={12} />任务 {planCompleted}/{planSteps.length}</button>}{preview && <span className="preview-tag">示例数据 · 无 Rust IPC</span>}</div>}
        {c.error && <div className="error-message">{c.error}</div>}
        {navigation === 'agents' && planSteps.length > 0 && <TaskPlanPanel steps={planSteps} running={planRunning} variant="card" />}
        {navigation === 'activity' ? <Suspense fallback={<div role="status" className="dashboard-loading">Loading dashboard…</div>}>
          <ActivityDashboard data={dashboardData} projectName={currentProject.name} onBack={() => setNavigation('agents')} loading={!preview && !liveDashboard && !activityError} error={activityError} />
        </Suspense> : navigation === 'extensions' ? <ExtensionsView controller={extensions} /> : navigation === 'automations' ? <div className="automation-page"><span className="welcome-icon"><Zap size={27} strokeWidth={1.25} /></span><h3>Your agents, on autopilot.</h3><p>定时执行重复性任务，让专注留给更重要的工作。</p><div className="automation-placeholder"><Clock3 size={19} /><div><strong>Scheduled agents</strong><p>自动任务尚未接入后端，本次重构仅展示入口。</p></div><span className="soon-label">Coming soon</span></div><button className="button-ghost" onClick={() => setNavigation('agents')}>返回 Agents<ArrowUpRight size={14} /></button></div>
          : <>
            <ChatStream historyRef={history} runs={c.runs} loadingSession={c.loadingSession}
              detailTools={c.detail?.tools} liveTools={c.liveTools} liveTimeline={c.liveTimeline}
              plans={plans} reasoningDisplay={preferences.reasoningDisplay} theme={theme} preview={preview}
              messageAttachments={messageAttachments} pending={c.pending}
              onCopyError={setToastError} onReviewPlan={reviewPlan} onDecidePlan={(plan, decision) => void c.decidePlan(plan, decision)}
              onSelectFile={selectReviewFile}
              onOpenFile={openWorkspaceFile}
              onEditPrompt={(text) => { setPrompt(text); setPromptFocus((value) => value + 1); }}
              onJumpToTurn={(runId) => {
                const root = history.current;
                const turn = root?.querySelector<HTMLElement>(`[data-turn-id="${runId}"]`);
                if (!root || !turn) return;
                const top = root.scrollTop + turn.getBoundingClientRect().top - root.getBoundingClientRect().top - 12;
                scroll.scrollToTop(top);
              }}
              onResolveApproval={preview || !c.resolveApproval ? undefined : (runId, approved) => void c.resolveApproval!(runId, approved)}
              onScroll={scroll.onScroll} />
            {runSummary && runSummary.files > 0 && <ChangeSummaryCard summary={runSummary} changes={runChanges}
              loading={runChangesLoading} error={runChangesError} running={running} onExpand={loadRunChanges}
              onReview={(path) => { setListTab('changes'); setReviewOpen(true); setActiveFile({ kind: 'change', path }); }}
              onOpen={(path) => { setActiveFile({ kind: 'workspace', path }); if (!preview) void workspace.selectFile(path); }}
              onRevert={previewRevert} />}
            <Composer atBottom={atBottom} onScrollBottom={scroll.stickToBottom}
              onSubmit={(event) => void submit(event)} attachments={attachments} onAttachmentsChange={setAttachments}
              blocked={blocked} prompt={prompt} onPromptChange={setPrompt} promptBytes={promptBytes} focusRequest={promptFocus}
              preview={preview} mode={c.state.mode} onSetMode={(mode) => void c.setMode(mode)}
              status={status} steps={stepCount}
              currentProject={currentProject} projects={projects}
              onSelectProject={(id) => c.selectProject?.(id)}
              onSetProjectPath={c.updateProjectPath ? (id, dir) => c.updateProjectPath!(id, dir) : undefined}
              onBrowse={preview ? undefined : pickDirectory}
              onNewProject={() => setNewProjectOpen(true)}
              preferences={preferences} onPreferences={updatePreferences} settingsVersion={settingsVersion}
              onError={setToastError} onCapability={updateCapability}
              running={running} pending={c.pending}
              onCancel={() => { const run = c.state.activeRun; if (run) void c.cancel(run.id); }}
              runs={c.runs} metrics={c.metrics} context={capability.context} modelLabel={capability.model} />
          </>}
      </main>
      } review={reviewOpen ? <RightPanel listTab={listTab} onListTabChange={setListTab} openTabs={openTabs} onAddTab={addTab} onCloseTab={closeTab} activeFile={activeFile} onActiveFile={setActiveFile} reviewFiles={panelReviewFiles} plans={plans} planSteps={planSteps} planRunning={planRunning} explorerView={explorerView} onExplorerViewChange={setExplorerView} scope={`${currentProject.id}:${c.selectedSessionId}`} fontSize={preferences.editorFontSize} fontFamily={preferences.editorFont} preview={preview} workspace={workspace} /> : null} />
    </div>
    <SettingsPanel open={settingsOpen} onOpenChange={setSettingsOpen} preview={preview} preferences={preferences} onPreferences={updatePreferences} onModelsChanged={() => setSettingsVersion((value) => value + 1)} />
    <CommandCenter open={paletteOpen} onOpenChange={setPaletteOpen} source={source} projects={projects} commands={paletteCommands}
      files={paletteFiles} filesLoading={!preview && paletteOpen && workspace.loading}
      onSelectSession={(id) => { setNavigation('agents'); c.selectSession(id); }}
      onSelectFile={(path) => {
        setReviewOpen(true); setListTab('explorer'); setActiveFile({ kind: 'workspace', path });
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
      <RevertChangesDialog open={revertOpen} onOpenChange={setRevertOpen} preview={revertPreview}
        loading={revertLoading} applying={reverting} error={revertError} onApply={applyRevert} />
  </Tooltip.Provider></I18nProvider>;
}

