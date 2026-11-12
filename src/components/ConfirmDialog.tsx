import * as Dialog from '@radix-ui/react-dialog';
import { TriangleAlert } from 'lucide-react';

import { Button } from './ui';

/// Destructive confirmation shared by session/project/provider deletion.
/// Radix Dialog keeps focus, Escape and the overlay contract consistent with
/// the other dialogs; callers only own the open state and action.
export default function ConfirmDialog({ open, onOpenChange, title, description, confirmLabel = '删除', cancelLabel = '取消', danger = true, onConfirm }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
  onConfirm: () => void;
}) {
  return <Dialog.Root open={open} onOpenChange={onOpenChange}>
    <Dialog.Portal>
      <Dialog.Overlay className="dialog-overlay" />
      <Dialog.Content className="confirm-dialog" aria-describedby={undefined}>
        <Dialog.Title className="confirm-dialog-title"><TriangleAlert size={18} />{title}</Dialog.Title>
        <Dialog.Description className="confirm-dialog-description">{description}</Dialog.Description>
        <div className="confirm-dialog-actions">
          <Dialog.Close asChild><Button variant="ghost">{cancelLabel}</Button></Dialog.Close>
          <Button variant={danger ? 'destructive' : 'primary'} onClick={() => { onConfirm(); onOpenChange(false); }}>{confirmLabel}</Button>
        </div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}
