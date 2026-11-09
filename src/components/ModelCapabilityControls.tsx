import * as Tooltip from '@radix-ui/react-tooltip';
import { BrainCircuit, Gauge } from 'lucide-react';
import { contextLabel, modelCapability, thinkingLabels, type ProviderId, type ThinkingLevel } from '../modelCapabilities';
import { Choice } from './ui';

export default function ModelCapabilityControls({ model, provider, reasoning, disabled, onReasoning, detailed = false }: {
  model: string; provider: ProviderId; reasoning?: ThinkingLevel; disabled?: boolean;
  onReasoning: (value: ThinkingLevel) => void; detailed?: boolean;
}) {
  const capability = modelCapability(model, provider);
  const current = reasoning && capability.thinking.includes(reasoning) ? reasoning
    : capability.thinking.includes('medium') ? 'medium' : capability.thinking[0];
  const fixedNote = '当前接入使用模型内置推理，没有可调的 Low / Medium / High 参数；不会发送虚假的 reasoning_effort。需要调节强度可选择 o3-mini，或在 Anthropic 中选择 Claude 3.7。';
  const contextNote = capability.context !== null
    ? `模型参考上下文上限：${capability.context.toLocaleString('en-US')} tokens。网关或部署配置可能更低；此处不是本次已用 Token 数，也不会修改服务端上下文配置。`
    : `${provider === 'ollama' ? '本地模型的实际上下文由 Ollama num_ctx / 服务配置决定。' : '此自定义模型未提供可验证的上下文上限。'}${capability.referenceContext ? `模型参考上限 ${capability.referenceContext.toLocaleString('en-US')} tokens，不代表当前部署值。` : ''}应用不会将未知值冒充 128k。`;
  return <>
    {capability.thinking.length > 0 && <Choice label="思考强度" value={current} disabled={disabled}
      className="thinking-picker" onChange={(value) => onReasoning(value as ThinkingLevel)}
      options={capability.thinking.map((value) => ({ value, label: <span className="thinking-label">{thinkingLabels[value]}</span> }))} />}
    {capability.reasoning === 'fixed' && <Tooltip.Root>
      <Tooltip.Trigger asChild><span tabIndex={0} className="thinking-fixed" aria-label="思考模式：模型内置推理" title={fixedNote}><BrainCircuit size={12} />Thinking</span></Tooltip.Trigger>
      <Tooltip.Portal><Tooltip.Content className="tooltip-content capability-tooltip" sideOffset={8}>{fixedNote}</Tooltip.Content></Tooltip.Portal>
    </Tooltip.Root>}
    <Tooltip.Root>
      <Tooltip.Trigger asChild><span tabIndex={0} className="context-limit" aria-label="上下文长度" title={contextNote}><Gauge size={12} /><span>{contextLabel(capability.context)}</span></span></Tooltip.Trigger>
      <Tooltip.Portal><Tooltip.Content className="tooltip-content capability-tooltip" sideOffset={8}>{contextNote}</Tooltip.Content></Tooltip.Portal>
    </Tooltip.Root>
    {detailed && <p className="capability-description">{capability.reasoning === 'fixed' ? fixedNote : capability.thinking.length > 0
      ? '思考强度会保存并通过 Rust 发送给当前 Provider，不是 UI Mock。上下文显示模型参考上限，实际以服务端为准。'
      : '当前模型 / API 未配置可调思考强度。切换为 o3-mini 等受支持的模型后，会自动显示强度选项。'}{capability.context === null && ` ${contextNote}`}</p>}
  </>;
}
