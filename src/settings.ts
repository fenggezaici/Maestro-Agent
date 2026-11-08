export interface LlmSettings { apiKey: string; baseUrl: string; model: string }

export const LLM_SETTINGS_KEY = 'maestro.llm-settings.v1';
export const defaultLlmSettings: LlmSettings = { apiKey: '', baseUrl: 'https://api.openai.com/v1', model: 'gpt-4o-mini' };

export function readLlmSettings(): LlmSettings {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(LLM_SETTINGS_KEY) ?? 'null');
    if (value && typeof value === 'object') {
      const settings = value as Partial<LlmSettings>;
      if (typeof settings.apiKey === 'string' && typeof settings.baseUrl === 'string' && typeof settings.model === 'string') {
        return { apiKey: settings.apiKey, baseUrl: settings.baseUrl, model: settings.model };
      }
    }
  } catch { /* Invalid/unavailable local settings fall back without logging credentials. */ }
  return { ...defaultLlmSettings };
}

export function validateLlmSettings(settings: LlmSettings): LlmSettings {
  const value = { apiKey: settings.apiKey.trim(), baseUrl: settings.baseUrl.trim(), model: settings.model.trim() };
  if (!value.apiKey || value.apiKey.length > 8192 || /[\u0000-\u001f\u007f]/.test(value.apiKey)) throw new Error('请在 Settings 中填写有效 API Key');
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
  try { localStorage.setItem(LLM_SETTINGS_KEY, JSON.stringify(value)); } catch { throw new Error('无法保存设置：浏览器存储不可用'); }
  return value;
}

export function clearLlmSettings() {
  try { localStorage.removeItem(LLM_SETTINGS_KEY); } catch { throw new Error('无法清除设置：浏览器存储不可用'); }
}
