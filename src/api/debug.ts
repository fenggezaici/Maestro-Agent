/// Debug-only e2e injection surface. `debug_start_fake_stream` exists in
/// release builds too but always errors there — only debug builds produce the
/// fake streaming run. The e2e driver (scripts/e2e-desktop.mjs) reaches it via
/// `window.__TAURI_INTERNALS__.invoke` because CDP evaluate cannot import
/// modules; this wrapper keeps the command on the typed API contract so the
/// ipc_contract guard can see a caller.
import { invoke } from '@tauri-apps/api/core';

export interface DebugFakeStream {
  sessionId: number;
  runId: number;
}

export const debugApi = {
  startFakeStream: (sessionId?: number, seconds?: number) =>
    invoke<DebugFakeStream>('debug_start_fake_stream', {
      session_id: sessionId ?? null,
      seconds: seconds ?? null,
    }),
};
