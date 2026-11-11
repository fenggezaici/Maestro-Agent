import type { ProviderId } from './modelCapabilities';
import type { FetchedModel } from './modelFetch';
import type { ReasoningKind } from './modelCatalog';

/// User-managed model list per provider — same shape as ZCode's provider
/// panel: every row is editable (context window, max output, reasoning kind)
/// and can be toggled on/off.
export interface ProviderModel {
  id: string;
  context?: number;
  output?: number;
  reasoning?: ReasoningKind;
  enabled: boolean;
}

const key = (provider: ProviderId | (string & {})) => `maestro.llm.models.${provider}`;
const removedKey = (provider: ProviderId | (string & {})) => `maestro.llm.models.removed.${provider}`;

function readRemoved(provider: ProviderId | (string & {})): Set<string> {
  try { const v = JSON.parse(localStorage.getItem(removedKey(provider)) ?? '[]'); return new Set(Array.isArray(v) ? v : []); } catch { return new Set(); }
}

export function removeProviderModel(provider: ProviderId | (string & {}), id: string) {
  saveProviderModels(provider, readProviderModels(provider).filter((item) => item.id !== id));
  const removed = readRemoved(provider); removed.add(id);
  try { localStorage.setItem(removedKey(provider), JSON.stringify([...removed])); } catch { /* quota */ }
}

export function clearRemoved(provider: ProviderId | (string & {}), id: string) {
  const removed = readRemoved(provider);
  if (!removed.has(id)) return;
  removed.delete(id);
  try { localStorage.setItem(removedKey(provider), JSON.stringify([...removed])); } catch { /* quota */ }
}
const legacyKey = (provider: ProviderId | (string & {})) => `maestro.llm.capability.${provider}`;

export function readProviderModels(provider: ProviderId | (string & {})): ProviderModel[] {
  try {
    const value = JSON.parse(localStorage.getItem(key(provider)) ?? 'null');
    if (!Array.isArray(value)) return migrateLegacy(provider);
    return value.filter((item) => typeof item?.id === 'string');
  } catch {
    return [];
  }
}

/// One-time merge of the old `maestro.llm.capability.{provider}` override map
/// into the list format.
function migrateLegacy(provider: ProviderId | (string & {})): ProviderModel[] {
  try {
    const legacy = JSON.parse(localStorage.getItem(legacyKey(provider)) ?? 'null');
    if (!legacy || typeof legacy !== 'object') return [];
    const migrated: ProviderModel[] = Object.entries(legacy)
      .filter(([, v]) => v && typeof v === 'object')
      .map(([id, v]) => ({ id, context: (v as { context?: number }).context, reasoning: (v as { reasoning?: ReasoningKind }).reasoning, enabled: true }));
    if (migrated.length) saveProviderModels(provider, migrated);
    try { localStorage.removeItem(legacyKey(provider)); } catch { /* quota */ }
    return migrated;
  } catch {
    return [];
  }
}

export function saveProviderModels(provider: ProviderId | (string & {}), models: ProviderModel[]) {
  try { localStorage.setItem(key(provider), JSON.stringify(models.slice(0, 300))); } catch { /* quota */ }
}

/// Merge provider-fetched ids into the list: new ids are appended (enabled),
/// existing rows keep user-edited context/reasoning; fetched context fills gaps.
export function mergeFetched(provider: ProviderId | (string & {}), fetched: FetchedModel[]): ProviderModel[] {
  const existing = readProviderModels(provider);
  const byId = new Map(existing.map((item) => [item.id, item]));
  const removed = readRemoved(provider);
  for (const item of fetched) {
    if (removed.has(item.id)) continue;
    const row = byId.get(item.id);
    if (!row) byId.set(item.id, { id: item.id, context: item.context ?? undefined, enabled: true });
    else if (row.context === undefined && item.context !== null) byId.set(item.id, { ...row, context: item.context });
  }
  return [...byId.values()];
}

export function providerModel(provider: ProviderId | (string & {}), id: string): ProviderModel | undefined {
  return readProviderModels(provider).find((item) => item.id === (id ?? '').trim());
}
