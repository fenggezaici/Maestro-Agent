import { useState, type FormEvent } from 'react';
import { Eye, EyeOff, KeyRound, Save, Trash2 } from 'lucide-react';
import { clearLlmSettings, defaultLlmSettings, readLlmSettings, saveLlmSettings } from '../settings';

export default function LlmSettingsForm({ preview, onSaved, onChanged }: { preview: boolean; onSaved: () => void; onChanged?: () => void }) {
  const [values, setValues] = useState(() => preview ? { ...defaultLlmSettings } : readLlmSettings());
  const [showKey, setShowKey] = useState(false);
  const [error, setError] = useState<string | null>(null);
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (preview) return;
    try { saveLlmSettings(values); setError(null); onSaved(); } catch (reason) { setError(reason instanceof Error ? reason.message : '无法保存设置'); }
  }
  return <form className="llm-settings-form" onSubmit={submit}>
    <div className="settings-section-label"><KeyRound size={13} />OpenAI-compatible API</div>
    <label htmlFor="llm-base-url">Base URL</label><input id="llm-base-url" autoComplete="off" spellCheck={false} value={values.baseUrl} disabled={preview}
      onChange={(event) => setValues({ ...values, baseUrl: event.target.value })} placeholder="https://your-provider.example/v1" />
    <label htmlFor="llm-model">Model</label><input id="llm-model" autoComplete="off" spellCheck={false} value={values.model} disabled={preview}
      onChange={(event) => setValues({ ...values, model: event.target.value })} placeholder="model-name" />
    <label htmlFor="llm-api-key">API Key</label><div className="secret-input"><input id="llm-api-key" type={showKey ? 'text' : 'password'} autoComplete="off" spellCheck={false} value={values.apiKey} disabled={preview}
      onChange={(event) => setValues({ ...values, apiKey: event.target.value })} placeholder="Enter your API key" />
      <button type="button" className="icon-button" aria-label={showKey ? '隐藏 API Key' : '显示 API Key'} disabled={preview} onClick={() => setShowKey(!showKey)}>{showKey ? <EyeOff size={14} /> : <Eye size={14} />}</button></div>
    <p className="settings-secret-warning">API Key 按要求保存在 localStorage，属于明文存储，并非安全凭据库。仅在信任的本机使用；发送时通过 IPC 交给 Rust，不写入会话数据库或日志。</p>
    {preview && <p className="settings-note">前端预览不会读取或保存真实连接配置；请在 Tauri 客户端测试模型。</p>}
    {error && <p role="alert" className="settings-form-error">{error}</p>}
    <div className="settings-form-actions"><button type="button" className="button-ghost" disabled={preview} onClick={() => {
      try { clearLlmSettings(); setValues({ ...defaultLlmSettings }); setError(null); onChanged?.(); } catch (reason) { setError(reason instanceof Error ? reason.message : '无法清除设置'); }
    }}><Trash2 size={13} />清除配置</button><button type="submit" className="button-approve" disabled={preview}><Save size={13} />保存设置</button></div>
  </form>;
}
