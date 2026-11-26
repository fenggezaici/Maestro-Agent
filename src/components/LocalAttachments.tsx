import { FileText, Image, X } from 'lucide-react';

export interface LocalAttachment { id: string; name: string; type: 'image' | 'text'; url: string; dataUrl?: string; text?: string }

export default function LocalAttachments({ items, onRemove, sent = false }: { items: LocalAttachment[]; onRemove?: (id: string) => void; sent?: boolean }) {
  if (!items.length) return null;
  return <div className={`local-attachments ${sent ? 'sent-attachments' : ''}`}>{items.map((item) => <div key={item.id} className="attachment-preview">
    {item.type === 'image' ? <img src={item.url} alt={item.name} /> : <span className="attachment-text-icon">{item.name.slice(0, 2)}</span>}
    <div>{item.type === 'image' ? <Image size={11} /> : <FileText size={11} />}<span>{item.name}</span></div>
    <small>{sent ? (item.type === 'image' && !item.dataUrl ? '本地预览 · 图片未发送' : '已随消息发送') : 'Local preview only'}</small>
    {onRemove && <button type="button" aria-label={`移除附件 ${item.name}`} onClick={() => onRemove(item.id)}><X size={12} /></button>}
  </div>)}</div>;
}
