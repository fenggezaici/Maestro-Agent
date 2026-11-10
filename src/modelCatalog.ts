import { invoke } from '@tauri-apps/api/core';
import { modelCapability, type ModelCapability, type ProviderId, type ThinkingLevel } from './modelCapabilities';
import { fetchedContext } from './modelFetch';
import { readProviderModels, saveProviderModels } from './providerModels';

/// One entry of the public models.dev catalog (fetched via the Rust
/// `model_catalog` command — the same metadata source opencode-style tools use).
export interface CatalogModel {
  id: string;
  context: number | null;
  reasoning: boolean;
}

const CATALOG_KEY = 'maestro.models.catalog';
const STALE_MS = 24 * 60 * 60 * 1000;

export function readCatalog(): CatalogModel[] {
  try {
    const value = JSON.parse(localStorage.getItem(CATALOG_KEY) ?? 'null');
    const models = Array.isArray(value) ? value : value?.models;
    return Array.isArray(models) ? models.filter((item) => typeof item?.id === 'string') : [];
  } catch {
    return [];
  }
}

export function catalogStale(): boolean {
  try {
    const value = JSON.parse(localStorage.getItem(CATALOG_KEY) ?? 'null');
    return typeof value?.at !== 'number' || Date.now() - value.at > STALE_MS;
  } catch {
    return true;
  }
}

/// Refresh the catalog in the background; resolves to the freshest list (old
/// cache is kept on failure). Safe to call on app start.
export async function loadCatalog(): Promise<CatalogModel[]> {
  const cached = readCatalog();
  if (!catalogStale() && cached.length) return cached;
  try {
    const models = await invoke<CatalogModel[]>('model_catalog');
    if (models.length) {
      try { localStorage.setItem(CATALOG_KEY, JSON.stringify({ at: Date.now(), models })); } catch { /* quota */ }
      return models;
    }
  } catch { /* network must not break startup */ }
  return cached;
}

/// Exact id, then provider-prefix-stripped match (`deepseek/deepseek-r1` → `deepseek-r1`).
export function catalogLookup(model: string): CatalogModel | null {
  const trimmed = model.trim();
  if (!trimmed) return null;
  const list = readCatalog();
  const exact = list.find((item) => item.id === trimmed);
  if (exact) return exact;
  const suffix = trimmed.split('/').pop();
  return list.find((item) => item.id === suffix) ?? null;
}

// ---------------------------------------------------------------------------

export type ReasoningKind = 'none' | 'fixed' | 'effort' | 'budget';

export interface CapabilityOverride {
  context?: number;
  reasoning?: ReasoningKind;
}

export function capabilityOverrides(provider: ProviderId): Record<string, CapabilityOverride> {
  // Source of truth is the provider model list (ZCode-style panel); only
  // enabled rows contribute overrides.
  const map: Record<string, CapabilityOverride> = {};
  for (const item of readProviderModels(provider)) {
    if (item.enabled && (item.context !== undefined || item.reasoning !== undefined)) {
      map[item.id] = { context: item.context, reasoning: item.reasoning };
    }
  }
  return map;
}

export function saveCapabilityOverride(provider: ProviderId, model: string, override: CapabilityOverride) {
  const list = readProviderModels(provider);
  const clean: CapabilityOverride = {};
  if (Number.isFinite(override.context) && (override.context ?? 0) > 0) clean.context = override.context;
  if (override.reasoning) clean.reasoning = override.reasoning;
  const index = list.findIndex((item) => item.id === model);
  const row = { ...(index >= 0 ? list[index] : { id: model, enabled: true }), context: clean.context, reasoning: clean.reasoning };
  if (!row.context && !row.reasoning && index < 0) return;
  if (index >= 0) list[index] = row; else list.push(row);
  saveProviderModels(provider, list);
}

const EFFORT_LEVELS: ThinkingLevel[] = ['low', 'medium', 'high'];
const BUDGET_LEVELS: ThinkingLevel[] = ['low', 'medium', 'high'];

/// Merge order: user override → provider-fetched (Ollama context) →
/// models.dev catalog → built-in static table → unknown. Nothing is invented.
export function resolveCapability(model: string, provider: ProviderId): ModelCapability {
  const base = modelCapability(model, provider);
  const override = capabilityOverrides(provider)[model.trim()];
  const fetched = fetchedContext(model, provider);
  const catalog = catalogLookup(model);

  const reasoning: ModelCapability['reasoning'] = override?.reasoning
    ?? (catalog?.reasoning
      ? provider === 'anthropic' ? 'budget' : provider === 'openai' ? 'effort' : 'fixed'
      : base.reasoning);
  const thinking = override?.reasoning !== undefined
    ? override.reasoning === 'effort' ? EFFORT_LEVELS
      : override.reasoning === 'budget' ? BUDGET_LEVELS
      : []
    : catalog?.reasoning && !base.thinking.length
      ? reasoning === 'budget' ? BUDGET_LEVELS : reasoning === 'effort' ? EFFORT_LEVELS : []
      : base.thinking;

  return {
    context: override?.context ?? fetched ?? catalog?.context ?? base.context,
    referenceContext: base.referenceContext,
    thinking,
    reasoning,
    vision: base.vision,
  };
}
