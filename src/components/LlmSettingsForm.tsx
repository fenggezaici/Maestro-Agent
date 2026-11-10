import { useEffect, useState, type FormEvent } from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import * as Switch from '@radix-ui/react-switch';
import { Eye, EyeOff, KeyRound, Pencil, Plus, RefreshCw, Save, Trash2, X } from 'lucide-react';
import { clearLlmSettings, readLlmSettings, saveLlmSettings, type LlmSettings } from '../settings';
import { contextLabel, providers, type ProviderId } from '../modelCapabilities';
import { fetchModels, readFetchedModels, saveFetchedModels, type FetchedModel } from '../modelFetch';
import { clearRemoved, mergeFetched, readProviderModels, removeProviderModel, saveProviderModels, type ProviderModel } from '../providerModels';
import type { ReasoningKind } from '../modelCatalog';
import { Choice } from './ui';
import ModelCapabilityControls from './ModelCapabilityControls';

const REASONING_OPTIONS: { value: ReasoningKind | 'auto'; label: string }[] = [
  { value: 'auto', label: '跟随自动（目录 / 静态表）' },
  { value: 'none', label: '无思考参数' },
  { value: 'fixed', label: '固定推理（模型内置）' },
  { value: 'effort', label: '可调强度 reasoning_effort' },
  { value: 'budget', label: '预算式 thinking budget' },
];

export default function LlmSettingsForm({ preview, onSaved, onChanged, provider = 'openai' }: { preview: boolean; onSaved: () => void; onChanged?: () => void; provider?: ProviderId }) {
  const load = (): LlmSettings => preview ? { apiKey: '', baseUrl: providers[provider].baseUrl, model: providers[provider].model, provider } : readLlmSettings();
  const [values, setValues] = useState(load);
  useEffect(() => { setValues(load()); }, [provider, preview]);
  const [showKey, setShowKey] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [models, setModels] = useState<ProviderModel[]>(() => readProviderModels(provider));
  const [fetching, setFetching] = useState(false);
  const [editing, setEditing] = useState<ProviderModel | 'new' | null>(null);
  useEffect(() => { setModels(readProviderModels(provider)); }, [provider, preview]);

  function persist(next: ProviderModel[]) { setModels(next); if (!preview) saveProviderModels(provider, next); }

  async function pull() {
    if (preview || fetching) return;
    setFetching(true); setError(null);
    try {
      const list = await fetchModels(provider, values.baseUrl, values.apiKey);
      saveFetchedModels(provider, list);
      persist(mergeFetched(provider, list));
    } catch (reason) { setError(reason instanceof Error ? reason.message : '无法拉取模型列表'); }
    finally { setFetching(false); }
  }

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (preview) return;
    try { saveLlmSettings(values); setError(null); onSaved(); void pull(); } catch (reason) { setError(reason instanceof Error ? reason.message : '无法保存设置'); }
  }

  return <form className="llm-settings-form" onSubmit={submit}>
    <div className="settings-section-label"><KeyRound size={13} />{providers[provider].name} / {provider === 'anthropic' ? 'Messages API' : 'OpenAI-compatible API'}</div>
    <label htmlFor="llm-base-url">Base URL</label><input id="llm-base-url" autoComplete="off" spellCheck={false} value={values.baseUrl} disabled={preview}
      onChange={(event) => setValues({ ...values, baseUrl: event.target.value })} placeholder="https://your-provider.example/v1" />
    <label htmlFor="llm-api-key">API Key</label><div className="secret-input"><input id="llm-api-key" type={showKey ? 'text' : 'password'} autoComplete="off" spellCheck={false} value={values.apiKey} disabled={preview}
      onChange={(event) => setValues({ ...values, apiKey: event.target.value })} placeholder="Enter your API key" />
      <button type="button" className="icon-button" aria-label={showKey ? '隐藏 API Key' : '显示 API Key'} disabled={preview} onClick={() => setShowKey(!showKey)}>{showKey ? <EyeOff size={14} /> : <Eye size={14} />}</button></div>

    <div className="model-list-heading"><span>模型列表</span>
      <button type="button" className="icon-button" aria-label="拉取模型列表" title="从 Provider 拉取真实模型列表" disabled={preview || fetching} onClick={() => void pull()}><RefreshCw size={13} /></button>
      <button type="button" className="button-ghost" disabled={preview} onClick={() => setEditing('new')}><Plus size={12} />添加模型</button>
    </div>
    {models.length === 0 && <p className="settings-note">暂无模型。手填下方当前模型即可使用；也可拉取或手动添加。</p>}
    <div className="model-list" role="list">
      {models.map((item) => <div className="model-list-row" role="listitem" key={item.id}>
        <button type="button" className="model-list-id" title="设为当前模型"
          aria-label={`设为当前模型 ${item.id}`} aria-current={values.model === item.id}
          onClick={() => setValues({ ...values, model: item.id, reasoning: undefined })}>{item.id}</button>
        {item.context !== undefined && <span className="model-list-context">{contextLabel(item.context)}</span>}
        <button type="button" className="icon-button" aria-label={`编辑 ${item.id}`} disabled={preview} onClick={() => setEditing(item)}><Pencil size={12} /></button>
        <button type="button" className="icon-button" aria-label={`删除 ${item.id}`} disabled={preview}
          onClick={() => { if (!preview) removeProviderModel(provider, item.id); persist(models.filter((row) => row.id !== item.id)); }}><Trash2 size={12} /></button>
        <Switch.Root className="settings-switch" aria-label={`启用 ${item.id}`} checked={item.enabled} disabled={preview}
          onCheckedChange={(enabled) => persist(models.map((row) => row.id === item.id ? { ...row, enabled } : row))}><Switch.Thumb /></Switch.Root>
      </div>)}
    </div>

    <label htmlFor="llm-model">Model</label><input id="llm-model" autoComplete="off" spellCheck={false} value={values.model}
      onChange={(event) => setValues({ ...values, model: event.target.value, reasoning: undefined })} placeholder="model-name" />
    <div className="model-capability-summary" role="group" aria-label="模型能力">
      <strong>模型能力 · Context & Thinking</strong>
      <div className="model-capability-options"><ModelCapabilityControls model={values.model} provider={values.provider ?? provider} reasoning={values.reasoning}
        onReasoning={(reasoning) => setValues({ ...values, reasoning })} detailed /></div>
    </div>
    <p className="settings-secret-warning">API Key 按要求保存在 localStorage，属于明文存储，并非安全凭据库。仅在信任的本机使用；发送时通过 IPC 交给 Rust，不写入会话数据库或日志。</p>
    {preview && <p className="settings-note">前端预览不会读取或保存真实连接配置；请在 Tauri 客户端测试模型。</p>}
    {error && <p role="alert" className="settings-form-error">{error}</p>}
    <div className="settings-form-actions"><button type="button" className="button-ghost" disabled={preview} onClick={() => {
      try { clearLlmSettings(); setValues(load()); setError(null); onChanged?.(); } catch (reason) { setError(reason instanceof Error ? reason.message : '无法清除设置'); }
    }}><Trash2 size={13} />清除配置</button><button type="submit" className="button-approve" disabled={preview}><Save size={13} />保存设置</button></div>
    <ModelEditDialog editing={editing} onClose={() => setEditing(null)} onSave={(row) => {
      if (!preview) clearRemoved(provider, row.id);
      const exists = models.some((item) => item.id === row.id);
      persist(exists ? models.map((item) => item.id === row.id ? row : item) : [...models, row]);
      setEditing(null);
    }} />
  </form>;
}

