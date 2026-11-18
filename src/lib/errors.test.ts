import { expect, it } from 'vitest';
import { errorText } from './errors';

it('keeps the backend reason when Tauri rejects with a plain string', () => {
  expect(errorText('未知 Provider')).toBe('未知 Provider');
  expect(errorText('  无法写入凭据文件  ')).toBe('无法写入凭据文件');
});

it('unwraps Error objects and message-like values', () => {
  expect(errorText(new Error('凭据加密失败'))).toBe('凭据加密失败');
  expect(errorText({ message: 'API Key 无效' })).toBe('API Key 无效');
  expect(errorText({ code: 1 })).toBe('[object Object]');
});

it('never returns an empty line', () => {
  expect(errorText('')).toBe('未知错误');
  expect(errorText('   ')).toBe('未知错误');
  expect(errorText(null)).toBe('未知错误');
  expect(errorText(undefined)).toBe('未知错误');
  expect(errorText(new Error(''))).toBe('未知错误');
});
