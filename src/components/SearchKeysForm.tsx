import { useEffect, useState } from 'react';
import { Eye, EyeOff, Search, Trash2 } from 'lucide-react';
import { hasProviderCredential } from '../settings';
import { credentialsApi, isLiveDesktop, refreshCredentialStatus } from '../api/credentials';
import { errorText } from '../lib/errors';

const ENGINES = [
  { id: 'tavily', name: 'Tavily', hint: '为 agent 场景设计，摘要更结构化' },
  { id: 'brave', name: 'Brave', hint: '独立索引，免费额度内足够个人使用' },
] as const;

/// 联网搜索的 BYO key 配置：与 LLM key 同一凭据链路（DPAPI 保护、明文不落地）。
/// 引擎自动解析——配了哪个用哪个，两者都有时 Tavily 优先；无需额外选择状态。
export default function SearchKeysForm({ preview }: { preview: boolean }) {
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [stored, setStored] = useState<Record<string, boolean>>({});
  const [showKey, setShowKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    const next: Record<string, boolean> = {};
    for (const engine of ENGINES) next[engine.id] = hasProviderCredential(engine.id);
    setStored(next);
  }, [preview, notice]);

  async function save(id: string) {
    if (preview) return;
    const key = (drafts[id] ?? '').trim();
    if (!key) { setError('请输入 key 后再保存'); return; }
    try {
      await credentialsApi.save(id, key);
      await refreshCredentialStatus();
      setDrafts((current) => ({ ...current, [id]: '' }));
      setError(null);
      setNotice(`${id} key 已保存到系统凭据`);
    } catch (reason) { setError(errorText(reason)); }
  }

  async function clear(id: string) {
    if (preview) return;
    try {
      await credentialsApi.clear(id);
      await refreshCredentialStatus();
      setError(null);
      setNotice(`${id} key 已清除`);
    } catch (reason) { setError(errorText(reason)); }
  }

  return <div className="search-keys-form">
    <div className="settings-section-label"><Search size={13} />联网搜索（web_search 工具）</div>
    <p className="settings-note">配置任一引擎的 key 即可启用；两者都配置时 Tavily 优先。key 经系统凭据保护存储，不回显。{!isLiveDesktop() && !preview ? '' : ''}</p>
    {ENGINES.map((engine) => <div key={engine.id} className="preference-row search-key-row">
      <div>
        <strong>{engine.name}{stored[engine.id] ? <span className="credential-stored-tag">已配置</span> : null}</strong>
        <p>{engine.hint}</p>
      </div>
      <div className="search-key-controls">
        <div className="secret-input">
          <input type={showKey === engine.id ? 'text' : 'password'} autoComplete="off" spellCheck={false} disabled={preview}
            value={drafts[engine.id] ?? ''} aria-label={`${engine.name} API key`}
            onChange={(event) => setDrafts((current) => ({ ...current, [engine.id]: event.target.value }))}
            placeholder={stored[engine.id] ? '已保存 · 输入以覆盖' : '输入 API key'} />
          <button type="button" className="icon-button" aria-label="显示或隐藏 key" onClick={() => setShowKey(showKey === engine.id ? null : engine.id)}>{showKey === engine.id ? <EyeOff size={13} /> : <Eye size={13} />}</button>
        </div>
        <button type="button" className="button-ghost" disabled={preview} onClick={() => void save(engine.id)}>保存</button>
        {stored[engine.id] ? <button type="button" className="icon-button" aria-label={`清除 ${engine.name} key`} disabled={preview} onClick={() => void clear(engine.id)}><Trash2 size={13} /></button> : null}
      </div>
    </div>)}
    {error && <p className="settings-note" role="alert">{error}</p>}
    {notice && !error && <p className="settings-note" role="status">{notice}</p>}
  </div>;
}
