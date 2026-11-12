import * as Popover from '@radix-ui/react-popover';
import * as Slider from '@radix-ui/react-slider';
import { BrainCircuit, Check, ChevronDown, PlugZap } from 'lucide-react';
import { thinkingLabels, type ModelCapability, type ProviderId, type ThinkingLevel } from '../modelCapabilities';
import { getProvider } from '../customProviders';

export interface ModelPickerRow {
  id: string;
  context: number | null;
}

/// ZCode-style composer picker: one popover holds both the model list and the
/// thinking-strength row (pills for effort/budget models, static line for
/// fixed-reasoning, hidden when unsupported).
export default function ModelPicker({ model, provider, capability, thinking, rows, disabled, onModel, onThinking }: {
  model: string; provider: ProviderId | (string & {}); capability: ModelCapability; thinking: ThinkingLevel;
  rows: ModelPickerRow[]; disabled?: boolean;
  onModel: (id: string) => void; onThinking: (level: ThinkingLevel) => void;
}) {
  return <Popover.Root>
    <Popover.Trigger asChild>
      <button type="button" className="model-picker-trigger" aria-label="选择模型" disabled={disabled}>
        <BrainCircuit size={12} aria-hidden="true" /><span className="model-picker-name">{model}</span><ChevronDown size={12} />
      </button>
    </Popover.Trigger>
    <Popover.Portal>
      <Popover.Content className="model-picker" side="top" align="end" sideOffset={8} collisionPadding={12}>
        <div className="model-picker-header"><PlugZap size={12} />({getProvider(provider).name})<span className="model-picker-api">API</span></div>
        <div className="model-picker-section-label">模型</div>
        <div className="model-picker-list">
          {rows.length === 0 && <p className="model-picker-empty">暂无模型，到 Settings→模型 添加或拉取。</p>}
          {rows.map((row) => <button type="button" key={row.id} className="model-picker-row" onClick={() => onModel(row.id)}>
            <span className="model-picker-id">{row.id}</span>
            {row.context != null && <span className="model-picker-context">{row.context >= 1000000 ? `${(row.context / 1000000).toFixed(1)}M` : `${Math.round(row.context / 1000)}k`}</span>}
            {row.id === model && <Check size={13} aria-label="当前模型" />}
          </button>)}
        </div>
        {capability.reasoning === 'effort' || capability.reasoning === 'budget'
          ? <div className="model-picker-thinking">
            <div className="model-picker-slider-label">思考强度 <strong>{thinkingLabels[thinking]}</strong><span className="model-picker-kbd">Ctrl+T</span></div>
            <Slider.Root className="thinking-slider" aria-label="思考强度"
              min={0} max={capability.thinking.length - 1} step={1}
              value={[Math.max(0, capability.thinking.indexOf(thinking))]}
              onPointerDown={(event) => {
                // Own drag fallback: track pointer globally so drag works even
                // where setPointerCapture is unreliable (WebView2 popover).
                const root = event.currentTarget;
                const pick = (clientX: number) => {
                  const rect = root.getBoundingClientRect();
                  if (rect.width <= 0) return;
                  const index = Math.round(((clientX - rect.left) / rect.width) * (capability.thinking.length - 1));
                  const level = capability.thinking[Math.max(0, Math.min(capability.thinking.length - 1, index))];
                  if (level) onThinking(level);
                };
                pick(event.clientX);
                const move = (ev: MouseEvent) => pick(ev.clientX);
                const up = () => {
                  window.removeEventListener('pointermove', move);
                  window.removeEventListener('pointerup', up);
                  window.removeEventListener('mousemove', move);
                  window.removeEventListener('mouseup', up);
                };
                window.addEventListener('pointermove', move);
                window.addEventListener('pointerup', up);
                // WebView2 fallback: plain mouse events for where pointer
                // capture inside portaled popovers is unreliable.
                window.addEventListener('mousemove', move);
                window.addEventListener('mouseup', up);
              }}
              onMouseDown={(event) => {
                const root = event.currentTarget;
                const pick = (clientX: number) => {
                  const rect = root.getBoundingClientRect();
                  if (rect.width <= 0) return;
                  const index = Math.round(((clientX - rect.left) / rect.width) * (capability.thinking.length - 1));
                  const level = capability.thinking[Math.max(0, Math.min(capability.thinking.length - 1, index))];
                  if (level) onThinking(level);
                };
                const move = (ev: MouseEvent) => pick(ev.clientX);
                const up = () => { window.removeEventListener('mousemove', move); window.removeEventListener('mouseup', up); };
                window.addEventListener('mousemove', move);
                window.addEventListener('mouseup', up);
              }}
              onValueChange={([index]) => { const level = capability.thinking[index]; if (level) onThinking(level); }}>
              <Slider.Track className="thinking-slider-track">
                {capability.thinking.map((_, index) => <span key={index} className="thinking-slider-dot"
                  style={{ left: `${(index / Math.max(1, capability.thinking.length - 1)) * 100}%` }} />)}
              </Slider.Track>
              <Slider.Thumb className="thinking-slider-thumb" aria-label={`思考强度 ${thinkingLabels[thinking]}`} />
            </Slider.Root>
          </div>
          : capability.reasoning === 'fixed'
            ? <div className="model-picker-thinking"><div className="model-picker-section-label">思考强度</div>
              <p className="model-picker-fixed"><BrainCircuit size={11} />内置推理（模型固定）</p></div>
            : null}
      </Popover.Content>
    </Popover.Portal>
  </Popover.Root>;
}
