import { useEffect, useState } from 'react';
import { Gauge } from 'lucide-react';
import { readLlmSettings, saveLlmSettings } from '../settings';
import type { UiPreferences } from '../uiPreferences';
import { contextLabel, type ProviderId, type ThinkingLevel } from '../modelCapabilities';
import { fetchedStale, fetchModels, saveFetchedModels } from '../modelFetch';
import { loadCatalog, resolveCapability } from '../modelCatalog';
import { mergeFetched, readProviderModels, saveProviderModels } from '../providerModels';
import ModelPicker, { type ModelPickerRow } from './ModelPicker';

export default function ComposerControls({ preview, disabled, preferences, onPreferences, settingsVersion, onError, onCapability }: {
  preview: boolean; disabled: boolean; preferences: UiPreferences; onPreferences: (value: UiPreferences) => void;
  settingsVersion: number; onError: (message: string) => void;
  onCapability: (model: string, provider: ProviderId, context: number | null) => void;
}) {
  const [model, setModel] = useState(() => preview ? preferences.previewModel : readLlmSettings().model);
  const [provider, setProvider] = useState<ProviderId>(() => preview ? preferences.provider : readLlmSettings().provider ?? 'openai');
  const [thinking, setThinking] = useState<ThinkingLevel>('medium');
  const [refresh, setRefresh] = useState(0);
  useEffect(() => {
    const value = preview ? null : readLlmSettings();
    const next = value?.model ?? preferences.previewModel;
    setModel(next);
    setProvider(value?.provider ?? (preview ? preferences.provider : 'openai'));
    setThinking(value?.reasoning ?? 'medium');
  }, [preview, preferences.previewModel, preferences.provider, settingsVersion]);
  const capability = resolveCapability(model, provider);
  const context = capability.context;
  useEffect(() => { onCapability(model, provider, context); }, [model, provider, context, onCapability]);

  // Refresh the public models.dev catalog in the background (deferred so it
  // never races initial IPC setup); feeds context/reasoning for known models.
  useEffect(() => {
    if (preview) return;
    const timer = setTimeout(() => void loadCatalog(), 1500);
    return () => clearTimeout(timer);
  }, [preview]);

  // Ctrl+T cycles thinking levels for adjustable models (ZCode parity).
  useEffect(() => {
    if (preview) return;
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== 't') return;
      const tag = (event.target as HTMLElement)?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || (event.target as HTMLElement)?.isContentEditable) return;
      const levels = resolveCapability(model, provider).thinking;
      if (!levels.length) return;
      const next = levels[(levels.indexOf(thinking) + 1) % levels.length];
      try { const saved = saveLlmSettings({ ...readLlmSettings(), reasoning: next }); setThinking(saved.reasoning ?? 'medium'); } catch { /* storage */ }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [preview, model, provider, thinking]);

  // Silently refresh the provider's real model list when credentials exist and
  // the cache is stale — merged into the provider model list afterwards.
  useEffect(() => {
    if (preview || !fetchedStale(provider)) return;
    const settings = readLlmSettings();
    if (!settings.baseUrl?.trim() || (provider !== 'ollama' && !settings.apiKey?.trim())) return;
    const timer = setTimeout(() => {
      fetchModels(provider, settings.baseUrl, settings.apiKey ?? '')
        .then((models) => {
          saveFetchedModels(provider, models);
          saveProviderModels(provider, mergeFetched(provider, models));
          setRefresh((value) => value + 1);
        })
        .catch(() => { /* background refresh must not interrupt typing */ });
    }, 1500);
    return () => clearTimeout(timer);
  }, [preview, provider, settingsVersion]);

  const rows: ModelPickerRow[] = (() => {
    void refresh;
    // Only the enabled provider model list + the current selection: rows
    // removed/disabled in Settings disappear here too.
    const ids = new Set<string>([model]);
    for (const item of readProviderModels(provider)) if (item.enabled) ids.add(item.id);
    return [...ids].map((id) => ({ id, context: resolveCapability(id, provider).context }));
  })();

  function choose(id: string) {
    if (id === model) return;
    if (preview) { setModel(id); onPreferences({ ...preferences, previewModel: id }); return; }
    try {
      const supported = resolveCapability(id, provider).thinking;
      const reasoning = supported.includes('medium') ? 'medium' : supported[0];
      saveLlmSettings({ ...readLlmSettings(), model: id, reasoning });
      setModel(id);
      setThinking(reasoning ?? 'medium');
    } catch (error) { onError(error instanceof Error ? error.message : '无法选择模型'); }
  }

  const contextNote = context !== null
    ? `模型参考上下文上限：${context.toLocaleString('en-US')} tokens。网关或部署配置可能更低；此处不是本次已用 Token 数。`
    : provider === 'ollama'
      ? '本地模型的实际上下文由 Ollama num_ctx / 服务配置决定；后台自动拉取 /api/show 的真实 context_length。'
      : 'OpenAI 协议的 /models 不返回上下文与思考元数据；Settings→模型 可在模型列表中手动声明。';

  return <div className="composer-model-controls">
    <ModelPicker model={model} provider={provider} capability={capability} thinking={thinking} rows={rows}
      disabled={!preview && disabled}
      onModel={choose}
      onThinking={(value) => {
        if (preview) { setThinking(value); return; }
        try { const saved = saveLlmSettings({ ...readLlmSettings(), reasoning: value }); setThinking(saved.reasoning ?? 'medium'); }
        catch (error) { onError(error instanceof Error ? error.message : 'Cannot save thinking mode'); }
      }} />
    <span tabIndex={0} className="context-limit" aria-label="上下文长度" title={contextNote}><Gauge size={12} /><span>{contextLabel(context)}</span></span>
  </div>;
}
