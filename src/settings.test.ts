import { beforeEach, expect, it, vi } from 'vitest';
import { defaultLlmSettings, LLM_SETTINGS_KEY, readLlmSettings, saveLlmSettings, validateLlmSettings } from './settings';

beforeEach(() => localStorage.clear());

it('persists validated connection settings and tolerates invalid saved data', () => {
  const settings = { apiKey: 'test-key', baseUrl: 'https://example.test/v1', model: 'test-model' };
  saveLlmSettings(settings);
  expect(readLlmSettings()).toEqual(settings);
  localStorage.setItem(LLM_SETTINGS_KEY, '{invalid');
  expect(readLlmSettings()).toEqual(defaultLlmSettings);
  expect(() => validateLlmSettings({ ...settings, apiKey: '' })).toThrow();
  expect(() => validateLlmSettings({ ...settings, baseUrl: 'file:///tmp/model' })).toThrow();
  expect(() => validateLlmSettings({ ...settings, baseUrl: 'https://user:password@example.test/v1' })).toThrow();
});

it('reports unavailable localStorage instead of pretending a save succeeded', () => {
  const write = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('blocked'); });
  expect(() => saveLlmSettings({ apiKey: 'test-key', baseUrl: 'http://localhost:1234/v1', model: 'test-model' })).toThrow('浏览器存储不可用');
  write.mockRestore();
});
