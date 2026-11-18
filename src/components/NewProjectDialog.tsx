import { useEffect, useState, type FormEvent } from 'react';
import { Dialog, DialogClose, DialogContent, DialogTitle } from './ui/dialog';
import { Folder, FolderGit2, FolderOpen, FolderPlus, Pencil, Trash2 } from 'lucide-react';
import type { WorkbenchProject } from './workspace';
import { IconButton } from './ui';

const pathLeaf = (path: string) => path.replace(/[\\/]+$/, '').split(/[\\/]/).pop() ?? path;

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
    if (!path.trim()) { setError('请为项目选择工作区目录——模型需要它读取文件'); return; }
    const message = await onCreate(name.trim(), path.trim() || undefined);
    setPending(false);
    if (message) { setError(message); return; }
    setName(''); setPath(''); setError(null);
    onOpenChange(false);
  }

  return <Dialog open={open} onOpenChange={(value) => { setError(null); onOpenChange(value); }}>
    <DialogContent variant="form" className="project-dialog">
        <DialogTitle asChild><h3><FolderGit2 size={16} />新建项目</h3></DialogTitle>
        <p className="dialog-hint">项目是一组会话的容器，可选绑定一个本地工作区目录（Explorer 将切换到该目录）。</p>
        <form onSubmit={(event) => void submit(event)}>
          <label htmlFor="project-name">项目名称</label>
          <input id="project-name" autoComplete="off" spellCheck={false} autoFocus value={name} disabled={disabled || pending}
            onChange={(event) => setName(event.target.value)} placeholder="My Project" maxLength={128} />
          <label htmlFor="project-path"><FolderOpen size={11} /> 工作区目录（必填，模型将读取此目录）</label>
          <div className="field-with-browse">
            <input id="project-path" autoComplete="off" spellCheck={false} value={path} disabled={disabled || pending}
              onChange={(event) => setPath(event.target.value)} placeholder="E:\code\my-project" />
            {onBrowse && <IconButton label="浏览目录" disabled={disabled || pending}
              onClick={async () => { const picked = await onBrowse(); if (picked) setPath(picked); }}><FolderOpen size={14} /></IconButton>}
          </div>
          {error && <p role="alert" className="settings-form-error">{error}</p>}
          <div className="dialog-actions">
            <DialogClose asChild><button type="button" className="button-ghost">取消</button></DialogClose>
            <button type="submit" className="button-approve" disabled={disabled || pending || !name.trim()}>创建项目</button>
          </div>
        </form>
    </DialogContent>
  </Dialog>;
}

export function DeleteProjectDialog({ project, onOpenChange, onConfirm }: {
  project: WorkbenchProject | null; onOpenChange: (open: boolean) => void;
  onConfirm: (projectId: string) => Promise<string | null>;
}) {
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const open = project !== null;
  const sessionCount = (project?.sessions.length ?? 0) + (project?.archivedSessions?.length ?? 0);
  return <Dialog open={open} onOpenChange={(value) => { if (!value) { setError(null); onOpenChange(false); } }}>
    <DialogContent variant="default" className="project-dialog">
        <DialogTitle asChild><h3 className="dialog-danger"><Trash2 size={15} />删除项目</h3></DialogTitle>
        <p className="dialog-hint">将永久删除项目 <strong>{project?.name}</strong> 及其 {sessionCount} 个会话（含全部对话记录）。此操作不可撤销。</p>
        {error && <p role="alert" className="settings-form-error">{error}</p>}
        <div className="dialog-actions">
          <DialogClose asChild><button type="button" className="button-ghost">取消</button></DialogClose>
          <button type="button" className="button-approve dialog-danger" disabled={pending} onClick={async () => {
            if (!project || pending) return;
            setPending(true);
            const message = await onConfirm(project.id);
            setPending(false);
            if (message) { setError(message); return; }
            onOpenChange(false);
          }}>删除项目</button>
        </div>
    </DialogContent>
  </Dialog>;
}

export function EditProjectDialog({ project, onOpenChange, onBrowse, onSave, onDelete }: {
  project: WorkbenchProject | null;
  onOpenChange: (open: boolean) => void;
  onBrowse?: () => Promise<string | null>;
  onSave?: (name: string, path: string | null) => Promise<string | null>;
  onDelete?: (projectId: string) => void;
}) {
  const [name, setName] = useState('');
  const [path, setPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const open = project !== null;
  const projectId = project?.id;
  useEffect(() => {
    if (projectId === undefined) return;
    setName(project?.name ?? '');
    setPath(project?.path ?? null);
    setError(null);
  }, [projectId, project?.name, project?.path]);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending || !project) return;
    const nextName = name.trim();
    if (!nextName) { setError('项目名称不能为空'); return; }
    if (!onSave) { onOpenChange(false); return; }
    setPending(true);
    setError(null);
    const message = await onSave(nextName, path?.trim() || null);
    setPending(false);
    if (message) { setError(message); return; }
    onOpenChange(false);
  }

  return <Dialog open={open} onOpenChange={(value) => { if (!value) { setError(null); onOpenChange(false); } }}>
    <DialogContent variant="form" className="project-dialog">
      <DialogTitle asChild><h3><Pencil size={15} />编辑项目</h3></DialogTitle>
      <form onSubmit={(event) => void submit(event)}>
        <label htmlFor="edit-project-name">项目名称</label>
        <input id="edit-project-name" autoComplete="off" spellCheck={false} autoFocus value={name} disabled={pending}
          onChange={(event) => setName(event.target.value)} placeholder="项目名称" maxLength={128} />
        <label>源文件夹</label>
        <div className="edit-roots">
          {path && <div className="edit-root-row">
            <Folder size={14} />
            <span className="edit-root-name" title={path}>{pathLeaf(path)}</span>
            <span className="edit-root-badge">主要</span>
          </div>}
          {onBrowse && <button type="button" className="edit-root-add" disabled={pending}
            onClick={async () => { const picked = await onBrowse(); if (picked) setPath(picked); }}>
            <FolderPlus size={14} />{path ? '更换文件夹' : '添加文件夹'}
          </button>}
        </div>
        {error && <p role="alert" className="settings-form-error">{error}</p>}
        <div className="dialog-actions">
          {onDelete && <button type="button" className="button-ghost dialog-danger edit-project-delete" disabled={pending}
            onClick={() => { if (project && !pending) { onOpenChange(false); onDelete(project.id); } }}>删除项目</button>}
          <DialogClose asChild><button type="button" className="button-ghost">取消</button></DialogClose>
          <button type="submit" className="button-approve" disabled={pending || !name.trim() || !onSave}>保存</button>
        </div>
      </form>
    </DialogContent>
  </Dialog>;
}