/// ZCode-style 编辑模型配置 dialog: model id, context window, max output,
/// advanced reasoning kind.
function ModelEditDialog({ editing, onClose, onSave }: {
  editing: ProviderModel | 'new' | null; onClose: () => void; onSave: (row: ProviderModel) => void;
}) {
  const isNew = editing === 'new';
  const base = editing && editing !== 'new' ? editing : { id: '', enabled: true } as ProviderModel;
  const [draft, setDraft] = useState<ProviderModel>(base);
  useEffect(() => { setDraft(editing && editing !== 'new' ? editing : { id: '', enabled: true }); }, [editing]);
  const open = editing !== null;
  return <Dialog.Root open={open} onOpenChange={(value) => !value && onClose()}>
    <Dialog.Portal><Dialog.Overlay className="dialog-overlay" />
      <Dialog.Content className="project-dialog" aria-describedby={undefined}>
        <Dialog.Title asChild><h3>{isNew ? '添加模型' : '编辑模型配置'}</h3></Dialog.Title>
        <form onSubmit={(event) => { event.preventDefault(); if (draft.id.trim()) onSave({ ...draft, id: draft.id.trim() }); }}>
          <label htmlFor="model-edit-id">模型 ID</label>
          <input id="model-edit-id" autoComplete="off" spellCheck={false} autoFocus value={draft.id} maxLength={128}
            onChange={(event) => setDraft({ ...draft, id: event.target.value })} placeholder="deepseek-v4.1-flash" />
          <label htmlFor="model-edit-context">上下文窗口</label>
          <input id="model-edit-context" autoComplete="off" inputMode="numeric" spellCheck={false}
            value={draft.context ?? ''} placeholder="如 1000000"
            onChange={(event) => { const digits = event.target.value.replace(/[^\d]/g, ''); setDraft({ ...draft, context: digits ? Number(digits) : undefined }); }} />
          <label htmlFor="model-edit-output">最大输出 Token</label>
          <input id="model-edit-output" autoComplete="off" inputMode="numeric" spellCheck={false}
            value={draft.output ?? ''} placeholder="如 384000"
            onChange={(event) => { const digits = event.target.value.replace(/[^\d]/g, ''); setDraft({ ...draft, output: digits ? Number(digits) : undefined }); }} />
          <Choice label="思考类型" value={draft.reasoning ?? 'auto'} onChange={(value) => setDraft({ ...draft, reasoning: value === 'auto' ? undefined : value as ReasoningKind })} options={REASONING_OPTIONS} />
          <div className="dialog-actions">
            <Dialog.Close asChild><button type="button" className="button-ghost">取消</button></Dialog.Close>
            <button type="submit" className="button-approve" disabled={!draft.id.trim()}>保存</button>
          </div>
        </form>
        <Dialog.Close asChild><button type="button" className="icon-button dialog-close" aria-label="关闭"><X size={15} /></button></Dialog.Close>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}
