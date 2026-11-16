import { Dialog, DialogClose, DialogContent, DialogDescription, DialogTitle } from './ui/dialog';
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
  return <Dialog open={open} onOpenChange={onOpenChange}>
    <DialogContent variant="default" className="confirm-dialog">
      <DialogTitle className="confirm-dialog-title"><TriangleAlert size={18} />{title}</DialogTitle>
      <DialogDescription className="confirm-dialog-description">{description}</DialogDescription>
      <div className="confirm-dialog-actions">
        <DialogClose asChild><Button variant="ghost">{cancelLabel}</Button></DialogClose>
        <Button variant={danger ? 'destructive' : 'primary'} onClick={() => { onConfirm(); onOpenChange(false); }}>{confirmLabel}</Button>
      </div>
    </DialogContent>
  </Dialog>;
}
