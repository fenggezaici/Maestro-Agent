import { modelCapability, providers, thinkingLabels, type ProviderId, type ThinkingLevel } from './modelCapabilities';
import { getProvider } from './customProviders';
export interface LlmSettings { apiKey: string; baseUrl: string; model: string; provider?: ProviderId | (string & {}); reasoning?: ThinkingLevel }

export const LLM_SETTINGS_KEY = 'maestro.llm-settings.v1';
export const defaultLlmSettings: LlmSettings = { apiKey: '', baseUrl: 'https://api.openai.com/v1', model: 'gpt-5' };
export const LLM_PROFILES_KEY = 'maestro.llm-profiles.v2';
interface Profiles { active: ProviderId | (string & {}); profiles: Record<string, LlmSettings> }

/// Providers whose key is stored in the OS-protected backend store. The
/// webview only ever learns *whether* a key exists, never its value.
let credentialCache = new Set<string>();

export function setCredentialStatus(providersWithKeys: string[]): void {
  credentialCache = new Set(providersWithKeys);
}

/// Legacy plaintext keys still sitting in localStorage. Read only by the
/// one-time migration (and by the "configured" fallback until it succeeds).
export interface LegacyCredential { provider: string; apiKey: string }

export function legacyCredentials(): LegacyCredential[] {
  const found: LegacyCredential[] = [];
  try {
    const value = JSON.parse(localStorage.getItem(LLM_PROFILES_KEY) ?? 'null');
    for (const [id, profile] of Object.entries((value?.profiles ?? {}) as Record<string, LlmSettings>)) {
      const key = typeof profile?.apiKey === 'string' ? profile.apiKey.trim() : '';
      if (key) found.push({ provider: id, apiKey: key });
    }
    const legacy = JSON.parse(localStorage.getItem(LLM_SETTINGS_KEY) ?? 'null');
    const key = typeof legacy?.apiKey === 'string' ? legacy.apiKey.trim() : '';
    if (key && !found.some((item) => item.provider === 'openai')) found.push({ provider: 'openai', apiKey: key });
  } catch { /* Invalid local data: nothing to migrate. */ }
  return found;
}

/// Drop plaintext keys from localStorage. Only called after the backend
/// confirmed it stored them, so a failed migration keeps the old data.
export function stripLegacyCredentials(): void {
  for (const item of legacyCredentials()) stripLegacyCredential(item.provider);
}

/// Drop one provider's plaintext key after the backend confirmed that exact
/// provider. Other providers keep their data, so one bad key cannot block the
/// rest of the migration.
export function stripLegacyCredential(provider: string): void {
  try {
    const value = JSON.parse(localStorage.getItem(LLM_PROFILES_KEY) ?? 'null');
    const profile = value?.profiles?.[provider];
    if (value && typeof value === 'object' && profile && typeof profile === 'object') {
      profile.apiKey = '';
      localStorage.setItem(LLM_PROFILES_KEY, JSON.stringify(value));
    }
    if (provider === 'openai') {
      const legacy = JSON.parse(localStorage.getItem(LLM_SETTINGS_KEY) ?? 'null');
      if (legacy && typeof legacy === 'object' && 'apiKey' in legacy) {
        localStorage.setItem(LLM_SETTINGS_KEY, JSON.stringify({ ...legacy, apiKey: '' }));
      }
    }
  } catch { /* Nothing to strip. */ }
}

function profiles(): Profiles {
  const result: Profiles = { active: 'openai', profiles: Object.fromEntries(Object.entries(providers).map(([id, value]) => [id, { apiKey: '', baseUrl: value.baseUrl, model: value.model, provider: id }])) as Record<ProviderId, LlmSettings> };
  try {
    const legacy = JSON.parse(localStorage.getItem(LLM_SETTINGS_KEY) ?? 'null');
    if (legacy && ['apiKey', 'baseUrl', 'model'].every((key) => typeof legacy[key] === 'string')) result.profiles.openai = { apiKey: legacy.apiKey, baseUrl: legacy.baseUrl, model: legacy.model, provider: 'openai', ...(typeof legacy.reasoning === 'string' && Object.hasOwn(thinkingLabels, legacy.reasoning) ? { reasoning: legacy.reasoning } : {}) };
    const value = JSON.parse(localStorage.getItem(LLM_PROFILES_KEY) ?? 'null');
    if (value && typeof value === 'object') {
      result.active = typeof value.active === 'string' ? value.active : result.active;
      for (const id of Object.keys(value.profiles ?? {})) {
        const profile = value.profiles?.[id];
        if (profile && ['apiKey', 'baseUrl', 'model'].every((key) => typeof profile[key] === 'string')) result.profiles[id] = { apiKey: profile.apiKey, baseUrl: profile.baseUrl, model: profile.model, provider: id, ...(typeof profile.reasoning === 'string' && Object.hasOwn(thinkingLabels, profile.reasoning) ? { reasoning: profile.reasoning } : {}) };
      }
    }
  } catch { /* Preserve usable defaults; no credential logging. */ }
  return result;
}

