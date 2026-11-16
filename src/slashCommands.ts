import type { CommandView, SkillView } from './api/extensions';

export interface SlashAction {
  /** Stable id, also the text after `/` (e.g. `plan`). */
  id: string;
  description: string;
  disabled?: boolean;
  run: () => void;
}

export type SlashItem =
  | { id: string; kind: 'action'; name: string; description: string; disabled?: boolean; badge: string; action: SlashAction }
  | { id: string; kind: 'skill'; name: string; description: string; disabled?: boolean; badge: string; skill: SkillView }
  | { id: string; kind: 'command'; name: string; description: string; disabled?: boolean; badge: string; command: CommandView };

export interface SlashGroup {
  title: string;
  items: SlashItem[];
}

const MAX_PER_GROUP = 20;

/// A slash query only exists while the prompt starts with `/` and the user has
/// not dismissed the panel for this edit.
export function slashQuery(prompt: string, dismissed: boolean): { query: string; args: string } | null {
  if (dismissed || !prompt.startsWith('/')) return null;
  const match = /^\/([^\s]*)(?:\s([\s\S]*))?$/.exec(prompt);
  if (!match) return null;
  return { query: match[1] ?? '', args: match[2] ?? '' };
}

function matches(text: string, query: string) {
  return !query || text.toLowerCase().includes(query);
}

export function buildSlashGroups(
  actions: SlashAction[],
  skills: SkillView[],
  commands: CommandView[],
  query: string,
): SlashGroup[] {
  const needle = query.trim().toLowerCase();
  return [
    {
      title: '动作',
      items: actions
        .filter((action) => matches(`/${action.id} ${action.description}`, needle))
        .slice(0, MAX_PER_GROUP)
        .map<SlashItem>((action) => ({
          id: `action:${action.id}`,
          kind: 'action',
          name: action.id,
          description: action.description,
          disabled: action.disabled,
          badge: '动作',
          action,
        })),
    },
    {
      title: '技能',
      items: skills
        .filter((skill) => matches(`${skill.name} ${skill.description}`, needle))
        .slice(0, MAX_PER_GROUP)
        .map<SlashItem>((skill) => ({
          id: `skill:${skill.name}`,
          kind: 'skill',
          name: skill.name,
          description: skill.description,
          badge: skill.source === 'project' ? '技能 · 项目' : '技能',
          skill,
        })),
    },
    {
      title: '自定义命令',
      items: commands
        .filter((command) => matches(`${command.name} ${command.description}`, needle))
        .slice(0, MAX_PER_GROUP)
        .map<SlashItem>((command) => ({
          id: `command:${command.name}`,
          kind: 'command',
          name: command.name,
          description: command.description,
          badge: command.source === 'project' ? '命令 · 项目' : '命令',
          command,
        })),
    },
  ];
}

/// Replace `$ARGUMENTS`, or append the arguments when the template has no slot.
export function expandCommand(body: string, args: string) {
  const trimmed = args.trim();
  if (body.includes('$ARGUMENTS')) return body.split('$ARGUMENTS').join(trimmed);
  return trimmed ? `${body.trimEnd()}\n\n${trimmed}` : body;
}

export function skillDirective(name: string, args: string) {
  const task = args.trim();
  return `请先调用 load_skill 加载技能「${name}」，并严格按照该技能的指令执行。${task ? `\n\n任务：${task}` : ''}`;
}
