/// 浏览器面板地址输入规范化：无协议补 http://（dev server 场景为主），
/// 非法输入返回 null——调用方按"不是 URL"提示，不静默乱跳转。
export function normalizeUrl(raw: string): string | null {
  const input = raw.trim();
  if (!input || input.length > 2048) return null;
  // 用 :// 判定而非 scheme 正则——`localhost:3000` 的冒号是端口不是协议。
  const withScheme = /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(input) ? input : `http://${input}`;
  let parsed: URL;
  try {
    parsed = new URL(withScheme);
  } catch {
    return null;
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return null;
  if (!parsed.hostname) return null;
  return parsed.href;
}
