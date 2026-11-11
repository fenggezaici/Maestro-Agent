import { providers, type ProviderId } from './modelCapabilities';

/// User-added OpenAI-compatible endpoints (ZCode-style 自定义供应商). Each is
/// a local profile with its own base URL / API key / model list — stored as
/// plain localStorage like the built-in provider profiles.
export interface CustomProvider {
  id: string;
  name: string;
  baseUrl: string;
}

const KEY = 'maestro.llm.customProviders';

export function readCustomProviders(): CustomProvider[] {
  try {
    const value = JSON.parse(localStorage.getItem(KEY) ?? 'null');
    return Array.isArray(value) ? value.filter((item) => typeof item?.id === 'string' && typeof item?.name === 'string') : [];
  } catch {
    return [];
  }
}

export function saveCustomProvider(entry: { name: string; baseUrl: string; id?: string }): CustomProvider {
  const name = (entry.name ?? '').trim().slice(0, 60);
  const baseUrl = (entry.baseUrl ?? '').trim().replace(/\/+$/, '');
  if (!name) throw new Error('供应商名称不能为空');
  if (!/^https?:\/\//i.test(baseUrl)) throw new Error('Base URL 需要以 http(s):// 开头');
  const list = readCustomProviders();
  const id = entry.id?.trim() || `custom-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
  if (providers[id as ProviderId] && !entry.id) throw new Error('供应商 ID 与内置冲突');
  const index = list.findIndex((item) => item.id === id);
  const row = { id, name, baseUrl };
  if (index >= 0) list[index] = row; else list.push(row);
  try { localStorage.setItem(KEY, JSON.stringify(list.slice(0, 50))); } catch { throw new Error('浏览器存储不可用'); }
  return row;
}

export function removeCustomProvider(id: string) {
  try { localStorage.setItem(KEY, JSON.stringify(readCustomProviders().filter((item) => item.id !== id))); } catch { /* quota */ }
}

export function isCustomProvider(id: string): boolean {
  return readCustomProviders().some((item) => item.id === id);
}

export interface ProviderInfo { name: string; baseUrl: string; model: string }

/// Static providers merged with user-added custom endpoints.
export function getProvider(id: string): ProviderInfo {
  const builtin = providers[id as keyof typeof providers];
  if (builtin) return builtin;
  const custom = readCustomProviders().find((item) => item.id === id);
  if (custom) return { name: custom.name, baseUrl: custom.baseUrl, model: '' };
  return { name: id, baseUrl: '', model: '' };
}

/// Wire format for the Rust `Provider` enum — custom endpoints speak the
/// OpenAI chat-completions dialect.
export function wireProvider(id: string): ProviderId {
  return providers[id as ProviderId] ? (id as ProviderId) : 'openai';
}
