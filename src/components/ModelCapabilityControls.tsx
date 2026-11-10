import * as Tooltip from '@radix-ui/react-tooltip';
import { BrainCircuit, Gauge } from 'lucide-react';
import { contextLabel, thinkingLabels, type ProviderId, type ThinkingLevel } from '../modelCapabilities';
import { resolveCapability } from '../modelCatalog';
import { Choice } from './ui';

export default function ModelCapabilityControls({ model, provider, reasoning, disabled, onReasoning, detailed = false, variant = 'select', context }: {
  model: string; provider: ProviderId | (string & {}); reasoning?: ThinkingLevel; disabled?: boolean;
  onReasoning: (value: ThinkingLevel) => void; detailed?: boolean; variant?: 'select' | 'cycle';
  context?: number | null;
}) {
  const capability = resolveCapability(model, provider);
  const resolvedContext = context !== undefined ? context : capability.context;
  const current = reasoning && capability.thinking.includes(reasoning) ? reasoning
    : capability.thinking.includes('medium') ? 'medium' : capability.thinking[0];
  const fixedNote = '当前接入使用模型内置推理，没有可调的 Low / Medium / High 参数；不会发送虚假的 reasoning_effort。需要调节强度可选择 o3-mini，或在 Anthropic 中选择 Claude 3.7。';
  const contextNote = resolvedContext !== null
    ? `模型参考上下文上限：${resolvedContext.toLocaleString('en-US')} tokens。网关或部署配置可能更低；此处不是本次已用 Token 数，也不会修改服务端上下文配置。`
    : `${provider === 'ollama' ? '本地模型的实际上下文由 Ollama num_ctx / 服务配置决定；已在后台自动拉取真实 context_length。' : 'OpenAI 协议的 /models 接口不返回上下文与思考元数据，此自定义模型无可验证上限；Settings→模型 可拉取模型列表确认模型 ID。'}${capability.referenceContext ? `模型参考上限 ${capability.referenceContext.toLocaleString('en-US')} tokens，不代表当前部署值。` : ''}应用不会将未知值冒充 128k。`;
  const levels = capability.thinking;
  const next = levels.length > 1 ? levels[(levels.indexOf(current) + 1) % levels.length] : undefined;
  const cycleNote = `思考强度：单击切换档位（${levels.map((value) => thinkingLabels[value]).join(' / ')}）。保存并通过 Rust 发送给当前 Provider。`;
  return <>
    {levels.length > 0 && variant === 'select' && <Choice label="思考强度" value={current} disabled={disabled}
      className="thinking-picker" onChange={(value) => onReasoning(value as ThinkingLevel)}
      options={levels.map((value) => ({ value, label: <span className="thinking-label">{thinkingLabels[value]}</span> }))} />}
    {levels.length > 0 && variant === 'cycle' && <Tooltip.Root>
      <Tooltip.Trigger asChild><button type="button" aria-label="思考强度" className="thinking-cycle" disabled={disabled || !next}
        onClick={() => next && onReasoning(next)}><BrainCircuit size={12} />{thinkingLabels[current]}</button></Tooltip.Trigger>
      <Tooltip.Portal><Tooltip.Content className="tooltip-content capability-tooltip" sideOffset={8}>{cycleNote}</Tooltip.Content></Tooltip.Portal>
    </Tooltip.Root>}
    {capability.reasoning === 'fixed' && <Tooltip.Root>
      <Tooltip.Trigger asChild><span tabIndex={0} className="thinking-fixed" aria-label="思考模式：模型内置推理" title={fixedNote}><BrainCircuit size={12} />Thinking</span></Tooltip.Trigger>
      <Tooltip.Portal><Tooltip.Content className="tooltip-content capability-tooltip" sideOffset={8}>{fixedNote}</Tooltip.Content></Tooltip.Portal>
    </Tooltip.Root>}
    <Tooltip.Root>
      <Tooltip.Trigger asChild><span tabIndex={0} className="context-limit" aria-label="上下文长度" title={contextNote}><Gauge size={12} /><span>{contextLabel(resolvedContext)}</span></span></Tooltip.Trigger>
      <Tooltip.Portal><Tooltip.Content className="tooltip-content capability-tooltip" sideOffset={8}>{contextNote}</Tooltip.Content></Tooltip.Portal>
    </Tooltip.Root>
    {detailed && <p className="capability-description">{capability.reasoning === 'fixed' ? fixedNote : capability.thinking.length > 0
      ? '思考强度会保存并通过 Rust 发送给当前 Provider，不是 UI Mock。上下文显示模型参考上限，实际以服务端为准。'
      : '当前模型 / API 未配置可调思考强度。切换为 o3-mini 等受支持的模型后，会自动显示强度选项。'}{resolvedContext === null && ` ${contextNote}`}</p>}
  </>;
}
