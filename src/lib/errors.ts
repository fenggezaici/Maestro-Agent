/// Normalize an IPC/JS failure into a readable line. Tauri's `invoke` rejects
/// with a plain string, so `reason instanceof Error` is false for backend
/// errors — treating both shapes keeps the real reason instead of a generic
/// fallback. Never append request arguments here: a credential must not end up
/// in a toast or a log line.
export function errorText(reason: unknown): string {
  if (reason instanceof Error) return reason.message.trim() || '未知错误';
  if (typeof reason === 'string') return reason.trim() || '未知错误';
  if (reason && typeof reason === 'object' && 'message' in reason) {
    const message = (reason as { message: unknown }).message;
    if (typeof message === 'string' && message.trim()) return message.trim();
  }
  if (reason === null || reason === undefined) return '未知错误';
  return String(reason);
}
