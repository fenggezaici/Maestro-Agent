import { useState, type FormEvent } from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import { FolderGit2, FolderOpen, Trash2, X } from 'lucide-react';
import type { WorkbenchProject } from './workspace';
import { IconButton } from './ui';

export function NewProjectDialog({ open, onOpenChange, disabled, onBrowse, onCreate }: {
  open: boolean; onOpenChange: (open: boolean) => void; disabled?: boolean;
  onBrowse?: () => Promise<string | null>;
  onCreate: (name: string, path?: string) => Promise<string | null>;
}) {
  const [name, setName] = useState('');
  const [path, setPath] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending || disabled) return;
    setPending(true);
    const message = await onCreate(name.trim(), path.trim() || undefined);
    setPending(false);
    if (message) { setError(message); return; }
    setName(''); setPath(''); setError(null);
    onOpenChange(false);
  }

  return <Dialog.Root open={open} onOpenChange={(value) => { setError(null); onOpenChange(value); }}>
    <Dialog.Portal><Dialog.Overlay className="dialog-overlay" />
      <Dialog.Content className="project-dialog" aria-describedby={undefined}>
        <Dialog.Title asChild><h3><FolderGit2 size={16} />新建项目</h3></Dialog.Title>
        <p className="dialog-hint">项目是一组会话的容器，可选绑定一个本地工作区目录（Explorer 将切换到该目录）。</p>
        <form onSubmit={(event) => void submit(event)}>
          <label htmlFor="project-name">项目名称</label>
          <input id="project-name" autoComplete="off" spellCheck={false} autoFocus value={name} disabled={disabled || pending}
            onChange={(event) => setName(event.target.value)} placeholder="My Project" maxLength={128} />
          <label htmlFor="project-path"><FolderOpen size={11} /> 工作区目录（可选）</label>
          <div className="field-with-browse">
            <input id="project-path" autoComplete="off" spellCheck={false} value={path} disabled={disabled || pending}
              onChange={(event) => setPath(event.target.value)} placeholder="E:\code\my-project" />
            {onBrowse && <IconButton label="浏览目录" disabled={disabled || pending}
              onClick={async () => { const picked = await onBrowse(); if (picked) setPath(picked); }}><FolderOpen size={14} /></IconButton>}
          </div>
          {error && <p role="alert" className="settings-form-error">{error}</p>}
          <div className="dialog-actions">
            <Dialog.Close asChild><button type="button" className="button-ghost">取消</button></Dialog.Close>
            <button type="submit" className="button-approve" disabled={disabled || pending || !name.trim()}>创建项目</button>
          </div>
        </form>
        <Dialog.Close asChild><button type="button" className="icon-button dialog-close" aria-label="关闭"><X size={15} /></button></Dialog.Close>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}

export function DeleteProjectDialog({ project, onOpenChange, onConfirm }: {
  project: WorkbenchProject | null; onOpenChange: (open: boolean) => void;
  onConfirm: (projectId: string) => Promise<string | null>;
}) {
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const open = project !== null;
  const sessionCount = (project?.sessions.length ?? 0) + (project?.archivedSessions?.length ?? 0);
  return <Dialog.Root open={open} onOpenChange={(value) => { if (!value) { setError(null); onOpenChange(false); } }}>
    <Dialog.Portal><Dialog.Overlay className="dialog-overlay" />
      <Dialog.Content className="project-dialog" aria-describedby={undefined}>
        <Dialog.Title asChild><h3 className="dialog-danger"><Trash2 size={15} />删除项目</h3></Dialog.Title>
        <p className="dialog-hint">将永久删除项目 <strong>{project?.name}</strong> 及其 {sessionCount} 个会话（含全部对话记录）。此操作不可撤销。</p>
        {error && <p role="alert" className="settings-form-error">{error}</p>}
        <div className="dialog-actions">
          <Dialog.Close asChild><button type="button" className="button-ghost">取消</button></Dialog.Close>
          <button type="button" className="button-approve dialog-danger" disabled={pending} onClick={async () => {
            if (!project || pending) return;
            setPending(true);
            const message = await onConfirm(project.id);
            setPending(false);
            if (message) { setError(message); return; }
            onOpenChange(false);
          }}>删除项目</button>
        </div>
        <Dialog.Close asChild><button type="button" className="icon-button dialog-close" aria-label="关闭"><X size={15} /></button></Dialog.Close>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}
