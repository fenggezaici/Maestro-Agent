/// 节点库浮层：Houdini Tab / ComfyUI 搜索框范式。
/// 锚定在唤起点的非模态面板；↑↓ 导航、Enter 放置、Shift+Enter 连续放置、Esc 关闭。
/// 连线松手唤起时（pending）只显示可连接的节点类型。
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import * as Tooltip from '@radix-ui/react-tooltip';
import { Search, Zap } from 'lucide-react';
import {
  filterLibrary, readRecentIds, type ItemDetail, type LibraryItem, type LibrarySection, type PendingConnection,
} from '../lib/nodeLibrary';

/// 条目详情卡：标题 + 用途描述 + 元信息表 + 图中语义行。
/// 左侧树卡片、浮层行、画布扩展节点共用；无 detail 时原样渲染 children。
export function ItemHint({ title, detail, side = 'right', open, children }: {
  title: string;
  detail?: ItemDetail;
  side?: 'top' | 'right' | 'bottom' | 'left';
  /** 传入 false 可强制关闭（如拖拽 ghost 期间）。 */
  open?: boolean;
  children: ReactNode;
}) {
  if (!detail) return <>{children}</>;
  return (
    <Tooltip.Root open={open}>
      <Tooltip.Trigger asChild>{children}</Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content className="item-hint" side={side} sideOffset={8} collisionPadding={10}>
          <strong className="item-hint-title">{title}</strong>
          {detail.description && <p className="item-hint-desc">{detail.description}</p>}
          {detail.meta && detail.meta.length > 0 && (
            <dl className="item-hint-meta">
              {detail.meta.map(([label, value]) => (
                <div key={label} className="item-hint-meta-row">
                  <dt>{label}</dt><dd title={value}>{value}</dd>
                </div>
              ))}
            </dl>
          )}
          {detail.usage && <p className="item-hint-usage">{detail.usage}</p>}
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

export interface NodeLibraryProps {
  /** 相对 .sandbox-canvas-wrap 的锚点（已由父组件夹取边界）。 */
  anchor: { x: number; y: number };
  pending?: PendingConnection | null;
  /** pending 上下文提示行（如 '⚡ 修改文件 → ?'）。 */
  pendingLabel?: string;
  items: LibraryItem[];
  onPick: (item: LibraryItem, keepOpen: boolean) => void;
  onClose: () => void;
}

export default function NodeLibrary({ anchor, pending, pendingLabel, items, onPick, onClose }: NodeLibraryProps) {
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const recentIds = useMemo(() => readRecentIds(), []);

  const sections: LibrarySection[] = useMemo(
    () => filterLibrary(items, query, pending, recentIds),
    [items, query, pending, recentIds],
  );
  const flat = useMemo(() => sections.flatMap((section) => section.items), [sections]);

  useEffect(() => { setActive(0); }, [query]);
  useEffect(() => {
    listRef.current?.querySelector('[data-active="true"]')?.scrollIntoView?.({ block: 'nearest' });
  }, [active]);

  const pick = (entry: LibraryItem | undefined, keepOpen: boolean) => {
    if (!entry) return;
    if (keepOpen) setQuery('');
    onPick(entry, keepOpen);
  };

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      setActive((value) => flat.length ? (value + 1) % flat.length : 0);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      setActive((value) => flat.length ? (value - 1 + flat.length) % flat.length : 0);
    } else if (event.key === 'Enter') {
      event.preventDefault();
      pick(flat[active], event.shiftKey);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      onClose();
    }
  };

  let index = -1;
  return (
    <div className="node-library" style={{ left: anchor.x, top: anchor.y }} role="dialog" aria-label="节点库"
      onKeyDown={onKeyDown} onPointerDown={(event) => event.stopPropagation()}>
      <div className="node-library-search">
        <Search size={14} />
        <input autoFocus aria-label="搜索节点" value={query}
          placeholder={pending ? '搜索可连接的节点…' : '搜索节点（输入、权限、触发器、扩展…）'}
          onChange={(event) => setQuery(event.target.value)} />
        <kbd className="node-library-kbd">Esc</kbd>
      </div>
      <div className="node-library-context">
        {pending ? <><Zap size={11} /> {pendingLabel ?? '选择要连接的节点'}</> : '选择节点放置到画布'}
      </div>
      <div ref={listRef} className="node-library-list" role="listbox">
        {flat.length === 0 && (
          <p className="node-library-empty">{pending ? '没有可连接的节点类型' : '没有匹配的节点'}</p>
        )}
        {sections.map((section) => (
          <div key={section.label} className="node-library-section">
            <p className="node-library-section-label">{section.label}</p>
            {section.items.map((entry) => {
              index += 1;
              const position = index;
              return (
                <ItemHint key={entry.id} title={entry.title} detail={entry.detail}>
                  <button type="button" role="option" aria-selected={position === active}
                    data-active={position === active} className="node-library-item"
                    onMouseEnter={() => setActive(position)}
                    onClick={(event) => pick(entry, event.shiftKey)}>
                    <span className="node-library-item-title">{entry.title}</span>
                    {entry.subtitle && <span className="node-library-item-sub">{entry.subtitle}</span>}
                    <span className="node-library-item-cat">{entry.category.join(' / ')}</span>
                  </button>
                </ItemHint>
              );
            })}
          </div>
        ))}
      </div>
      <div className="node-library-footer">
        {/* 键盘流补偿：↑↓ 选中时无 hover，在这里展示当前项语义 */}
        {(flat[active]?.detail?.usage ?? flat[active]?.detail?.description) && (
          <p className="node-library-active-hint">{flat[active]!.detail!.usage ?? flat[active]!.detail!.description}</p>
        )}
        <span>↑↓ 选择 · Enter 放置 · Shift+Enter 连续放置 · Esc 关闭</span>
      </div>
    </div>
  );
}
