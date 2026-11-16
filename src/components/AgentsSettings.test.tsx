import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import AgentsSettings from './AgentsSettings';
import type { AgentDefinition } from '../api/agents';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

const builtin: AgentDefinition[] = [
  { name: 'explore', description: '只读代码调研', category: '研究', mode: 'subagent', model: null, prompt: '', tools: ['read_file', 'list_dir', 'load_skill'], steps: 8, hidden: false, color: '#8B5CF6', source: 'builtin' },
  { name: 'general', description: '只读分析总结', category: '研究', mode: 'subagent', model: null, prompt: '', tools: ['read_file', 'list_dir', 'load_skill'], steps: 8, hidden: false, color: '#22D3EE', source: 'builtin' },
];
const custom: AgentDefinition = { name: 'reviewer', description: '代码评审', category: '质量', mode: 'subagent', model: null, prompt: '只读评审', tools: ['read_file'], steps: 6, hidden: false, color: null, source: 'global' };

const mockList = (agents: AgentDefinition[]) => {
  mocks.invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case 'list_agents': return structuredClone(agents);
      case 'get_workspace_info': return { name: 'ws', path: 'E:/ws' };
      case 'save_agent':
      case 'delete_agent': return;
      default: throw new Error(`Unexpected command ${command}`);
    }
  });
};

beforeEach(() => mockList(builtin));

describe('AgentsSettings', () => {
  it('marks builtin agents and keeps them undeletable', async () => {
    render(<AgentsSettings preview={false} />);
    expect(await screen.findByText('explore')).toBeInTheDocument();
    expect(screen.getAllByText('内置')).toHaveLength(2);
    expect(screen.getByLabelText('删除 explore')).toBeDisabled();
    expect(mocks.invoke).toHaveBeenCalledWith('list_agents', { scope: 'all' });
  });

  it('creates a global agent with the read-only tool selection', async () => {
    render(<AgentsSettings preview={false} />);
    await screen.findByText('explore');
    fireEvent.click(screen.getByRole('button', { name: '新建' }));
    fireEvent.change(screen.getByLabelText('名称'), { target: { value: 'reviewer' } });
    fireEvent.change(screen.getByLabelText('描述'), { target: { value: '代码评审' } });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('save_agent', {
      scope: 'global',
      agent: expect.objectContaining({ name: 'reviewer', description: '代码评审', mode: 'subagent', source: 'global' }),
    }));
  });

  it('deletes a custom agent from its own scope', async () => {
    mockList([...builtin, custom]);
    render(<AgentsSettings preview={false} />);
    await screen.findByText('reviewer');
    fireEvent.click(screen.getByLabelText('删除 reviewer'));
    fireEvent.click(await screen.findByRole('button', { name: '删除' }));

    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('delete_agent', { scope: 'global', name: 'reviewer' }));
  });

  it('keeps edits to an existing agent name-locked', async () => {
    mockList([...builtin, custom]);
    render(<AgentsSettings preview={false} />);
    await screen.findByText('reviewer');
    fireEvent.click(screen.getAllByRole('button', { name: '编辑' }).at(-1)!);
    expect(screen.getByLabelText('名称')).toBeDisabled();
    expect(screen.getByText(/保存到全局作用域/)).toBeInTheDocument();
  });
});
