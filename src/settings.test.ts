import { beforeEach, expect, it, vi } from 'vitest';
import {
  defaultLlmSettings,
  hasProviderCredential,
  LLM_PROFILES_KEY,
  LLM_SETTINGS_KEY,
  legacyCredentials,
  readLlmSettings,
  saveLlmSettings,
  selectLlmProvider,
  setCredentialStatus,
  stripLegacyCredentials,
  validateLlmSettings,
} from './settings';

beforeEach(() => {
  localStorage.clear();
  setCredentialStatus([]);
});

it('persists validated connection settings and never writes the key', () => {
  const settings = { apiKey: 'test-key', baseUrl: 'https://example.test/v1', model: 'test-model' };
  saveLlmSettings(settings);
  expect(readLlmSettings()).toEqual({ ...settings, apiKey: '' });
  expect(localStorage.getItem(LLM_SETTINGS_KEY)).not.toContain('test-key');
  localStorage.setItem(LLM_SETTINGS_KEY, '{invalid');
  expect(readLlmSettings()).toEqual(defaultLlmSettings);
  expect(() => validateLlmSettings({ ...settings, apiKey: '' })).toThrow();
  expect(() => validateLlmSettings({ ...settings, baseUrl: 'file:///tmp/model' })).toThrow();
  expect(() => validateLlmSettings({ ...settings, baseUrl: 'https://user:password@example.test/v1' })).toThrow();
});

it('requires HTTPS for remote endpoints but keeps the local exception', () => {
  const base = { apiKey: 'test-key', model: 'test-model' };
  expect(() => validateLlmSettings({ ...base, baseUrl: 'http://remote.example.com/v1' })).toThrow('HTTPS');
  expect(validateLlmSettings({ ...base, baseUrl: 'http://localhost:11434/v1' }).baseUrl).toBe('http://localhost:11434/v1');
  expect(validateLlmSettings({ ...base, baseUrl: 'http://127.0.0.1:1234/v1' }).baseUrl).toBe('http://127.0.0.1:1234/v1');
  expect(validateLlmSettings({ ...base, baseUrl: 'https://remote.example.com/v1' }).baseUrl).toBe('https://remote.example.com/v1');
});

it('reports unavailable localStorage instead of pretending a save succeeded', () => {
  const write = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('blocked'); });
  expect(() => saveLlmSettings({ apiKey: 'test-key', baseUrl: 'http://localhost:1234/v1', model: 'test-model' })).toThrow('浏览器存储不可用');
  write.mockRestore();
});

it('migrates legacy settings without copying keys between independent Providers', () => {
  const legacy = { apiKey: 'original-key', baseUrl: 'https://custom.example/v1', model: 'custom-model' };
  localStorage.setItem(LLM_SETTINGS_KEY, JSON.stringify(legacy));
  const deepseek = selectLlmProvider('deepseek');
  expect(deepseek).toMatchObject({ apiKey: '', baseUrl: 'https://api.deepseek.com/v1', model: 'deepseek-chat' });
  saveLlmSettings({ ...deepseek, apiKey: 'deepseek-key' });
  expect(selectLlmProvider('openai')).toMatchObject(legacy);
  expect(selectLlmProvider('deepseek')).toMatchObject({ model: 'deepseek-chat', apiKey: '' });
  const ollama = selectLlmProvider('ollama');
  expect(() => saveLlmSettings(ollama)).not.toThrow();
  expect(readLlmSettings()).toMatchObject({ provider: 'ollama', apiKey: '', baseUrl: 'http://localhost:11434/v1' });
});

it('exposes legacy keys only for the one-time migration and strips them after', () => {
  localStorage.setItem(LLM_SETTINGS_KEY, JSON.stringify({ apiKey: 'legacy-openai', baseUrl: 'https://example.test/v1', model: 'm' }));
  localStorage.setItem(LLM_PROFILES_KEY, JSON.stringify({
    active: 'deepseek',
    profiles: { deepseek: { apiKey: 'legacy-deepseek', baseUrl: 'https://api.deepseek.com/v1', model: 'deepseek-chat' } },
  }));
  expect(legacyCredentials()).toEqual([
    { provider: 'deepseek', apiKey: 'legacy-deepseek' },
    { provider: 'openai', apiKey: 'legacy-openai' },
  ]);
  // Before migration the key still counts as configured (upgrade path).
  expect(hasProviderCredential('openai')).toBe(true);
  expect(hasProviderCredential('ollama')).toBe(true);

  stripLegacyCredentials();
  expect(legacyCredentials()).toEqual([]);
  expect(hasProviderCredential('openai')).toBe(false);
  expect(hasProviderCredential('ollama')).toBe(true);

  setCredentialStatus(['openai']);
  expect(hasProviderCredential('openai')).toBe(true);
});
