import type { SlashGroup, SlashItem } from '../slashCommands';

/// Slash palette floating above the composer. Purely presentational: the
/// parent owns filtering, the active index and execution.
export default function SlashPanel({ groups, index, onHover, onSelect }: {
  groups: SlashGroup[];
  index: number;
  onHover: (index: number) => void;
  onSelect: (item: SlashItem) => void;
}) {
  const total = groups.reduce((count, group) => count + group.items.length, 0);
  let cursor = -1;
  return <div className="slash-panel" role="listbox" aria-label="斜杠命令">
    {total === 0 && <p className="slash-empty">没有匹配的命令</p>}
    {groups.map((group) => group.items.length === 0 ? null : <div className="slash-group" key={group.title}>
      <p className="slash-group-title">{group.title}</p>
      {group.items.map((item) => {
        cursor += 1;
        const active = cursor === index;
        return <button type="button" role="option" aria-selected={active} key={item.id}
          className={`slash-item${active ? ' active' : ''}`} disabled={item.disabled}
          onMouseEnter={() => onHover(cursor)}
          onMouseDown={(event) => { event.preventDefault(); onSelect(item); }}>
          <span className="slash-item-name">/{item.name}</span>
          <span className="slash-item-desc">{item.description || '（无描述）'}</span>
          <span className="slash-item-badge">{item.badge}</span>
        </button>;
      })}
    </div>)}
  </div>;
}
