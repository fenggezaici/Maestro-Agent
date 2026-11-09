import { useEffect, useState } from 'react';
import { BrainCircuit } from 'lucide-react';
import { Choice } from './ui';
import { readLlmSettings, saveLlmSettings } from '../settings';
import type { UiPreferences } from '../uiPreferences';
import { contextLabel, modelCapability, providers, type ProviderId, type ThinkingLevel } from '../modelCapabilities';
import ModelCapabilityControls from './ModelCapabilityControls';

const compactContext = (value: number | null) => value === null ? null : contextLabel(value).replace(' context', '');

export default function ComposerControls({ preview, disabled, preferences, onPreferences, settingsVersion, onError, onCapability }: {
  preview: boolean; disabled: boolean; preferences: UiPreferences; onPreferences: (value: UiPreferences) => void;
  settingsVersion: number; onError: (message: string) => void;
  onCapability: (model: string, provider: ProviderId, context: number | null) => void;
}) {
  const [model, setModel] = useState(() => preview ? preferences.previewModel : readLlmSettings().model);
  const [provider, setProvider] = useState<ProviderId>(() => preview ? preferences.provider : readLlmSettings().provider ?? 'openai');
  const [thinking, setThinking] = useState<ThinkingLevel>('medium');
  useEffect(() => {
    const value = preview ? null : readLlmSettings();
    setModel(value?.model ?? preferences.previewModel);
    setProvider(value?.provider ?? (preview ? preferences.provider : 'openai'));
    setThinking(value?.reasoning ?? 'medium');
  }, [preview, preferences.previewModel, preferences.provider, settingsVersion]);
  const capability = modelCapability(model, provider);
  useEffect(() => { onCapability(model, provider, capability.context); }, [model, provider, capability.context, onCapability]);
  const options = [...new Set([model, ...providers[provider].models])];
  return <div className="composer-model-controls">
    <Choice label="选择模型" value={model} disabled={!preview && disabled} className="composer-model-picker"
      options={options.map((value) => {
        const context = compactContext(modelCapability(value, provider).context);
        return { value, label: <span className="model-option"><span className="model-label"><BrainCircuit size={12} />{value}</span>{context && <small>{context}</small>}</span> };
      })}
      onChange={(value) => {
        if (preview) { setModel(value); onPreferences({ ...preferences, previewModel: value }); return; }
        try {
          const supported = modelCapability(value, provider).thinking;
          const reasoning = supported.includes('medium') ? 'medium' : supported[0];
          saveLlmSettings({ ...readLlmSettings(), model: value, reasoning }); setModel(value); setThinking(reasoning ?? 'medium');
        } catch (error) { onError(error instanceof Error ? error.message : '无法选择模型'); }
      }} />
    <ModelCapabilityControls model={model} provider={provider} reasoning={thinking} disabled={!preview && disabled} onReasoning={(value) => {
      if (preview) { setThinking(value); return; }
      try { const saved = saveLlmSettings({ ...readLlmSettings(), reasoning: value }); setThinking(saved.reasoning ?? 'medium'); }
      catch (error) { onError(error instanceof Error ? error.message : 'Cannot save thinking mode'); }
    }} />
  </div>;
}
