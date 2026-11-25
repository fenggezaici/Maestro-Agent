import { useEffect, useState, type FormEvent } from 'react';
import { FolderOpen } from 'lucide-react';
import { Dialog, DialogClose, DialogContent, DialogTitle } from './dialog';
import { pickDirectory, pickFile } from '../../api/dialog';
import { IconButton } from '../ui';

export interface FormField {
  id: string;
  label: string;
  placeholder?: string;
  required?: boolean;
  textarea?: boolean;
  browse?: 'directory' | 'file';
  disabled?: boolean;
  options?: { value: string; label: string; disabled?: boolean }[];
  /// 条件显示（例如 MCP 按类型切换 stdio/远程字段）；隐藏字段不参与必填校验。
  when?: (values: Record<string, string>) => boolean;
}

/** 通用小表单对话框：技能/MCP/子智能体导入等设置页共用。 */
export default function FormDialog({ open, onOpenChange, title, hint, submitLabel, fields, canBrowse = false, initialValues, onSubmit }: {
  open: boolean; onOpenChange: (open: boolean) => void; title: string; hint?: string;
  submitLabel: string;
  canBrowse?: boolean;
  fields: FormField[];
  initialValues?: Record<string, string>;
  onSubmit: (values: Record<string, string>) => Promise<string | null>;
}) {
  const defaults = () => ({
    ...Object.fromEntries(fields.filter((field) => field.options?.length).map((field) => [field.id, field.options![0].value])),
    ...initialValues,
  });
  const [values, setValues] = useState<Record<string, string>>(defaults);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  useEffect(() => { if (open) setValues(defaults()); }, [open]);
  const visible = fields.filter((field) => !field.when || field.when(values));
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) return;
    setPending(true);
    const message = await onSubmit(values);
    setPending(false);
    if (message) { setError(message); return; }
    setValues(defaults()); setError(null); onOpenChange(false);
  }
  return <Dialog open={open} onOpenChange={(value) => { setError(null); onOpenChange(value); }}>
    <DialogContent variant="form" className="project-dialog">
        <DialogTitle asChild><h3>{title}</h3></DialogTitle>
        {hint && <p className="dialog-hint">{hint}</p>}
        <form onSubmit={(event) => void submit(event)}>
          {visible.map((field) => <div key={field.id}>
            <label htmlFor={`ext-${field.id}`}>{field.label}{field.required !== false && !field.options ? ' *' : ''}</label>
            {field.options
              ? <select id={`ext-${field.id}`} value={values[field.id] ?? field.options[0]?.value ?? ''} disabled={field.disabled}
                onChange={(e) => setValues((v) => ({ ...v, [field.id]: e.target.value }))}>
                {field.options.map((option) => <option key={option.value} value={option.value} disabled={option.disabled}>{option.label}</option>)}
              </select>
              : field.textarea
                ? <textarea id={`ext-${field.id}`} rows={3} value={values[field.id] ?? ''} disabled={field.disabled} onChange={(e) => setValues((v) => ({ ...v, [field.id]: e.target.value }))} placeholder={field.placeholder} />
                : <div className="field-with-browse">
                  <input id={`ext-${field.id}`} autoComplete="off" spellCheck={false} value={values[field.id] ?? ''} disabled={field.disabled} onChange={(e) => setValues((v) => ({ ...v, [field.id]: e.target.value }))} placeholder={field.placeholder} />
                  {field.browse && <IconButton label={field.browse === 'directory' ? '浏览目录' : '浏览文件'} disabled={!canBrowse}
                    onClick={async () => {
                      const picked = field.browse === 'directory' ? await pickDirectory() : await pickFile(['json']);
                      if (picked) setValues((v) => ({ ...v, [field.id]: picked }));
                    }}><FolderOpen size={14} /></IconButton>}
                </div>}
          </div>)}
          {error && <p role="alert" className="settings-form-error">{error}</p>}
          <div className="dialog-actions">
            <DialogClose asChild><button type="button" className="button-ghost">取消</button></DialogClose>
            <button type="submit" className="button-approve" disabled={pending || visible.some((f) => f.required !== false && !f.options && !(values[f.id] ?? '').trim())}>{submitLabel}</button>
          </div>
        </form>
    </DialogContent>
  </Dialog>;
}
