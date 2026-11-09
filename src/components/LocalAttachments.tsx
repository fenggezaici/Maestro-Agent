import { Film, Image, X } from 'lucide-react';

export interface LocalAttachment { id: string; name: string; type: 'image' | 'video'; url: string }

export default function LocalAttachments({ items, onRemove, sent = false }: { items: LocalAttachment[]; onRemove?: (id: string) => void; sent?: boolean }) {
  if (!items.length) return null;
  return <div className={`local-attachments ${sent ? 'sent-attachments' : ''}`}>{items.map((item) => <div key={item.id} className="attachment-preview">
    {item.type === 'image' ? <img src={item.url} alt={item.name} /> : <video src={item.url} preload="metadata" muted aria-label={item.name} />}
    <div>{item.type === 'image' ? <Image size={11} /> : <Film size={11} />}<span>{item.name}</span></div>
    <small>{sent ? '未上传 · 本地占位' : 'Local preview only'}</small>
    {onRemove && <button type="button" aria-label={`移除附件 ${item.name}`} onClick={() => onRemove(item.id)}><X size={12} /></button>}
  </div>)}</div>;
}
