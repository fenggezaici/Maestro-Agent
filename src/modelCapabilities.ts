export type ProviderId = 'openai' | 'deepseek' | 'anthropic' | 'ollama';
export type ThinkingLevel = 'minimal' | 'low' | 'medium' | 'high' | 'xhigh' | 'max';
export interface ModelCapability { context: number | null; referenceContext?: number; thinking: ThinkingLevel[]; reasoning: 'effort' | 'budget' | 'fixed' | 'none'; vision: boolean }
export const thinkingLabels: Record<ThinkingLevel, string> = { minimal: 'Minimal', low: 'Low', medium: 'Medium', high: 'High', xhigh: 'Extra High', max: 'Max' };
export const providers = {
  openai: { name: 'OpenAI', baseUrl: 'https://api.openai.com/v1', model: 'gpt-5', models: ['gpt-5', 'gpt-5.2'] },
  deepseek: { name: 'DeepSeek', baseUrl: 'https://api.deepseek.com/v1', model: 'deepseek-chat', models: ['deepseek-chat', 'deepseek-reasoner', 'deepseek-coder'] },
  anthropic: { name: 'Anthropic', baseUrl: 'https://api.anthropic.com/v1', model: 'claude-3-7-sonnet-latest', models: ['claude-3-7-sonnet-latest'] },
  ollama: { name: 'Ollama', baseUrl: 'http://localhost:11434/v1', model: 'qwen2.5-coder', models: ['qwen2.5-coder', 'llama3.2', 'deepseek-r1'] },
} satisfies Record<ProviderId, { name: string; baseUrl: string; model: string; models: string[] }>;

// Catalog limits are model references, not discovery of a gateway's configured limits.
const catalog: Record<string, ModelCapability> = {
  'gpt-5': { context: 400000, thinking: ['low', 'medium', 'high'], reasoning: 'effort', vision: true },
  'gpt-5.2': { context: 400000, thinking: ['low', 'medium', 'high', 'xhigh'], reasoning: 'effort', vision: true },
  'deepseek-chat': { context: 128000, thinking: [], reasoning: 'none', vision: false },
  'deepseek-r1': { context: 128000, thinking: [], reasoning: 'fixed', vision: false },
  'deepseek-coder': { context: 64000, thinking: [], reasoning: 'none', vision: false },
  'deepseek-reasoner': { context: 128000, thinking: [], reasoning: 'fixed', vision: false },
  'claude-3-7-sonnet-latest': { context: 200000, thinking: ['low', 'medium', 'high'], reasoning: 'budget', vision: true },
};

export function modelCapability(model: string, provider?: ProviderId | (string & {})): ModelCapability {
  // Metadata aliases only: never rewrite the model ID sent to the provider.
  const name = (model ?? '').trim().toLowerCase();
  const aliases: Record<string, string> = {
    'deepseek-ai/deepseek-r1': 'deepseek-r1',
    'deepseek-r1:latest': 'deepseek-r1',
    'claude-3.7-sonnet': 'claude-3-7-sonnet-latest',
    'claude-3.5-sonnet': 'claude-3-5-sonnet-latest',
    'claude-3-7-sonnet': 'claude-3-7-sonnet-latest',
    'claude-3-5-sonnet': 'claude-3-5-sonnet-latest',
  };
  const normalized = aliases[name] ?? name;
  const taggedR1 = /^deepseek-r1(?::|[-/]distill-)/.test(normalized);
  const capability: ModelCapability = catalog[normalized] ?? {
    context: null, thinking: [], reasoning: taggedR1 ? 'fixed' : 'none', vision: false,
  };
  // Ollama num_ctx depends on local configuration, not the hosted model's limit.
  // Preserve reasoning identity even when the deployment context is unknown.
  if (provider === 'ollama') return { ...capability, context: null, referenceContext: capability.context ?? undefined, thinking: [], reasoning: capability.reasoning === 'fixed' ? 'fixed' : 'none' };
  if ((capability.reasoning === 'budget' && provider && provider !== 'anthropic')
    || (capability.reasoning === 'effort' && provider && provider !== 'openai')) {
    return { ...capability, thinking: [], reasoning: 'none' };
  }
  return capability;
}

export function contextLabel(value: number | null) { return value === null ? 'Context · 服务端配置' : value >= 1000000 ? `${(value / 1000000).toFixed(1)}M context` : `${Math.round(value / 1000)}k context`; }
