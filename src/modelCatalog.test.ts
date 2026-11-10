import { beforeEach, describe, expect, it } from 'vitest';
import { catalogLookup, capabilityOverrides, loadCatalog, readCatalog, resolveCapability, saveCapabilityOverride } from './modelCatalog';

const catalog = [
  { id: 'deepseek-chat', context: 128000, reasoning: false },
  { id: 'deepseek-reasoner', context: 128000, reasoning: true },
  { id: 'gpt-5.2', context: 400000, reasoning: true },
  { id: 'claude-sonnet-4-5', context: 200000, reasoning: true },
];

beforeEach(() => {
  localStorage.clear();
  localStorage.setItem('maestro.models.catalog', JSON.stringify({ at: Date.now(), models: catalog }));
});

describe('catalogLookup', () => {
  it('matches exact ids and strips provider prefixes', () => {
    expect(catalogLookup('deepseek-reasoner')?.context).toBe(128000);
    expect(catalogLookup('deepseek/deepseek-reasoner')?.id).toBe('deepseek-reasoner');
    expect(catalogLookup('unknown-xyz')).toBeNull();
  });
});

describe('resolveCapability', () => {
  it('uses models.dev context + provider-aware reasoning for catalog models', () => {
    expect(resolveCapability('deepseek-reasoner', 'deepseek')).toMatchObject({ context: 128000, reasoning: 'fixed' });
    expect(resolveCapability('gpt-5.2', 'openai')).toMatchObject({ context: 400000, reasoning: 'effort', thinking: ['low', 'medium', 'high', 'xhigh'] });
    expect(resolveCapability('claude-sonnet-4-5', 'anthropic').reasoning).toBe('budget');
  });

  it('keeps honest unknown for models nobody knows', () => {
    const resolved = resolveCapability('DeepSeek-V4.1-Flash-1017', 'deepseek');
    expect(resolved.context).toBeNull();
    expect(resolved.reasoning).toBe('none');
  });

  it('user override wins over catalog and static table', () => {
    saveCapabilityOverride('deepseek', 'DeepSeek-V4.1-Flash-1017', { context: 64000, reasoning: 'effort' });
    const resolved = resolveCapability('DeepSeek-V4.1-Flash-1017', 'deepseek');
    expect(resolved).toMatchObject({ context: 64000, reasoning: 'effort', thinking: ['low', 'medium', 'high'] });
    saveCapabilityOverride('deepseek', 'DeepSeek-V4.1-Flash-1017', { context: 64000, reasoning: 'effort' });
    expect(capabilityOverrides('deepseek')['DeepSeek-V4.1-Flash-1017']).toMatchObject({ context: 64000 });
    saveCapabilityOverride('deepseek', 'DeepSeek-V4.1-Flash-1017', {});
    expect(capabilityOverrides('deepseek')['DeepSeek-V4.1-Flash-1017']).toBeUndefined();
  });

  it('loadCatalog keeps the cache when invoke is unavailable', async () => {
    const models = await loadCatalog();
    expect(models.length).toBeGreaterThan(0);
    expect(readCatalog()[0].id).toBe('deepseek-chat');
  });
});
