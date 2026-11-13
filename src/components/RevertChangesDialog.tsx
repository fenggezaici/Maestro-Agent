import * as Dialog from '@radix-ui/react-dialog';
import { Loader2, Undo2 } from 'lucide-react';
import type { RevertPreview } from '../api/changes';

const REASON: Record<string, string> = {
  external_modified: '文件在之后被修改',
  file_read_failed: '文件读取失败',
  checkpoint_missing: '旧版本没有可回退快照',
};

export default function RevertChangesDialog({ open, onOpenChange, preview, loading, applying, error, onApply }: {
  open: boolean; onOpenChange: (open: boolean) => void; preview: RevertPreview | null;
  loading: boolean; applying: boolean; error: string | null; onApply: () => void;
}) {
  const safe = preview?.safeFiles.length ?? 0;
  const unsafe = preview?.unsafeFiles ?? [];
  const ignored = preview?.ignoredFiles ?? [];
  return <Dialog.Root open={open} onOpenChange={onOpenChange}>
    <Dialog.Portal><Dialog.Overlay className="dialog-overlay" />
      <Dialog.Content className="confirm-dialog" aria-describedby={undefined}>
        <Dialog.Title className="confirm-dialog-title"><Undo2 size={18} />撤销本轮文件修改</Dialog.Title>
        <Dialog.Description className="confirm-dialog-description">
          将恢复本轮修改前的文件内容。被后续修改或缺少快照的文件会跳过。
        </Dialog.Description>
        <div className="revert-preview">
          {loading ? <p><Loader2 size={13} className="spin" />正在检查文件状态…</p> : preview ? <>
            <p><strong>{safe}</strong> 个文件可安全撤销</p>
            {preview.safeFiles.map((path) => <div className="revert-file" key={path}><span>{path}</span><em>可撤销</em></div>)}
            {unsafe.map((item) => <div className="revert-file" key={item.path}><span>{item.path}</span><em>{REASON[item.reason] ?? item.reason}</em></div>)}
            {ignored.map((item) => <div className="revert-file" key={item.path}><span>{item.path}</span><em>{REASON[item.reason] ?? item.reason}</em></div>)}
          </> : <p>没有可用的回退信息。</p>}
          {error && <p role="alert" className="settings-form-error">{error}</p>}
        </div>
        <div className="confirm-dialog-actions">
          <Dialog.Close asChild><button type="button" className="button-ghost">取消</button></Dialog.Close>
          <button type="button" className="button-approve" disabled={!preview?.safeFiles.length || loading || applying} onClick={onApply}>
            {applying ? '撤销中…' : '确认撤销'}
          </button>
        </div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}
