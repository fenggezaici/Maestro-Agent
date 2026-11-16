/// Dumb composer: prompt input, attachments, mode/project pickers and the
/// send/stop controls. Pure props in, callbacks out — no IPC, no stores.
import { useEffect, useRef, useState, type ChangeEvent, type Dispatch, type FormEvent, type RefObject, type SetStateAction } from 'react';
import * as Popover from '@radix-ui/react-popover';
import { ArrowDown, ArrowUp, Check, ChevronDown, Code2, FileEdit, FolderGit2, Grip, ListChecks, Paperclip, Search, ShieldCheck, Square, SquareTerminal, X, Zap } from 'lucide-react';
import type { PendingApprovalView } from '../lib/conversation';

import type { Mode, RunMetrics, RunView } from '../api/types';
import type { CommandView, SkillView } from '../api/extensions';
import type { UiPreferences } from '../uiPreferences';
import type { ProviderId } from '../modelCapabilities';
import type { WorkbenchProject } from './workspace';
import { Choice, IconButton } from './ui';
import LocalAttachments, { type LocalAttachment } from './LocalAttachments';
import ComposerControls from './ComposerControls';
import SessionStats, { sessionUsage } from './SessionStats';
import ContextRing from './ContextRing';
import SlashPanel from './SlashPanel';
import { buildSlashGroups, skillDirective, slashQuery, type SlashAction, type SlashItem } from '../slashCommands';

const AGENT_STATUS_LABEL: Record<'approval' | 'tool' | 'thinking' | 'working', string> = {
  approval: '等待批准', tool: '执行工具中', thinking: '思考中', working: '工作中',
};

function formatStatusElapsed(ms: number) {
  const seconds = Math.max(0, ms) / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)} 秒`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes} 分 ${(seconds - minutes * 60).toFixed(1)} 秒`;
}

