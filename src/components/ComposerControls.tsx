import { useEffect, useState } from 'react';
import { BrainCircuit } from 'lucide-react';
import { Choice } from './ui';
import { readLlmSettings, saveLlmSettings } from '../settings';
import type { UiPreferences } from '../uiPreferences';

export function ContextMeter({ limit }: { limit: number }) {
  const used = 2400;
  return <span className="context-meter" aria-label="上下文使用量（Mock）">
    <span>Tokens: 2.4k / {limit === 8192 ? '8k' : limit === 32768 ? '32k' : '128k'} <small>Mock</small></span>
    <meter aria-label="Mock context usage" min={0} max={limit} value={used} />
  </span>;
}

export default function ComposerControls({ preview, disabled, preferences, onPreferences, settingsVersion, onError }: {
  preview: boolean; disabled: boolean; preferences: UiPreferences; onPreferences: (value: UiPreferences) => void;
  settingsVersion: number; onError: (message: string) => void;
}) {
  const [model, setModel] = useState(() => preview ? preferences.previewModel : readLlmSettings().model);
  useEffect(() => { setModel(preview ? preferences.previewModel : readLlmSettings().model); }, [preview, preferences.previewModel, settingsVersion]);
  const options = [...new Set([model, 'deepseek-chat', 'gpt-4o-mini', 'qwen2.5-coder'])];
  return <div className="composer-model-controls">
    <Choice label="选择模型" value={model} disabled={!preview && disabled} className="composer-model-picker"
      options={options.map((value) => ({ value, label: <span className="model-label"><BrainCircuit size={12} />{value}</span> }))}
      onChange={(value) => {
        if (preview) { setModel(value); onPreferences({ ...preferences, previewModel: value }); return; }
        try { saveLlmSettings({ ...readLlmSettings(), model: value }); setModel(value); } catch (error) { onError(error instanceof Error ? error.message : '无法选择模型'); }
      }} />
    <Choice label="Mock 上下文长度" className="composer-context-picker" value={String(preferences.contextWindow)}
      options={[{ value: '8192', label: '8k tokens' }, { value: '32768', label: '32k tokens' }, { value: '128000', label: '128k tokens' }]}
      onChange={(value) => onPreferences({ ...preferences, contextWindow: Number(value) as UiPreferences['contextWindow'] })} />
    <ContextMeter limit={preferences.contextWindow} />
  </div>;
}
