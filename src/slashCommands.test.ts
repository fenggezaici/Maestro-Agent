import { describe, expect, it } from 'vitest';
import { buildSlashGroups, expandCommand, skillDirective, slashQuery, type SlashAction } from './slashCommands';

const actions: SlashAction[] = [
  { id: 'plan', description: '切换到 Plan 模式', run: () => {} },
  { id: 'code', description: '切换到 Code 模式', disabled: true, run: () => {} },
];
const skills = [{ name: 'pdf-tools', description: '处理 PDF', source: 'global' as const, path: 'skill.md' }];
const commands = [
  { name: 'review', description: '审查变更', argumentHint: '<路径>', source: 'project' as const, path: 'review.md' },
  { name: 'deploy', description: '发布构建', source: 'global' as const, path: 'deploy.md' },
];

describe('slashQuery', () => {
  it('opens only for a prompt that starts with a slash', () => {
    expect(slashQuery('hello', false)).toBeNull();
    expect(slashQuery('/plan', true)).toBeNull();
    expect(slashQuery('/', false)).toEqual({ query: '', args: '' });
    expect(slashQuery('/pl', false)).toEqual({ query: 'pl', args: '' });
    expect(slashQuery('/review src/a.ts', false)).toEqual({ query: 'review', args: 'src/a.ts' });
  });
});

describe('buildSlashGroups', () => {
  it('keeps the action → skill → command order and filters by name', () => {
    const all = buildSlashGroups(actions, skills, commands, '');
    expect(all.map((group) => group.title)).toEqual(['动作', '技能', '自定义命令']);
    expect(all[0].items).toHaveLength(2);
    expect(all[1].items[0].badge).toBe('技能');
    expect(all[2].items[0].badge).toBe('命令 · 项目');
    expect(all[2].items[1].badge).toBe('命令');

    const filtered = buildSlashGroups(actions, skills, commands, 'code');
    expect(filtered[0].items.map((item) => item.name)).toEqual(['code']);
    expect(filtered[0].items[0].disabled).toBe(true);
    expect(filtered[1].items).toHaveLength(0);
    expect(filtered[2].items).toHaveLength(0);
  });

  it('matches skill and command descriptions too', () => {
    expect(buildSlashGroups(actions, skills, commands, 'PDF')[1].items).toHaveLength(1);
    expect(buildSlashGroups(actions, skills, commands, '审查')[2].items).toHaveLength(1);
  });
});

describe('command expansion', () => {
  it('replaces $ARGUMENTS or appends when the template has no slot', () => {
    expect(expandCommand('请审查 $ARGUMENTS', 'src/a.ts')).toBe('请审查 src/a.ts');
    expect(expandCommand('请审查 $ARGUMENTS', '')).toBe('请审查 ');
    expect(expandCommand('请审查变更', 'src/a.ts')).toBe('请审查变更\n\nsrc/a.ts');
    expect(expandCommand('请审查变更', '  ')).toBe('请审查变更');
  });

  it('builds a skill directive that keeps the task text', () => {
    expect(skillDirective('pdf-tools', '')).toContain('load_skill');
    expect(skillDirective('pdf-tools', '合并两个文件')).toContain('任务：合并两个文件');
  });
});
