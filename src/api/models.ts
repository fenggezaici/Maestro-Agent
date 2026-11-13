/// Model metadata API: provider `/models` fetch and the public catalog —
/// thin pass-throughs over Tauri `invoke`.
import { invoke } from '@tauri-apps/api/core';
import type { ProviderId } from '../modelCapabilities';
import { wireProvider } from '../customProviders';

export interface FetchedModel {
  id: string;
  context: number | null;
}

export interface CatalogModel {
  id: string;
  context: number | null;
  reasoning: boolean;
  reasoningLevels: string[];
}

/// Ask the provider for its real model list (live Tauri only).
export const fetchModelsApi = (provider: ProviderId | (string & {}), baseUrl: string, apiKey: string) =>
  invoke<FetchedModel[]>('fetch_models', { provider: wireProvider(provider), baseUrl, apiKey });

/// Fetch the public models.dev catalog via the Rust `model_catalog` command.
export const fetchCatalogApi = () => invoke<CatalogModel[]>('model_catalog');
