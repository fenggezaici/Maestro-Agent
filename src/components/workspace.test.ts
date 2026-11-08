import { beforeEach, expect, it, vi } from 'vitest';
import { readExpandedProjects, saveExpandedProjects, type WorkbenchProject } from './workspace';

const projects: WorkbenchProject[] = [{ id: 'one', name: 'One', sessions: [] }, { id: 'two', name: 'Two', sessions: [] }];
beforeEach(() => localStorage.clear());

it('honors saved all-collapsed state and rejects invalid preference shapes', () => {
  expect(readExpandedProjects('key', projects)).toEqual(['one', 'two']);
  localStorage.setItem('key', '[]');
  expect(readExpandedProjects('key', projects)).toEqual([]);
  localStorage.setItem('key', '["one","unknown"]');
  expect(readExpandedProjects('key', projects)).toEqual(['one']);
  localStorage.setItem('key', '{invalid');
  expect(readExpandedProjects('key', projects)).toEqual(['one', 'two']);
});

it('separates live and preview preferences and tolerates unavailable storage', () => {
  saveExpandedProjects('maestro.projects.v1.preview', ['one']);
  expect(readExpandedProjects('maestro.projects.v1.live', projects)).toEqual(['one', 'two']);
  const write = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('Storage unavailable'); });
  expect(() => saveExpandedProjects('blocked', ['two'])).not.toThrow();
  write.mockRestore();
});
