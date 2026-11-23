import { describe, expect, it } from 'vitest';
import { formatEnvText, parseEnvText, parseHeadersText } from './envText';

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

  it('accepts a JSON object (ZCode-style paste)', () => {
    const result = parseEnvText('{\n  "TOKEN": "abc",\n  "EMPTY": "",\n  "NUM": 42\n}');
    expect(result.error).toBeNull();
    expect(result.env).toEqual({ TOKEN: 'abc', EMPTY: '', NUM: '42' });
  });

  it('rejects broken JSON and nested values', () => {
    expect(parseEnvText('{bad').error).toContain('JSON 解析失败');
    expect(parseEnvText('{"A": {"b": 1}}').error).toContain('值必须为字符串');
    expect(parseEnvText('{"1BAD": "x"}').error).toContain('环境变量名无效');
    // 非 { 开头的 JSON 值走逐行分支，同样给出格式错误
    expect(parseEnvText('["A"]').error).toContain('KEY=VALUE');
  });
});

describe('parseHeadersText', () => {
  it('parses Name: value lines', () => {
    const result = parseHeadersText('Authorization: Bearer x\nX-Empty: \n# comment\n');
    expect(result.error).toBeNull();
    expect(result.map).toEqual({ Authorization: 'Bearer x' });
  });

  it('accepts a JSON object and drops empty values', () => {
    const result = parseHeadersText('{"Authorization": "Bearer ol_api_x", "X-Skip": ""}');
    expect(result.error).toBeNull();
    expect(result.map).toEqual({ Authorization: 'Bearer ol_api_x' });
  });

  it('rejects invalid header names and non-string values', () => {
    expect(parseHeadersText('{"Bad Name": "x"}').error).toContain('请求头名无效');
    expect(parseHeadersText('{"A": null}').error).toContain('值必须为字符串');
    expect(parseHeadersText('{oops').error).toContain('JSON 解析失败');
  });
});

describe('formatEnvText', () => {
  it('round-trips through the parser', () => {
    const env = { A: '1', B: 'x y' };
    expect(parseEnvText(formatEnvText(env)).env).toEqual(env);
  });
});
