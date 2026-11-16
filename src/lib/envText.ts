/// Parse a `KEY=VALUE` block used by the MCP env editor.
export function parseEnvText(text: string): { env: Record<string, string>; error: string | null } {
  const env: Record<string, string> = {};
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    const index = line.indexOf('=');
    if (index <= 0) return { env: {}, error: `环境变量格式应为 KEY=VALUE：${line}` };
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