export default function Composer({ atBottom, onScrollBottom, onSubmit, attachments, onAttachmentsChange, blocked, prompt, onPromptChange, promptBytes, focusRequest, preview, mode, onSetMode, currentProject, projects, onSelectProject, onSetProjectPath, onBrowse, onNewProject, preferences, onPreferences, settingsVersion, onError, onCapability, running, pending, onCancel, runs, metrics, context, modelLabel, status, steps = 0, slash }: {
  atBottom: boolean;
  onScrollBottom: () => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  attachments: LocalAttachment[];
  onAttachmentsChange: Dispatch<SetStateAction<LocalAttachment[]>>;
  blocked: boolean;
  prompt: string;
  onPromptChange: (value: string) => void;
  promptBytes: number;
  focusRequest?: number;
  preview: boolean;
  mode: Mode;
  onSetMode: (mode: Mode) => void;
  currentProject: WorkbenchProject;
  projects: WorkbenchProject[];
  onSelectProject?: (id: number) => void;
  onSetProjectPath?: (id: number, dir: string) => Promise<unknown>;
  onBrowse?: () => Promise<string | null>;
  onNewProject: () => void;
  preferences: UiPreferences;
  onPreferences: (value: UiPreferences) => void;
  settingsVersion: number;
  onError: (message: string) => void;
  onCapability: (model: string, provider: ProviderId | (string & {}), context: number | null) => void;
  running: boolean;
  pending: boolean;
  onCancel: () => void;
  runs: RunView[];
  metrics?: Record<number, RunMetrics>;
  context: number | null;
  modelLabel: string;
  status?: 'approval' | 'tool' | 'thinking' | 'working' | null;
  steps?: number;
  slash?: {
    actions: SlashAction[];
    skills: SkillView[];
    commands: CommandView[];
    onRefresh: () => void;
    onRunCommand: (name: string, args: string) => void;
  };
}) {
  const fileInput = useRef<HTMLInputElement>(null);
  const promptInput = useRef<HTMLTextAreaElement>(null);
  const attachmentUrls = useRef(new Set<string>());

  // Editing a past user message refills the prompt; focus and park the caret
  // at the end so the user can adjust and resend immediately.
  useEffect(() => {
    if (!focusRequest) return;
    const element = promptInput.current;
    if (!element) return;
    element.focus();
    element.setSelectionRange(element.value.length, element.value.length);
  }, [focusRequest]);

  function removeAttachment(id: string) {
    const removed = attachments.find((item) => item.id === id);
    if (removed) {
      URL.revokeObjectURL(removed.url);
      attachmentUrls.current.delete(removed.url);
    }
    onAttachmentsChange((current) => current.filter((item) => item.id !== id));
  }

  function selectFiles(event: ChangeEvent<HTMLInputElement>) {
    const files = Array.from(event.target.files ?? []);
    const added: LocalAttachment[] = [];
    for (const file of files.slice(0, Math.max(0, 6 - attachments.length))) {
      if (file.size > 20 * 1024 * 1024) { onError('附件超过 20 MiB 上限'); continue; }
      const url = URL.createObjectURL(file); attachmentUrls.current.add(url);
      const type: LocalAttachment['type'] = file.type.startsWith('image/') ? 'image' : file.type.startsWith('video/') ? 'video' : 'text';
      const entry: LocalAttachment = { id: crypto.randomUUID(), name: file.name, type, url };
      if (type === 'image' && file.size <= 3 * 1024 * 1024) void new Promise<string | undefined>((resolve) => {
        const reader = new FileReader(); reader.onload = () => resolve(typeof reader.result === 'string' ? reader.result : undefined); reader.onerror = () => resolve(undefined); reader.readAsDataURL(file);
      }).then((dataUrl) => { if (dataUrl) onAttachmentsChange((current) => current.map((item) => item.id === entry.id ? { ...item, dataUrl } : item)); });
      else if (type === 'text' && file.size <= 64 * 1024) void file.text().then((text) => onAttachmentsChange((current) => current.map((item) => item.id === entry.id ? { ...item, text } : item))).catch(() => undefined);
      added.push(entry);
    }
    onAttachmentsChange((current) => [...current, ...added]); event.target.value = '';
  }

  const currentRun = runs.at(-1);
  const statusReport = currentRun ? metrics?.[currentRun.id] : undefined;
  const [statusTick, setStatusTick] = useState(0);
  const [projectQuery, setProjectQuery] = useState('');
  const statusStarted = useRef<number | null>(null);
  useEffect(() => {
    if (!status) { statusStarted.current = null; setStatusTick(0); return; }
    statusStarted.current ??= performance.now();
    const timer = window.setInterval(() => setStatusTick(performance.now() - (statusStarted.current ?? performance.now())), 100);
    return () => window.clearInterval(timer);
  }, [status, currentRun?.id]);
  const statusElapsed = Math.max(
    statusReport?.elapsedMs ?? 0,
    currentRun?.createdAt ? Date.now() - currentRun.createdAt * 1000 : 0,
    statusTick,
  );

  const [slashDismissed, setSlashDismissed] = useState(false);
  const [slashIndex, setSlashIndex] = useState(0);
  const slashState = slash ? slashQuery(prompt, slashDismissed) : null;
  const slashGroups = slash && slashState ? buildSlashGroups(slash.actions, slash.skills, slash.commands, slashState.query) : [];
  const slashItems = slashGroups.flatMap((group) => group.items);
  useEffect(() => { setSlashIndex(0); }, [slashState?.query]);
  useEffect(() => {
    setSlashIndex((current) => Math.min(current, Math.max(0, slashItems.length - 1)));
  }, [slashItems.length]);

  function selectSlash(item: SlashItem) {
    const args = slashState?.args ?? '';
    if (item.kind === 'action') {
      item.action.run();
      onPromptChange('');
    } else if (item.kind === 'skill') {
      onPromptChange(skillDirective(item.name, args));
    } else {
      onPromptChange('');
      slash?.onRunCommand(item.name, args);
    }
    setSlashDismissed(true);
    promptInput.current?.focus();
  }

  return <div className="composer-area"><button type="button" className={`scroll-bottom-fab ${atBottom ? '' : 'visible'}`} aria-label="回到底部" onClick={onScrollBottom}><ArrowDown size={16} /></button><div className="composer-inner">{status && <span className="agent-status-pill" role="status" aria-live="polite"><Grip size={13} /><span>{AGENT_STATUS_LABEL[status]} {formatStatusElapsed(statusElapsed)}</span></span>}<form onSubmit={onSubmit} className="composer">
    {slashState && <SlashPanel groups={slashGroups} index={slashIndex} onHover={setSlashIndex} onSelect={selectSlash} />}
    <LocalAttachments items={attachments} onRemove={removeAttachment} />
    <input ref={fileInput} type="file" hidden multiple aria-label="选择本地媒体附件" accept="image/png,image/jpeg,image/webp,image/gif,video/mp4,video/webm,.md,.txt,.py,.ts,.tsx,.js,.jsx,.json,.rs,.toml,.yaml,.yml,.log,.sql,.sh" onChange={selectFiles} />
    <textarea ref={promptInput} aria-label="发送消息" rows={3} value={prompt} onChange={(event) => {
      const value = event.target.value;
      setSlashDismissed(false);
      onPromptChange(value);
      if (slash && value === '/') slash.onRefresh();
    }} placeholder="描述你的任务… 输入 / 打开命令；Enter 发送，Shift+Enter 换行"
      disabled={blocked} onKeyDown={(event) => {
        if (slashState && !event.nativeEvent.isComposing) {
          if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault();
            if (slashItems.length === 0) return;
            setSlashIndex((current) => event.key === 'ArrowDown' ? Math.min(current + 1, slashItems.length - 1) : Math.max(current - 1, 0));
            return;
          }
          if (event.key === 'Escape') { event.preventDefault(); setSlashDismissed(true); return; }
          if ((event.key === 'Enter' || event.key === 'Tab') && slashItems.length > 0) {
            event.preventDefault();
            const item = slashItems[slashIndex];
            if (item && !item.disabled) selectSlash(item);
            return;
          }
        }
        if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); event.currentTarget.form?.requestSubmit(); }
      }} />
    <div className="composer-toolbar"><IconButton label="添加图片或视频" disabled={!preview && (running || pending || blocked)} onClick={() => fileInput.current?.click()}><Paperclip size={17} /></IconButton>

      <Choice label="由 Rust 强制执行" value={mode} disabled={!preview && (blocked || running)} onChange={(value) => onSetMode(value as Mode)} options={[
        { value: 'plan', label: <span className="choice-item-stack"><span className="choice-item-main"><ListChecks size={13} />计划</span><span className="choice-item-desc">只读分析，不修改文件</span></span> },
        { value: 'code', label: <span className="choice-item-stack"><span className="choice-item-main"><Code2 size={13} />代码</span><span className="choice-item-desc">写文件直接执行；危险命令需批准</span></span> },
        { value: 'confirm', label: <span className="choice-item-stack"><span className="choice-item-main"><ShieldCheck size={13} />确认</span><span className="choice-item-desc">每个工具/命令都需确认，批准后执行</span></span> },
        { value: 'auto', label: <span className="choice-item-stack"><span className="choice-item-main"><Zap size={13} />自动</span><span className="choice-item-desc">已开放工具直接执行（含写入/命令）</span></span> },
      ]} />
      <Popover.Root onOpenChange={(open) => { if (!open) setProjectQuery(''); }}>
        <Popover.Trigger asChild><button type="button" className="composer-project-chip" disabled={!preview && (blocked || running)} aria-label="当前项目"><FolderGit2 size={13} />{currentProject.path ? currentProject.name : '不在项目中工作'}<ChevronDown size={11} /></button></Popover.Trigger>
        <Popover.Portal><Popover.Content className="model-picker project-picker" side="top" align="start" sideOffset={8}>
          <div className="project-picker-search"><Search size={13} /><input aria-label="搜索项目" placeholder="搜索项目或设备" value={projectQuery} onChange={(event) => setProjectQuery(event.target.value)} autoFocus /></div>
          <div className="model-picker-list" role="listbox" aria-label="项目列表">
            {projects.filter((project) => Boolean(project.path) && `${project.name} ${project.path}`.toLowerCase().includes(projectQuery.trim().toLowerCase())).map((project) => {
              const numeric = Number(project.id);
              const selected = currentProject.path ? currentProject.id === project.id : false;
              return <div key={project.id} className="model-picker-row project-picker-row" role="menuitemcheckbox" aria-checked={selected}
                onClick={() => { if (!preview && Number.isFinite(numeric)) { onSelectProject?.(numeric); } }}>
                <FolderGit2 size={14} /><span className="model-picker-id">{project.name}</span>
                <span className="model-picker-context project-picker-path" title={project.path ?? ''}>{project.path?.split(/[\/]/).pop() ?? ''}</span>
                {selected && <Check size={13} className="project-picker-check" />}
              </div>;
            })}
            {projects.filter((project) => Boolean(project.path) && `${project.name} ${project.path}`.toLowerCase().includes(projectQuery.trim().toLowerCase())).length === 0 && <p className="project-picker-empty">没有匹配的项目</p>}
          </div>
          <div className="project-picker-footer">
            <button type="button" className="project-picker-action" onClick={onNewProject}><span>＋ 创建项目</span></button>
            {(() => { const noProject = projects.find((project) => !project.path); const numeric = noProject ? Number(noProject.id) : NaN; if (!noProject || !Number.isFinite(numeric)) return null; return <button type="button" className="project-picker-action" role="menuitemcheckbox" aria-checked={!currentProject.path} onClick={() => onSelectProject?.(numeric)}><span>不在项目中工作</span>{!currentProject.path && <Check size={13} />}</button>; })()}
          </div>
        </Popover.Content></Popover.Portal>
      </Popover.Root>
      <div className="flex-1" /><ComposerControls preview={preview} disabled={blocked || running} preferences={preferences} onPreferences={onPreferences} settingsVersion={settingsVersion} onError={onError} onCapability={onCapability} />
      <ContextRing used={sessionUsage(runs, metrics, context).contextUsed} context={context} />
      <span className={`input-count ${promptBytes > 8192 ? 'invalid' : ''}`}>{promptBytes > 0 ? `${promptBytes} / 8192` : 'Enter ↵'}</span>
      {running && !preview ? <IconButton label="停止生成" className="stop-button" onClick={onCancel}><Square size={13} /></IconButton> : <button type="submit" aria-label="发送" className="send-button" disabled={blocked || promptBytes === 0 || promptBytes > 8192}><ArrowUp size={19} strokeWidth={1.8} /></button>}
    </div>
  </form><SessionStats runs={runs} metrics={metrics} context={context} steps={steps} /><div className="composer-caption"><span><ShieldCheck size={11} />{preview ? '预览不调用模型；媒体仅本地显示' : '图片/文本附件随消息发送（图片需视觉模型）；视频仅本地预览。'}</span><span>{modelLabel || 'Maestro Agent'}</span></div></div></div>;
}
