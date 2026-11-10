import { invoke } from '@tauri-apps/api/core';
import type { ProviderId } from './modelCapabilities';

export interface FetchedModel {
  id: string;
  context: number | null;
}

/// Ask the provider for its real model list (live Tauri only).
export const fetchModels = (provider: ProviderId, baseUrl: string, apiKey: string) =>
  invoke<FetchedModel[]>('fetch_models', { provider, baseUrl, apiKey });

const key = (provider: ProviderId) => `maestro.llm.fetched.${provider}`;
const STALE_MS = 24 * 60 * 60 * 1000;

interface FetchedCache { at: number; models: FetchedModel[] }

function readCache(provider: ProviderId): FetchedCache {
  try {
    const value = JSON.parse(localStorage.getItem(key(provider)) ?? 'null');
    // Legacy format was a bare FetchedModel[].
    const models = Array.isArray(value) ? value : value?.models;
    if (!Array.isArray(models)) return { at: 0, models: [] };
    return { at: typeof value?.at === 'number' ? value.at : 0, models: models.filter((item) => typeof item?.id === 'string') };
  } catch {
    return { at: 0, models: [] };
  }
}

export function readFetchedModels(provider: ProviderId): FetchedModel[] {
  return readCache(provider).models;
}

/// True when the cached list is missing or older than 24h — worth a background refetch.
export function fetchedStale(provider: ProviderId): boolean {
  return Date.now() - readCache(provider).at > STALE_MS;
}

export function saveFetchedModels(provider: ProviderId, models: FetchedModel[]) {
  try { localStorage.setItem(key(provider), JSON.stringify({ at: Date.now(), models: models.slice(0, 500) })); } catch { /* quota */ }
}

/// Real context window fetched from the provider (e.g. Ollama /api/show), if known.
export function fetchedContext(model: string, provider: ProviderId): number | null {
  return readFetchedModels(provider).find((item) => item.id === model)?.context ?? null;
}
