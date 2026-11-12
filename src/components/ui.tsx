import * as Select from '@radix-ui/react-select';
import * as Tooltip from '@radix-ui/react-tooltip';
import { Check, ChevronDown } from 'lucide-react';
import type { ButtonHTMLAttributes, ReactNode } from 'react';

export type ButtonVariant = 'primary' | 'secondary' | 'outline' | 'ghost' | 'destructive';
export type ButtonSize = 'xs' | 'sm' | 'md' | 'lg' | 'icon';

/// Shared action button: one variant/size contract for new UI. Existing
/// one-off classes remain during the region-by-region migration.
export function Button({ variant = 'secondary', size = 'md', className = '', ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant; size?: ButtonSize }) {
  return <button type="button" className={`btn btn-${variant} btn-${size} ${className}`} {...props} />;
}

export function IconButton({ label, children, className = '', ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { label: string }) {
  return <Tooltip.Root>
    <Tooltip.Trigger asChild><button type="button" aria-label={label} className={`icon-button ${className}`} {...props}>{children}</button></Tooltip.Trigger>
    <Tooltip.Portal><Tooltip.Content className="tooltip-content" sideOffset={8}>{label}<Tooltip.Arrow className="fill-[#333]" /></Tooltip.Content></Tooltip.Portal>
  </Tooltip.Root>;
}

export function Choice({ label, value, onChange, options, disabled, className = '' }: {
  label: string; value: string; onChange: (value: string) => void;
  options: { value: string; label: ReactNode }[]; disabled?: boolean; className?: string;
}) {
  return <Select.Root value={value} onValueChange={onChange} disabled={disabled}>
    <Select.Trigger aria-label={label} className={`choice-trigger ${className}`}>
      <Select.Value /><Select.Icon><ChevronDown size={13} strokeWidth={1.6} /></Select.Icon>
    </Select.Trigger>
    <Select.Portal><Select.Content position="popper" sideOffset={6} className="choice-content">
      <Select.Viewport>{options.map((option) => <Select.Item key={option.value} value={option.value} className="choice-item">
        <Select.ItemText>{option.label}</Select.ItemText><Select.ItemIndicator><Check size={13} /></Select.ItemIndicator>
      </Select.Item>)}</Select.Viewport>
    </Select.Content></Select.Portal>
  </Select.Root>;
}
