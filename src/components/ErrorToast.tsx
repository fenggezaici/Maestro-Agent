import { useEffect } from 'react';
import { CircleAlert, X } from 'lucide-react';

export default function ErrorToast({ message, onDismiss }: { message: string | null; onDismiss: () => void }) {
  useEffect(() => {
    if (!message) return;
    const timeout = window.setTimeout(onDismiss, 7000);
    return () => window.clearTimeout(timeout);
  }, [message, onDismiss]);
  if (!message) return null;
  return <div role="alert" aria-live="assertive" className="error-toast"><CircleAlert size={17} /><div><strong>请求失败</strong><p>{message}</p></div><button type="button" aria-label="关闭错误提示" className="icon-button" onClick={onDismiss}><X size={14} /></button></div>;
}
