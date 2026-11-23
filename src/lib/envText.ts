/// Parse a `KEY=VALUE` block used by the MCP env editor.
export function parseEnvText(text: string): { env: Record<string, string>; error: string | null } {
  const trimmed = text.trim();
  if (trimmed.startsWith('{')) {
    const parsed = parseJsonObjectMap(trimmed, '环境变量', /^[A-Za-z_][A-Za-z0-9_]*$/, false);
    return { env: parsed.map, error: parsed.error };
  }
  const env: Record<string, string> = {};
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    const index = line.indexOf('=');
    if (index <= 0) return { env: {}, error: `环境变量格式应为 KEY=VALUE 或 JSON 对象：${line}` };
    const key = line.slice(0, index).trim();
    const value = line.slice(index + 1).trim();
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) return { env: {}, error: `环境变量名无效：${key}` };
    env[key] = value;
  }
  return { env, error: null };
}

export function formatEnvText(env: Record<string, string>) {
  return Object.entries(env).map(([key, value]) => `${key}=${value}`).join('\n');
}

/// Parse a `Name: value` block used by the remote-MCP headers editor.
export function parseHeadersText(text: string): { map: Record<string, string>; error: string | null } {
  const trimmed = text.trim();
  if (trimmed.startsWith('{')) {
    return parseJsonObjectMap(trimmed, '请求头', /^[A-Za-z0-9-]+$/, true);
  }
  const map: Record<string, string> = {};
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    const index = line.indexOf(':');
    if (index <= 0) return { map: {}, error: `请求头格式应为 Name: value 或 JSON 对象：${line}` };
    const key = line.slice(0, index).trim();
    const value = line.slice(index + 1).trim();
    if (!/^[A-Za-z0-9-]+$/.test(key)) return { map: {}, error: `请求头名无效：${key}` };
    if (value) map[key] = value;
  }
  return { map, error: null };
}

/// JSON object branch shared by both parsers (ZCode pastes `{"K": "v"}` directly).
function parseJsonObjectMap(
  text: string,
  label: string,
  keyPattern: RegExp,
  dropEmptyValues: boolean,
): { map: Record<string, string>; error: string | null } {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch (error) {
    return { map: {}, error: `${label} JSON 解析失败：${error instanceof Error ? error.message : String(error)}` };
  }
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
    return { map: {}, error: `${label} JSON 必须是对象` };
  }
  const map: Record<string, string> = {};
  for (const [key, value] of Object.entries(parsed)) {
    if (!keyPattern.test(key)) return { map: {}, error: `${label}名无效：${key}` };
    if (typeof value === 'string') {
      if (value || !dropEmptyValues) map[key] = value;
    } else if (typeof value === 'number' || typeof value === 'boolean') {
      map[key] = String(value);
    } else {
      return { map: {}, error: `${label}值必须为字符串：${key}` };
    }
  }
  return { map, error: null };
}
