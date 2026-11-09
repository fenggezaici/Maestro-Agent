import { expect, it } from 'vitest';
import { contextLabel, modelCapability, type ProviderId } from './modelCapabilities';
import { validateLlmSettings } from './settings';

it.each(['deepseek-r1', ' DeepSeek-R1 ', 'deepseek-ai/DeepSeek-R1'])('recognizes %s as built-in reasoning without inventing adjustable effort', (model) => {
  expect(modelCapability(model, 'openai')).toMatchObject({ context: 128000, reasoning: 'fixed', thinking: [] });
  const saved = validateLlmSettings({ model, apiKey: 'key', baseUrl: 'https://gateway.example/v1', reasoning: 'high' });
  expect(saved.model).toBe(model.trim());
  expect(saved.reasoning).toBeUndefined();
});

it.each([
  ['gpt-5', 'openai', 400000, ['low', 'medium', 'high']],
  ['gpt-5.2', 'openai', 400000, ['low', 'medium', 'high', 'xhigh']],
  ['claude-3.7-sonnet', 'anthropic', 200000, ['low', 'medium', 'high']],
  ['claude-3-7-sonnet', 'anthropic', 200000, ['low', 'medium', 'high']],
] as const)('maps %s strengths on its supported provider', (model, provider, context, thinking) => {
  expect(modelCapability(model, provider)).toMatchObject({ context, thinking });
});

it.each(['ollama', 'deepseek', 'anthropic'] as ProviderId[])('does not expose OpenAI effort parameters through %s', (provider) => {
  expect(modelCapability('gpt-5', provider).thinking).toEqual([]);
});

it('preserves local R1 reasoning identity without presenting model context as the configured num_ctx', () => {
  expect(modelCapability('deepseek-r1', 'ollama')).toMatchObject({ context: null, referenceContext: 128000, reasoning: 'fixed', thinking: [] });
  expect(modelCapability('deepseek-r1:8b', 'ollama')).toMatchObject({ context: null, reasoning: 'fixed', thinking: [] });
  expect(modelCapability('DeepSeek-R1-Distill-Qwen-7B', 'openai')).toMatchObject({ context: null, reasoning: 'fixed' });
  expect(modelCapability('my-custom-model', 'openai')).toMatchObject({ context: null, reasoning: 'none', thinking: [] });
  expect(contextLabel(null)).not.toContain('Unknown');
  expect(contextLabel(128000)).toBe('128k context');
});
