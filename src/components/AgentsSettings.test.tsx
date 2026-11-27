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
      case 'get_agent_defaults': return { model: null, steps: null, toolBudget: null, maxParallel: null, maxTotal: null };
      case 'save_agent_defaults': return;
      case 'get_workspace_info': return { name: 'ws', path: 'E:/ws' };
      case 'save_agent': return;
      default: throw new Error(`Unexpected command ${command}`);
    }
  });
};

beforeEach(() => mockList(builtin));

describe('AgentsSettings', () => {
  it('lists agents in a source-grouped picker and opens the editor on selection', async () => {
    mockList([...builtin, custom]);
    render(<AgentsSettings preview={false} />);
    const picker = await screen.findByLabelText('选择要配置的子智能体');
    await waitFor(() => expect(screen.getByRole('option', { name: 'reviewer' })).toBeInTheDocument());
    expect(screen.getByRole('group', { name: '内置' })).toBeInTheDocument();
    expect(screen.getByRole('group', { name: '全局' })).toBeInTheDocument();

    fireEvent.change(picker, { target: { value: 'reviewer' } });
    expect(screen.getByLabelText('名称')).toBeDisabled();
    expect(screen.getByText(/保存到全局作用域/)).toBeInTheDocument();
    expect(mocks.invoke).toHaveBeenCalledWith('list_agents', { scope: 'all' });
  });

  it('notes that editing a builtin agent writes a global override', async () => {
    render(<AgentsSettings preview={false} />);
    const picker = await screen.findByLabelText('选择要配置的子智能体');
    await waitFor(() => expect(screen.getByRole('option', { name: 'explore' })).toBeInTheDocument());
    fireEvent.change(picker, { target: { value: 'explore' } });
    expect(screen.getByText(/保存会写入全局覆盖文件/)).toBeInTheDocument();
  });

  it('opens a fresh draft when the deep link carries name=null', async () => {
    render(<AgentsSettings preview={false} focus={{ name: null, tick: 1 }} />);
    fireEvent.change(screen.getByLabelText('名称'), { target: { value: 'reviewer' } });
    fireEvent.change(screen.getByLabelText('描述'), { target: { value: '代码评审' } });
    fireEvent.change(screen.getByLabelText('工具预算（调用总次数，留空继承）'), { target: { value: '250' } });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('save_agent', {
      scope: 'global',
      agent: expect.objectContaining({ name: 'reviewer', description: '代码评审', mode: 'subagent', toolBudget: 250, source: 'global' }),
    }));
  });

  it('opens the focused agent from the deep link once the list loads', async () => {
    mockList([...builtin, custom]);
    render(<AgentsSettings preview={false} focus={{ name: 'reviewer', tick: 1 }} />);
    await waitFor(() => expect(screen.getByLabelText('名称')).toHaveValue('reviewer'));
    expect(screen.getByLabelText('名称')).toBeDisabled();
  });

  it('saves global defaults through save_agent_defaults', async () => {
    render(<AgentsSettings preview={false} />);
    await screen.findByLabelText('选择要配置的子智能体');
    fireEvent.change(screen.getByLabelText('默认轮次上限（LLM 对话轮）'), { target: { value: '12' } });
    fireEvent.change(screen.getByLabelText('单 run 总额度'), { target: { value: '12' } });
    fireEvent.click(screen.getByRole('button', { name: '保存全局默认' }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('save_agent_defaults', {
      defaults: expect.objectContaining({ steps: 12, maxTotal: 12 }),
    }));
  });

  it('clamps numeric fields to the raised caps (128 steps / 1000 budget)', async () => {
    render(<AgentsSettings preview={false} />);
    await screen.findByLabelText('选择要配置的子智能体');
    fireEvent.change(screen.getByLabelText('默认轮次上限（LLM 对话轮）'), { target: { value: '999' } });
    fireEvent.change(screen.getByLabelText('默认工具预算（工具调用总次数）'), { target: { value: '99999' } });
    fireEvent.click(screen.getByRole('button', { name: '保存全局默认' }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('save_agent_defaults', {
      defaults: expect.objectContaining({ steps: 128, toolBudget: 1000 }),
    }));
  });

  it('persists blank numeric fields as null so they inherit the global default', async () => {
    render(<AgentsSettings preview={false} focus={{ name: null, tick: 1 }} />);
    fireEvent.change(screen.getByLabelText('名称'), { target: { value: 'scout' } });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('save_agent', {
      scope: 'global',
      agent: expect.objectContaining({ name: 'scout', steps: null, toolBudget: null }),
    }));
  });

  it('toggles the hidden disable flag and persists it on save', async () => {
    mockList([...builtin, custom]);
    render(<AgentsSettings preview={false} />);
    const picker = await screen.findByLabelText('选择要配置的子智能体');
    await waitFor(() => expect(screen.getByRole('option', { name: 'reviewer' })).toBeInTheDocument());
    fireEvent.change(picker, { target: { value: 'reviewer' } });
    const toggle = screen.getByRole('checkbox', { name: /禁用/ });
    expect(toggle).not.toBeChecked();
    fireEvent.click(toggle);
    fireEvent.click(screen.getByRole('button', { name: '保存' }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('save_agent', {
      scope: 'global',
      agent: expect.objectContaining({ name: 'reviewer', hidden: true }),
    }));
  });

  it('keeps management UI out of settings', async () => {
    render(<AgentsSettings preview={false} />);
    await screen.findByLabelText('选择要配置的子智能体');
    expect(screen.queryByLabelText('搜索子智能体')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '新建' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '本地导入' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'GitHub 导入' })).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/^删除 /)).not.toBeInTheDocument();
  });
});
