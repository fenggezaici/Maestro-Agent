import { modelCapability, providers, thinkingLabels, type ProviderId, type ThinkingLevel } from './modelCapabilities';
import { getProvider } from './customProviders';
export interface LlmSettings { apiKey: string; baseUrl: string; model: string; provider?: ProviderId | (string & {}); reasoning?: ThinkingLevel }

export const LLM_SETTINGS_KEY = 'maestro.llm-settings.v1';
export const defaultLlmSettings: LlmSettings = { apiKey: '', baseUrl: 'https://api.openai.com/v1', model: 'gpt-5' };
export const LLM_PROFILES_KEY = 'maestro.llm-profiles.v2';
interface Profiles { active: ProviderId | (string & {}); profiles: Record<string, LlmSettings> }

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

export function hasProviderCredential(id: ProviderId | (string & {})): boolean {
  if (id === 'ollama') return true;
  try {
    if (localStorage.getItem(LLM_PROFILES_KEY)) return (profiles().profiles[id]?.apiKey ?? '').trim().length > 0;
    const legacy: unknown = JSON.parse(localStorage.getItem(LLM_SETTINGS_KEY) ?? 'null');
    return id === 'openai' && !!(legacy && typeof legacy === 'object' && typeof (legacy as LlmSettings).apiKey === 'string' && (legacy as LlmSettings).apiKey.trim().length > 0);
  } catch { return false; }
}

export function selectLlmProvider(id: ProviderId | (string & {})): LlmSettings {
  const value = profiles(); value.active = id;
  try { localStorage.setItem(LLM_PROFILES_KEY, JSON.stringify(value)); } catch { throw new Error('Cannot save provider settings'); }
  return value.profiles[id] ? { ...value.profiles[id] } : { apiKey: '', baseUrl: getProvider(id).baseUrl, model: getProvider(id).model, provider: id };
}

export function readLlmSettings(): LlmSettings {
  try {
    if (localStorage.getItem(LLM_PROFILES_KEY)) { const value = profiles(); return { ...value.profiles[value.active] }; }
    const value: unknown = JSON.parse(localStorage.getItem(LLM_SETTINGS_KEY) ?? 'null');
    if (value && typeof value === 'object') {
      const settings = value as Partial<LlmSettings>;
      if (typeof settings.apiKey === 'string' && typeof settings.baseUrl === 'string' && typeof settings.model === 'string') {
        return { apiKey: settings.apiKey, baseUrl: settings.baseUrl, model: settings.model, ...(settings.reasoning && Object.hasOwn(thinkingLabels, settings.reasoning) ? { reasoning: settings.reasoning } : {}) };
      }
    }
  } catch { /* Invalid/unavailable local settings fall back without logging credentials. */ }
  return { ...defaultLlmSettings };
}

export function validateLlmSettings(settings: LlmSettings): LlmSettings {
  const value = { ...settings, apiKey: (settings.apiKey ?? '').trim(), baseUrl: (settings.baseUrl ?? '').trim(), model: (settings.model ?? '').trim() };
  const strengths = modelCapability(value.model, value.provider ?? 'openai').thinking;
  if (strengths.length) value.reasoning = value.reasoning && strengths.includes(value.reasoning) ? value.reasoning : strengths.includes('medium') ? 'medium' : strengths[0];
  else delete value.reasoning;
  if ((!value.apiKey && value.provider !== 'ollama') || value.apiKey.length > 8192 || /[\u0000-\u001f\u007f]/.test(value.apiKey)) throw new Error('请在 Settings 中填写有效 API Key');
  if (!value.model || value.model.length > 256) throw new Error('请在 Settings 中填写 Model');
  let url: URL;
  try { url = new URL(value.baseUrl); } catch { throw new Error('Base URL 格式无效'); }
  if (!['http:', 'https:'].includes(url.protocol) || !url.hostname || url.username || url.password || url.search || url.hash) {
    throw new Error('Base URL 必须为 HTTP(S)，且不能包含账号、查询参数或片段');
  }
  return value;
}

export function saveLlmSettings(settings: LlmSettings): LlmSettings {
  const value = validateLlmSettings(settings);
  try {
    if (value.provider || localStorage.getItem(LLM_PROFILES_KEY)) {
      const all = profiles(); const id = value.provider ?? all.active;
      all.active = id; all.profiles[id] = { ...value, provider: id };
      localStorage.setItem(LLM_PROFILES_KEY, JSON.stringify(all));
    } else localStorage.setItem(LLM_SETTINGS_KEY, JSON.stringify(value));
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
