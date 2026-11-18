/// Provider credential IPC: the plaintext key travels once from the settings
/// form, is protected by the OS (DPAPI on Windows), and is never read back by
/// the webview — only `provider_credential_status` is.
import { invoke, isTauri } from '@tauri-apps/api/core';
import {
  legacyCredentials,
  setCredentialStatus,
  stripLegacyCredential,
} from '../settings';
import { errorText } from '../lib/errors';

export const credentialsApi = {
  save: (provider: string, apiKey: string) =>
    invoke<void>('save_provider_credential', { provider, api_key: apiKey }),
  status: () => invoke<string[]>('provider_credential_status'),
  clear: (provider: string) => invoke<void>('clear_provider_credential', { provider }),
};

export const isLiveDesktop = () => isTauri();

export async function refreshCredentialStatus(): Promise<void> {
  if (!isLiveDesktop()) return;
  try { setCredentialStatus(await credentialsApi.status()); } catch { /* Keep the previous view. */ }
}

/// One-time upgrade path: move plaintext keys out of localStorage. Providers
/// migrate independently — a key the backend rejects keeps its local value and
/// is named in the notice, while the rest still move.
export async function migrateLegacyCredentials(): Promise<string | null> {
  if (!isLiveDesktop()) return null;
  const pending = legacyCredentials();
  if (pending.length === 0) {
    await refreshCredentialStatus();
    return null;
  }

  const failed: Array<{ provider: string; reason: string }> = [];
  const attempted: string[] = [];
  for (const item of pending) {
    try {
      await credentialsApi.save(item.provider, item.apiKey);
      attempted.push(item.provider);
    } catch (reason) {
      const detail = errorText(reason);
      console.error(`[credentials] ${item.provider} 迁移失败：${detail}`);
      failed.push({ provider: item.provider, reason: detail });
    }
  }

  let stored: Set<string>;
  try {
    stored = new Set(await credentialsApi.status());
  } catch (reason) {
    // Without confirmation nothing is cleared: the local keys stay authoritative.
    const detail = errorText(reason);
    console.error(`[credentials] 读取凭据状态失败：${detail}`);
    return `API Key 迁移结果无法确认：${detail}。原设置已保留。`;
  }

  const confirmed = attempted.filter((provider) => stored.has(provider));
  for (const provider of confirmed) stripLegacyCredential(provider);
  setCredentialStatus([...stored]);

  if (failed.length === 0 && confirmed.length === attempted.length) return null;
  const problems = failed.map(({ provider, reason }) => `${provider}（${reason}）`);
  const unconfirmed = attempted.filter((provider) => !stored.has(provider));
  if (unconfirmed.length > 0) problems.push(`未确认写入：${unconfirmed.join('、')}`);
  return `部分 API Key 未能迁移：${problems.join('；')}。已成功的已移入系统凭据，其余原值保留。`;
}
