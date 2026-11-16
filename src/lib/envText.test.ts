import { describe, expect, it } from 'vitest';
import { formatEnvText, parseEnvText } from './envText';

describe('parseEnvText', () => {
  it('parses KEY=VALUE lines and skips blanks and comments', () => {
    const result = parseEnvText('# token\nGITHUB_PERSONAL_ACCESS_TOKEN=ghp_x\n\nDEBUG = 1\n');
    expect(result.error).toBeNull();
    expect(result.env).toEqual({ GITHUB_PERSONAL_ACCESS_TOKEN: 'ghp_x', DEBUG: '1' });
  });

  it('keeps "=" inside values', () => {
    expect(parseEnvText('TOKEN=a=b=c').env).toEqual({ TOKEN: 'a=b=c' });
  });

  it('rejects malformed lines and invalid keys', () => {
    expect(parseEnvText('NO_EQUALS').error).toContain('KEY=VALUE');
    expect(parseEnvText('1BAD=x').error).toContain('环境变量名无效');
  });
});

describe('formatEnvText', () => {
  it('round-trips through the parser', () => {
    const env = { A: '1', B: 'x y' };
    expect(parseEnvText(formatEnvText(env)).env).toEqual(env);
  });
});
