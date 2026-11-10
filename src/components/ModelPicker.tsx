import * as Popover from '@radix-ui/react-popover';
import { BrainCircuit, Check, ChevronDown, PlugZap } from 'lucide-react';
import { thinkingLabels, type ModelCapability, type ProviderId, type ThinkingLevel } from '../modelCapabilities';
import { providers } from '../modelCapabilities';

export interface ModelPickerRow {
  id: string;
  context: number | null;
}

/// ZCode-style composer picker: one popover holds both the model list and the
/// thinking-strength row (pills for effort/budget models, static line for
/// fixed-reasoning, hidden when unsupported).
export default function ModelPicker({ model, provider, capability, thinking, rows, disabled, onModel, onThinking }: {
  model: string; provider: ProviderId; capability: ModelCapability; thinking: ThinkingLevel;
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
        <div className="model-picker-header"><PlugZap size={12} />{providers[provider].name}<span className="model-picker-api">API</span></div>
        <div className="model-picker-section-label">模型</div>
        <div className="model-picker-list">
          {rows.length === 0 && <p className="model-picker-empty">暂无模型，到 Settings→模型 添加或拉取。</p>}
          {rows.map((row) => <button type="button" key={row.id} className="model-picker-row" onClick={() => onModel(row.id)}>
            <span className="model-picker-id">{row.id}</span>
            {row.id === model && <Check size={13} aria-label="当前模型" />}
          </button>)}
        </div>
        {capability.reasoning === 'effort' || capability.reasoning === 'budget'
          ? <div className="model-picker-thinking">
            <div className="model-picker-slider-label">思考强度 <strong>{thinkingLabels[thinking]}</strong><span className="model-picker-kbd">Ctrl+T</span></div>
            <div className="thinking-slider" role="slider" tabIndex={0} aria-label="思考强度"
              aria-valuemin={0} aria-valuemax={capability.thinking.length - 1}
              aria-valuenow={Math.max(0, capability.thinking.indexOf(thinking))}
              onKeyDown={(event) => {
                const index = Math.max(0, capability.thinking.indexOf(thinking));
                const next = event.key === 'ArrowRight' || event.key === 'ArrowUp' ? Math.min(capability.thinking.length - 1, index + 1)
                  : event.key === 'ArrowLeft' || event.key === 'ArrowDown' ? Math.max(0, index - 1)
                  : event.key === 'End' ? capability.thinking.length - 1
                  : event.key === 'Home' ? 0 : -1;
                if (next >= 0) { event.preventDefault(); onThinking(capability.thinking[next]); }
              }}>
              {capability.thinking.map((level, index) => <button type="button" key={level}
                className="thinking-slider-segment" aria-label={thinkingLabels[level]}
                onClick={() => onThinking(level)}>
                <span className={`thinking-slider-fill ${index <= capability.thinking.indexOf(thinking) ? 'filled' : ''}`} /></button>)}
              <span className="thinking-slider-thumb"
                style={{ left: `${(Math.max(0, capability.thinking.indexOf(thinking)) / Math.max(1, capability.thinking.length - 1)) * 100}%` }} />
            </div>
          </div>
          : capability.reasoning === 'fixed'
            ? <div className="model-picker-thinking"><div className="model-picker-section-label">思考强度</div>
              <p className="model-picker-fixed"><BrainCircuit size={11} />内置推理（模型固定）</p></div>
            : null}
      </Popover.Content>
    </Popover.Portal>
  </Popover.Root>;
}
