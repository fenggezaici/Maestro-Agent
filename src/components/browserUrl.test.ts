import { describe, expect, it } from 'vitest';
import { normalizeUrl } from './browserUrl';

describe('normalizeUrl', () => {
  it('补全裸 host 的 http 协议', () => {
    expect(normalizeUrl('localhost:3000')).toBe('http://localhost:3000/');
    expect(normalizeUrl('127.0.0.1:8080/dev')).toBe('http://127.0.0.1:8080/dev');
  });
  it('保留已有 http/https 协议', () => {
    expect(normalizeUrl('https://example.com/a?b=1')).toBe('https://example.com/a?b=1');
    expect(normalizeUrl('http://localhost:5173')).toBe('http://localhost:5173/');
  });
  it('拒绝非 http 协议与非法输入', () => {
    expect(normalizeUrl('file:///etc/passwd')).toBeNull();
    expect(normalizeUrl('javascript:alert(1)')).toBeNull();
    expect(normalizeUrl('')).toBeNull();
    expect(normalizeUrl('   ')).toBeNull();
  });
});
