import { useState, type ReactNode } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';

/// Generic ZCode-style collapsible block: summary row + animated expand
/// (grid-template-rows 0fr→1fr transition), default collapsed.
export default function CollapsibleBlock({ title, icon, subtitle, defaultOpen = false, contentClass = '', children }: {
  title: ReactNode; icon?: ReactNode; subtitle?: ReactNode; defaultOpen?: boolean; contentClass?: string; children: ReactNode;
}) {
  const [open, setOpen] = useState(defaultOpen);
  return <div className={`collapsible-block ${open ? 'open' : ''} ${contentClass}`}>
    <button type="button" className="collapsible-summary" aria-expanded={open} onClick={() => setOpen((value) => !value)}>
      {open ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
      {icon}<span className="collapsible-title">{title}</span>{subtitle && <span className="collapsible-subtitle">{subtitle}</span>}
    </button>
    <div className="collapsible-body" aria-hidden={!open}><div className="collapsible-inner">{children}</div></div>
  </div>;
}
