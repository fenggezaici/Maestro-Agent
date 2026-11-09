import { expect, it } from 'vitest';
import { relativeSessionAge } from './workspace';

it('formats relative session ages and keeps preview sessions illustrative', () => {
  const now = Date.parse('2026-11-09T12:00:00Z');
  const at = (secondsAgo: number) => Math.floor(now / 1000) - secondsAgo;
  expect(relativeSessionAge(at(30), 'live', now)).toBe('刚刚');
  expect(relativeSessionAge(at(300), 'live', now)).toBe('5 分钟前');
  expect(relativeSessionAge(at(7200), 'live', now)).toBe('2 小时前');
  expect(relativeSessionAge(at(86400 * 55), 'live', now)).toBe('55 天前');
  expect(relativeSessionAge(at(86400 * 60), 'live', now)).toBe('60 天前');
  expect(relativeSessionAge(at(86400 * 400), 'live', now)).toBe('1 年前');
  expect(relativeSessionAge(1, 'preview', now)).toBe('示例');
  expect(relativeSessionAge(0, 'live', now)).toBe('示例');
});