/// Whether the provider has a usable credential: stored in the OS-protected
/// backend, or still present as a legacy localStorage key awaiting migration.
export function hasProviderCredential(id: ProviderId | (string & {})): boolean {
  if (id === 'ollama') return true;
  if (credentialCache.has(id)) return true;
  return legacyCredentials().some((item) => item.provider === id);
}

export function selectLlmProvider(id: ProviderId | (string & {})): LlmSettings {
  const value = profiles(); value.active = id;
  try { localStorage.setItem(LLM_PROFILES_KEY, JSON.stringify(value)); } catch { throw new Error('Cannot save provider settings'); }
  return value.profiles[id] ? { ...value.profiles[id] } : { apiKey: '', baseUrl: getProvider(id).baseUrl, model: getProvider(id).model, provider: id };
}

export function readLlmSettings(): LlmSettings {
  try {
    if (localStorage.getItem(LLM_PROFILES_KEY)) { const value = profiles(); return { ...value.profiles[value.active], apiKey: '' }; }
    const value: unknown = JSON.parse(localStorage.getItem(LLM_SETTINGS_KEY) ?? 'null');
    if (value && typeof value === 'object') {
      const settings = value as Partial<LlmSettings>;
      if (typeof settings.baseUrl === 'string' && typeof settings.model === 'string') {
        return { apiKey: '', baseUrl: settings.baseUrl, model: settings.model, ...(settings.reasoning && Object.hasOwn(thinkingLabels, settings.reasoning) ? { reasoning: settings.reasoning } : {}) };
      }
    }
  } catch { /* Invalid/unavailable local settings fall back without logging credentials. */ }
  return { ...defaultLlmSettings };
}

const LOCAL_HOSTS = new Set(['localhost', '127.0.0.1', '::1', '[::1]']);

export function isLocalEndpoint(hostname: string): boolean {
  const host = hostname.toLowerCase();
  return LOCAL_HOSTS.has(host) || host.endsWith('.localhost');
}

/// `requireKey` is only needed where a key is being typed in (the settings
/// form). Starting a run validates the non-secret fields and checks the stored
/// credential separately, because the key no longer travels through the UI.
export function validateLlmSettings(settings: LlmSettings, options: { requireKey?: boolean } = {}): LlmSettings {
  const value = { ...settings, apiKey: (settings.apiKey ?? '').trim(), baseUrl: (settings.baseUrl ?? '').trim(), model: (settings.model ?? '').trim() };
  const strengths = modelCapability(value.model, value.provider ?? 'openai').thinking;
  if (strengths.length) value.reasoning = value.reasoning && strengths.includes(value.reasoning) ? value.reasoning : strengths.includes('medium') ? 'medium' : strengths[0];
  else delete value.reasoning;
  const requireKey = options.requireKey ?? true;
  if (value.apiKey.length > 8192 || /[\u0000-\u001f\u007f]/.test(value.apiKey)) throw new Error('请在 Settings 中填写有效 API Key');
  if (requireKey && !value.apiKey && value.provider !== 'ollama') throw new Error('请在 Settings 中填写有效 API Key');
  if (!value.model || value.model.length > 256) throw new Error('请在 Settings 中填写 Model');
  let url: URL;
  try { url = new URL(value.baseUrl); } catch { throw new Error('Base URL 格式无效'); }
  if (!['http:', 'https:'].includes(url.protocol) || !url.hostname || url.username || url.password || url.search || url.hash) {
    throw new Error('Base URL 必须为 HTTP(S)，且不能包含账号、查询参数或片段');
  }
  // Plaintext HTTP would send the key in the clear; only local endpoints
  // (Ollama, local proxies) keep the http exception.
  if (url.protocol === 'http:' && !isLocalEndpoint(url.hostname)) {
    throw new Error('Base URL 必须使用 HTTPS（本地地址除外）');
  }
  return value;
}

/// Persist the non-secret fields only. The key is written to the OS-protected
/// backend store by `api/credentials.ts`; it is never serialized here.
export function saveLlmSettings(settings: LlmSettings): LlmSettings {
  const value = validateLlmSettings(settings, { requireKey: false });
  const persisted: LlmSettings = { ...value, apiKey: '' };
  try {
    if (value.provider || localStorage.getItem(LLM_PROFILES_KEY)) {
      const all = profiles(); const id = value.provider ?? all.active;
      all.active = id; all.profiles[id] = { ...persisted, provider: id };
      localStorage.setItem(LLM_PROFILES_KEY, JSON.stringify(all));
    } else localStorage.setItem(LLM_SETTINGS_KEY, JSON.stringify(persisted));
  } catch { throw new Error('无法保存设置：浏览器存储不可用'); }
  return value;
}

export function clearLlmSettings() {
  try {
    if (localStorage.getItem(LLM_PROFILES_KEY)) {
      const value = profiles(); const provider = value.active;
      value.profiles[provider] = { apiKey: '', baseUrl: getProvider(provider).baseUrl, model: getProvider(provider).model, provider };
      localStorage.setItem(LLM_PROFILES_KEY, JSON.stringify(value));
    } else localStorage.removeItem(LLM_SETTINGS_KEY);
  } catch { throw new Error('无法清除设置：浏览器存储不可用'); }
}
