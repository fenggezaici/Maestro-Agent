import { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowLeft, ArrowRight, Globe, RotateCw } from 'lucide-react';
import { normalizeUrl } from './browserUrl';

const PLACEHOLDER = 'localhost:3000';

/// 内嵌浏览器预览：面向 dev server 场景的 iframe 面板。
/// 诚实边界——跨域 iframe 读不到真实地址与历史深度，后退/前进经 history
/// 导航命令下发（同源策略允许导航不允许读取），按钮因此始终可用、无历史时
/// 点击无效果；地址栏只反映"我们导航过的地址"，页面内跳转不回显。
export default function BrowserPane({ storageKey }: { storageKey: string }) {
  const [url, setUrl] = useState<string | null>(() => {
    try { return localStorage.getItem(storageKey); } catch { return null; }
  });
  const [draft, setDraft] = useState(() => url ?? '');
  const [error, setError] = useState<string | null>(null);
  const [reloadNonce, setReloadNonce] = useState(0);
  const frameRef = useRef<HTMLIFrameElement>(null);

  const navigate = useCallback((raw: string) => {
    const normalized = normalizeUrl(raw);
    if (!normalized) { setError('不是有效的 http/https 地址'); return; }
    setError(null);
    setUrl(normalized);
    setDraft(normalized);
    try { localStorage.setItem(storageKey, normalized); } catch { /* quota */ }
  }, [storageKey]);

  // 项目切换后地址栏跟随该项目持久化的 URL。
  useEffect(() => {
    let saved: string | null = null;
    try { saved = localStorage.getItem(storageKey); } catch { /* storage unavailable */ }
    setUrl(saved);
    setDraft(saved ?? '');
    setError(null);
  }, [storageKey]);

  const historyNav = (direction: 'back' | 'forward') => {
    try {
      const view = frameRef.current?.contentWindow;
      if (!view) return;
      if (direction === 'back') view.history.back(); else view.history.forward();
    } catch { /* 跨域导航命令多数情况被允许；失败时静默无操作 */ }
  };

  return <div className="browser-pane">
    <form className="browser-toolbar" onSubmit={(event) => { event.preventDefault(); navigate(draft); }}>
      <button type="button" className="icon-button" aria-label="后退" disabled={!url} onClick={() => historyNav('back')}><ArrowLeft size={13} /></button>
      <button type="button" className="icon-button" aria-label="前进" disabled={!url} onClick={() => historyNav('forward')}><ArrowRight size={13} /></button>
      <button type="button" className="icon-button" aria-label="刷新" disabled={!url} onClick={() => setReloadNonce((nonce) => nonce + 1)}><RotateCw size={13} /></button>
      <input
        className="browser-address"
        value={draft}
        onChange={(event) => { setDraft(event.target.value); if (error) setError(null); }}
        placeholder={PLACEHOLDER}
        aria-label="地址栏"
        spellCheck={false}
      />
      <button type="submit" className="icon-button browser-go" aria-label="访问"><Globe size={13} /></button>
    </form>
    {error && <p className="browser-error" role="alert">{error}</p>}
    {url
      ? <iframe key={`${url}#${reloadNonce}`} ref={frameRef} className="browser-frame" src={url} title="内嵌浏览器预览" />
      : <div className="editor-empty"><Globe size={34} strokeWidth={1} /><h3>预览本地页面</h3><p>输入 dev server 地址（如 localhost:3000）回车加载；页面内跳转不回显地址栏。</p></div>}
  </div>;
}
