import { FileText, Image, X } from 'lucide-react';
import { useState } from 'react';

export interface LocalAttachment { id: string; name: string; type: 'image' | 'text'; url: string; dataUrl?: string; text?: string }

/// base64 体积换算（含 padding 修正）——注入回执的体积数字。
function dataUrlBytes(dataUrl: string): number {
  const base64 = dataUrl.slice(dataUrl.indexOf(',') + 1);
  const padding = base64.endsWith('==') ? 2 : base64.endsWith('=') ? 1 : 0;
  return Math.max(0, Math.floor((base64.length * 3) / 4) - padding);
}

const kib = (bytes: number) => bytes >= 1024 * 1024 ? `${(bytes / 1024 / 1024).toFixed(1)} MiB` : `${Math.round(bytes / 1024)} KiB`;

function AttachmentCard({ item, onRemove, sent }: { item: LocalAttachment; onRemove?: (id: string) => void; sent: boolean }) {
  const [dims, setDims] = useState<string>('');
  // 注入回执：图片字节确实进入了本轮模型请求上下文（后端对当前轮全量携带，
  // 注入是构建请求的确定行为）——陈述「已注入」，不承诺「模型已理解」。
  const receipt = sent && item.type === 'image' && item.dataUrl
    ? `已注入模型上下文${dims ? ` · ${dims}` : ''} · ${kib(dataUrlBytes(item.dataUrl))}`
    : sent
      ? (item.type === 'image' && !item.dataUrl ? '本地预览 · 图片未发送' : '已随消息发送')
      : 'Local preview only';
  return <div className="attachment-preview">
    {item.type === 'image'
      ? <img src={item.url} alt={item.name} onLoad={(event) => {
          const img = event.currentTarget;
          if (img.naturalWidth) setDims(`${img.naturalWidth}×${img.naturalHeight}`);
        }} />
      : <span className="attachment-text-icon">{item.name.slice(0, 2)}</span>}
    <div>{item.type === 'image' ? <Image size={11} /> : <FileText size={11} />}<span>{item.name}</span></div>
    <small>{receipt}</small>
    {onRemove && <button type="button" aria-label={`移除附件 ${item.name}`} onClick={() => onRemove(item.id)}><X size={12} /></button>}
  </div>;
}

export default function LocalAttachments({ items, onRemove, sent = false }: { items: LocalAttachment[]; onRemove?: (id: string) => void; sent?: boolean }) {
  if (!items.length) return null;
  return <div className={`local-attachments ${sent ? 'sent-attachments' : ''}`}>{items.map((item) => <AttachmentCard key={item.id} item={item} onRemove={onRemove} sent={sent} />)}</div>;
}
