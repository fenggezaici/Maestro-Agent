import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import { FileCode2, History, MessageSquare, Search, TerminalSquare } from 'lucide-react';
import { fuzzyScore, pushPaletteHistory, readPaletteHistory, type FlatFile } from '../commandCenter';
import { relativeSessionAge, type WorkbenchProject } from './workspace';
import type { SessionView } from '../api/types';

export interface PaletteCommand {
  id: string;
  label: string;
  hint?: string;
  icon?: ReactNode;
  disabled?: boolean;
  run: () => void;
}

interface PaletteItem {
  key: string;
  label: string;
  detail?: string;
  hint?: string;
  icon: ReactNode;
  disabled?: boolean;
  historyLabel?: string;
  /** Re-applies the stored query instead of executing an action. */
  requery?: boolean;
  run: () => void;
}

interface PaletteSection {
  id: 'recent' | 'commands' | 'sessions' | 'files';
  label: string;
  items: PaletteItem[];
}

const SECTION_LIMIT = 40;

export default function CommandCenter({ open, onOpenChange, source, projects, commands, files, filesLoading, onSelectSession, onSelectFile }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  source: 'live' | 'preview';
  projects: WorkbenchProject[];
  commands: PaletteCommand[];
  files: FlatFile[];
  filesLoading?: boolean;
  onSelectSession: (id: number) => void;
  onSelectFile: (path: string) => void;
}) {
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const sessionRows = useMemo(() => projects.flatMap((project) =>
    project.sessions.map((session) => ({ session, projectName: project.name }))), [projects]);

  const sections = useMemo<PaletteSection[]>(() => {
    const trimmed = query.trim();
    const match = (label: string, extra = '') => fuzzyScore(trimmed, `${label} ${extra}`);
    const sort = <T extends { score: number }>(rows: T[]) => rows.sort((a, b) => b.score - a.score).slice(0, SECTION_LIMIT);
    if (!trimmed) {
      const recent = readPaletteHistory(source).map((entry, index) => ({
        key: `recent-${index}`, label: entry.label, detail: entry.detail, icon: <History size={14} />,
        historyLabel: entry.label, requery: true,
        run: () => setQuery(entry.label),
      }));
      return [
        ...(recent.length ? [{ id: 'recent' as const, label: 'Recent', items: recent }] : []),
        { id: 'commands' as const, label: 'Commands', items: commands.map((command) => toItem(command, 'cmd')) },
      ];
    }
    const commandItems = sort(commands.map((command) => ({ score: match(command.label) ?? -1, command }))
      .filter((row) => row.score >= 0)).map((row) => toItem(row.command, 'cmd'));
    const sessionItems = sort(sessionRows.map(({ session, projectName }) => ({
      score: match(session.title, projectName) ?? -1, session, projectName,
    })).filter((row) => row.score >= 0)).map(({ session, projectName }) => ({
      key: `session-${session.id}`, label: session.title,
      detail: `${projectName} · ${relativeSessionAge(session.updatedAt, source)} · #${session.id}`,
      icon: <MessageSquare size={14} />, historyLabel: session.title,
      run: () => onSelectSession(session.id),
    } satisfies PaletteItem));
    const fileItems = sort(files.map((file) => ({ score: match(file.path, file.name) ?? -1, file }))
      .filter((row) => row.score >= 0)).map(({ file }) => ({
        key: `file-${file.path}`, label: file.name, detail: file.path,
        icon: <FileCode2 size={14} />, historyLabel: file.path,
        run: () => onSelectFile(file.path),
      } satisfies PaletteItem));
    return [
      { id: 'commands' as const, label: 'Commands', items: commandItems },
      { id: 'sessions' as const, label: 'Sessions', items: sessionItems },
      ...(files.length || filesLoading ? [{ id: 'files' as const, label: 'Files', items: fileItems }] : []),
    ].filter((section) => section.items.length > 0 || (section.id === 'files' && filesLoading));
    // eslint-disable-next-line react-hooks/exhaustive-deps -- history entries are read on demand per query reset.
  }, [query, source, commands, sessionRows, files, filesLoading, onSelectSession, onSelectFile]);

  const flat = useMemo(() => sections.flatMap((section) => section.items), [sections]);
  useEffect(() => { setActive(0); }, [query, open]);
  useEffect(() => {
    if (!open) setQuery('');
  }, [open]);
  useEffect(() => {
    listRef.current?.querySelector('[data-active="true"]')?.scrollIntoView?.({ block: 'nearest' });
  }, [active]);

  function pick(item: PaletteItem | undefined) {
    if (!item || item.disabled) return;
    if (item.requery) { setQuery(item.historyLabel ?? item.label); return; }
    if (item.historyLabel) pushPaletteHistory(source, { label: item.historyLabel, detail: item.detail });
    onOpenChange(false);
    item.run();
  }

  function onKeyDown(event: React.KeyboardEvent) {
    if (event.nativeEvent.isComposing) return;
    if (event.key === 'ArrowDown') { event.preventDefault(); setActive((value) => flat.length ? (value + 1) % flat.length : 0); }
    else if (event.key === 'ArrowUp') { event.preventDefault(); setActive((value) => flat.length ? (value - 1 + flat.length) % flat.length : 0); }
    else if (event.key === 'Enter') { event.preventDefault(); pick(flat[active]); }
  }

  let index = -1;
  return <Dialog.Root open={open} onOpenChange={onOpenChange}>
    <Dialog.Portal>
      <Dialog.Overlay className="palette-overlay" />
      <Dialog.Content className="palette-dialog" aria-label="Command Center" onOpenAutoFocus={(event) => event.preventDefault()}>
        <Dialog.Title className="sr-only">Command Center</Dialog.Title>
        <div className="palette-input-row">
          <Search size={15} />
          <input autoFocus aria-label="搜索命令、会话或文件" value={query} placeholder="搜索命令、会话或文件…"
            onChange={(event) => setQuery(event.target.value)} onKeyDown={onKeyDown} />
          <kbd className="palette-kbd">Esc</kbd>
        </div>
        <div ref={listRef} className="palette-list" role="listbox" aria-label="命令面板结果">
          {sections.map((section) => <div key={section.id} className="palette-section">
            <p className="palette-section-label">{section.label}</p>
            {section.items.map((item) => {
              index += 1;
              const position = index;
              return <button key={item.key} type="button" role="option" aria-selected={position === active}
                data-active={position === active} className="palette-item" disabled={item.disabled}
                onMouseEnter={() => setActive(position)} onClick={() => pick(item)}>
                <span className="palette-item-icon">{item.icon}</span>
                <span className="palette-item-label">{item.label}</span>
                {item.detail && <span className="palette-item-detail">{item.detail}</span>}
                {item.hint && <kbd className="palette-kbd">{item.hint}</kbd>}
              </button>;
            })}
            {section.id === 'files' && filesLoading && <p className="palette-status" role="status">正在读取本地文件树…</p>}
          </div>)}
          {flat.length === 0 && !filesLoading && <p className="palette-status">没有匹配结果</p>}
        </div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}

function toItem(command: PaletteCommand, prefix: string): PaletteItem {
  return {
    key: `${prefix}-${command.id}`, label: command.label, hint: command.hint,
    icon: command.icon ?? <TerminalSquare size={14} />, disabled: command.disabled,
    historyLabel: command.label, run: command.run,
  };
}
