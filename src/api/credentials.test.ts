import { beforeEach, expect, it, vi } from 'vitest';
import { LLM_PROFILES_KEY, LLM_SETTINGS_KEY, legacyCredentials, setCredentialStatus } from '../settings';

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), tauri: true }));
vi.mock('@tauri-apps/api/core', () => ({
  invoke: mocks.invoke,
  isTauri: () => mocks.tauri,
}));

const { migrateLegacyCredentials } = await import('./credentials');

beforeEach(() => {
  localStorage.clear();
  setCredentialStatus([]);
  mocks.invoke.mockReset();
  mocks.tauri = true;
  vi.spyOn(console, 'error').mockImplementation(() => {});
});

const seedProfiles = (profiles: Record<string, string>) => {
  localStorage.setItem(LLM_PROFILES_KEY, JSON.stringify({
    active: Object.keys(profiles)[0],
    profiles: Object.fromEntries(Object.entries(profiles).map(([id, key]) => [
      id,
      { apiKey: key, baseUrl: 'https://example.test/v1', model: 'm' },
    ])),
  }));
};

it('moves every legacy key and only then strips them', async () => {
  seedProfiles({ openai: 'sk-one', deepseek: 'sk-two' });
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === 'save_provider_credential') return;
    if (command === 'provider_credential_status') return ['deepseek', 'openai'];
    throw new Error(`Unexpected ${command}`);
  });

  expect(await migrateLegacyCredentials()).toBeNull();
  expect(mocks.invoke).toHaveBeenCalledWith('save_provider_credential', { provider: 'openai', api_key: 'sk-one' });
  expect(legacyCredentials()).toEqual([]);
  expect(localStorage.getItem(LLM_PROFILES_KEY)).not.toContain('sk-one');
});

it('names a rejected provider, keeps its key, and still migrates the others', async () => {
  seedProfiles({ openai: 'sk-good', 'custom-abc': 'bad\u0000key' });
  mocks.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    if (command === 'save_provider_credential') {
      if (args?.provider === 'custom-abc') throw 'API Key 无效';
      return;
    }
    if (command === 'provider_credential_status') return ['openai'];
    throw new Error(`Unexpected ${command}`);
  });

  const notice = await migrateLegacyCredentials();
  expect(notice).toContain('custom-abc（API Key 无效）');
  expect(notice).toContain('其余原值保留');
  // The good key moved; the rejected one is still in localStorage.
  expect(legacyCredentials()).toEqual([{ provider: 'custom-abc', apiKey: 'bad\u0000key' }]);
  expect(setCredentialStatus).toBeDefined();
});

it('clears nothing when the backend does not confirm the write', async () => {
  seedProfiles({ openai: 'sk-one' });
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === 'save_provider_credential') return;
    if (command === 'provider_credential_status') return [];
    throw new Error(`Unexpected ${command}`);
  });

  const notice = await migrateLegacyCredentials();
  expect(notice).toContain('未确认写入：openai');
  expect(legacyCredentials()).toEqual([{ provider: 'openai', apiKey: 'sk-one' }]);
});

it('clears nothing when the status call itself fails', async () => {
  seedProfiles({ openai: 'sk-one' });
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === 'save_provider_credential') return;
    if (command === 'provider_credential_status') throw '无法读取凭据文件';
    throw new Error(`Unexpected ${command}`);
  });

  const notice = await migrateLegacyCredentials();
  expect(notice).toContain('无法读取凭据文件');
  expect(legacyCredentials()).toEqual([{ provider: 'openai', apiKey: 'sk-one' }]);
});

it('does nothing in preview or when there is nothing to migrate', async () => {
  mocks.tauri = false;
  seedProfiles({ openai: 'sk-one' });
  expect(await migrateLegacyCredentials()).toBeNull();
  expect(mocks.invoke).not.toHaveBeenCalled();

  mocks.tauri = true;
  localStorage.clear();
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === 'provider_credential_status') return ['openai'];
    throw new Error(`Unexpected ${command}`);
  });
  expect(await migrateLegacyCredentials()).toBeNull();
  expect(mocks.invoke).toHaveBeenCalledWith('provider_credential_status');
});

it('still clears the legacy v1 slot for openai', async () => {
  localStorage.setItem(LLM_SETTINGS_KEY, JSON.stringify({ apiKey: 'sk-v1', baseUrl: 'https://example.test/v1', model: 'm' }));
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === 'save_provider_credential') return;
    if (command === 'provider_credential_status') return ['openai'];
    throw new Error(`Unexpected ${command}`);
  });
  expect(await migrateLegacyCredentials()).toBeNull();
  expect(localStorage.getItem(LLM_SETTINGS_KEY)).not.toContain('sk-v1');
});
