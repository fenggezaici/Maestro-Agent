import * as DialogPrimitive from '@radix-ui/react-dialog';
import { X } from 'lucide-react';
import type { ComponentProps, ReactNode } from 'react';

/// ZCode-style dialog adapter: the primitive owns portal, overlay, fixed
/// centering and z-index. Feature dialogs only provide content and a variant.
export const Dialog = DialogPrimitive.Root;
export const DialogTrigger = DialogPrimitive.Trigger;
export const DialogClose = DialogPrimitive.Close;
export const DialogTitle = DialogPrimitive.Title;
export const DialogDescription = DialogPrimitive.Description;

export type DialogVariant = 'default' | 'form' | 'panel' | 'palette';

const VARIANT_CLASS: Record<DialogVariant, string> = {
  default: 'dialog-content-default',
  form: 'dialog-content-form',
  panel: 'dialog-content-panel',
  palette: 'dialog-content-palette',
};

export function DialogContent({ children, variant = 'default', showClose = true, overlayClassName, className, ...props }: ComponentProps<typeof DialogPrimitive.Content> & {
  variant?: DialogVariant;
  showClose?: boolean;
  overlayClassName?: string;
}) {
  return <DialogPrimitive.Portal>
    <DialogPrimitive.Overlay className={`dialog-overlay${overlayClassName ? ` ${overlayClassName}` : ''}`} />
    <DialogPrimitive.Content aria-describedby={undefined} {...props}
      className={`dialog-content ${VARIANT_CLASS[variant]}${className ? ` ${className}` : ''}`}>
      {children}
      {showClose && <DialogPrimitive.Close asChild>
        <button type="button" className="icon-button dialog-close" aria-label="关闭"><X size={15} /></button>
      </DialogPrimitive.Close>}
    </DialogPrimitive.Content>
  </DialogPrimitive.Portal>;
}

export function DialogHeader({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={`dialog-header${className ? ` ${className}` : ''}`}>{children}</div>;
}

export function DialogFooter({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={`dialog-actions${className ? ` ${className}` : ''}`}>{children}</div>;
}
