/// Dumb composer: prompt input, attachments, mode/project pickers and the
/// send/stop controls. Pure props in, callbacks out — no IPC, no stores.
import { useRef, type ChangeEvent, type FormEvent, type RefObject } from 'react';
import * as Popover from '@radix-ui/react-popover';
import { ArrowDown, ArrowUp, ChevronDown, Code2, FolderGit2, ListChecks, Paperclip, ShieldCheck, Square, Zap } from 'lucide-react';

import type { Mode, RunMetrics, RunView } from '../api/types';
import type { UiPreferences } from '../uiPreferences';
import type { ProviderId } from '../modelCapabilities';
import type { WorkbenchProject } from './workspace';
import { Choice, IconButton } from './ui';
import LocalAttachments, { type LocalAttachment } from './LocalAttachments';
import ComposerControls from './ComposerControls';
import SessionStats from './SessionStats';

export default function Composer({ atBottom, onScrollBottom, onSubmit, attachments, onAttachmentsChange, blocked, prompt, onPromptChange, promptBytes, preview, mode, onSetMode, currentProject, projects, onSelectProject, onSetProjectPath, onBrowse, onNewProject, preferences, onPreferences, settingsVersion, onError, onCapability, running, pending, onCancel, runs, metrics, context, modelLabel }: {
  atBottom: boolean;
  onScrollBottom: () => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  attachments: LocalAttachment[];
  onAttachmentsChange: (items: LocalAttachment[]) => void;
  blocked: boolean;
  prompt: string;
  onPromptChange: (value: string) => void;
  promptBytes: number;
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
}) {
  const fileInput = useRef<HTMLInputElement>(null);
  const attachmentUrls = useRef(new Set<string>());

  function removeAttachment(id: string) {
    onAttachmentsChange(attachments.filter((item) => {
      if (item.id === id) { URL.revokeObjectURL(item.url); attachmentUrls.current.delete(item.url); return false; }
      return true;
    }));
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
      }).then((dataUrl) => { if (dataUrl) onAttachmentsChange(attachments.map((item) => item.id === entry.id ? { ...item, dataUrl } : item)); });
      else if (type === 'text' && file.size <= 64 * 1024) void file.text().then((text) => onAttachmentsChange(attachments.map((item) => item.id === entry.id ? { ...item, text } : item))).catch(() => undefined);
      added.push(entry);
    }
    onAttachmentsChange([...attachments, ...added]); event.target.value = '';
  }

  return <div className="composer-area"><button type="button" className={`scroll-bottom-fab ${atBottom ? '' : 'visible'}`} aria-label="回到底部" onClick={onScrollBottom}><ArrowDown size={16} /></button><form onSubmit={onSubmit} className="composer">
    <LocalAttachments items={attachments} onRemove={removeAttachment} />
    <input ref={fileInput} type="file" hidden multiple aria-label="选择本地媒体附件" accept="image/png,image/jpeg,image/webp,image/gif,video/mp4,video/webm,.md,.txt,.py,.ts,.tsx,.js,.jsx,.json,.rs,.toml,.yaml,.yml,.log,.sql,.sh" onChange={selectFiles} />
    <textarea aria-label="发送消息" rows={3} value={prompt} onChange={(event) => onPromptChange(event.target.value)} placeholder="描述你的任务... Ctrl+Enter 发送"
      disabled={blocked} onKeyDown={(event) => { if ((event.ctrlKey || event.metaKey) && event.key === 'Enter' && !event.nativeEvent.isComposing) { event.preventDefault(); event.currentTarget.form?.requestSubmit(); } }} />
    <div className="composer-toolbar"><IconButton label="添加图片或视频" disabled={!preview && (running || pending)} onClick={() => fileInput.current?.click()}><Paperclip size={17} /></IconButton>

      <Choice label="由 Rust 强制执行" value={mode} disabled={!preview && blocked} onChange={(value) => onSetMode(value as Mode)} options={[
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
                onClick={() => { if (!preview && Number.isFinite(numeric)) onSelectProject?.(numeric); }}>
                <span className="model-picker-id">{project.name}{!project.path && <small className="project-picker-no-path"> 未设目录</small>}</span>
                {!preview && !project.path && Number.isFinite(numeric) && onSetProjectPath && onBrowse &&
                  <button type="button" className="project-picker-setpath" aria-label={`设置目录 ${project.name}`} onClick={async (event) => {
                    event.stopPropagation();
                    const dir = await onBrowse();
                    if (dir) await onSetProjectPath(numeric, dir);
                  }}>设置目录</button>}
                {project.path && <span className="model-picker-context project-picker-path" title={project.path}>{project.path.split(/[\/]/).pop()}</span>}
              </div>;
            })}
          </div>
          <button type="button" className="model-picker-level" onClick={onNewProject}>＋ 新建项目</button>
        </Popover.Content></Popover.Portal>
      </Popover.Root>
      <div className="flex-1" /><ComposerControls preview={preview} disabled={blocked} preferences={preferences} onPreferences={onPreferences} settingsVersion={settingsVersion} onError={onError} onCapability={onCapability} />
      <span className={`input-count ${promptBytes > 8192 ? 'invalid' : ''}`}>{promptBytes > 0 ? `${promptBytes} / 8192` : 'Ctrl ↵'}</span>
      {running && !preview ? <IconButton label="停止生成" className="stop-button" disabled={pending} onClick={onCancel}><Square size={13} /></IconButton> : <button type="submit" aria-label="发送" className="send-button" disabled={blocked || promptBytes === 0 || promptBytes > 8192}><ArrowUp size={19} strokeWidth={1.8} /></button>}
    </div>
  </form><SessionStats runs={runs} metrics={metrics} context={context} /><div className="composer-caption"><span><ShieldCheck size={11} />{preview ? '预览不调用模型；媒体仅本地显示' : '图片/文本附件随消息发送（图片需视觉模型）；视频仅本地预览。'}</span><span>{modelLabel || 'Maestro Agent'}</span></div></div>;
}
