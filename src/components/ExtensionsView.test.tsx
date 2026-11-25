import * as Tooltip from '@radix-ui/react-tooltip';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import ExtensionsView from './ExtensionsView';
import type { ExtensionsController } from '../hooks/useExtensions';
import type { AgentDefinition } from '../api/agents';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

const agents: AgentDefinition[] = [
  { name: 'explore', description: '只读代码调研', category: '研究', mode: 'subagent', model: null, prompt: '', tools: ['read_file', 'list_dir', 'load_skill'], steps: 8, toolBudget: 100, hidden: false, color: '#8B5CF6', source: 'builtin' },
  { name: 'general', description: '只读分析总结', category: '研究', mode: 'subagent', model: null, prompt: '', tools: ['read_file', 'list_dir', 'load_skill'], steps: 8, toolBudget: 100, hidden: false, color: '#22D3EE', source: 'builtin' },
  { name: 'reviewer', description: '代码评审', category: '质量', mode: 'subagent', model: null, prompt: '', tools: ['read_file'], steps: 6, toolBudget: 80, hidden: false, color: null, source: 'global' },
  { name: 'imported-cto', description: '导入角色', category: 'leadership', mode: 'subagent', model: null, prompt: '', tools: [], steps: null, toolBudget: null, hidden: false, color: null, source: 'global' },
];

const makeController = (overrides: Partial<ExtensionsController> = {}): ExtensionsController => ({
  skills: [], servers: [], plugins: [], agents, agentDefaults: null,
  recommendedSkills: [], recommendedMcps: [], mcpConfigText: '', loading: false,
  pending: null, loadError: null, importSummary: null, agentImportSummary: null,
  real: true, hasWorkspace: true, installErrors: {},
  isInstalled: () => false, refresh: async () => {},
  createSkill: vi.fn(async () => null), deleteSkill: vi.fn(async () => null),
  importSkill: vi.fn(async () => null), importSkillGithub: vi.fn(async () => null),
  addMcpServer: vi.fn(async () => null), setMcpTrust: vi.fn(async () => {}),
  importMcpJson: vi.fn(async () => null), toggleMcpServer: vi.fn(async () => {}),
  removeMcpServer: vi.fn(async () => null), installRecommended: vi.fn(async () => null),
  deleteAgent: vi.fn(async () => null), importAgentsDir: vi.fn(async () => null), importAgentsGithub: vi.fn(async () => null),
  setAgentsHidden: vi.fn(async () => null), deleteAgents: vi.fn(async () => null),
  ...overrides,
});

const openAgentsTab = () => fireEvent.mouseDown(screen.getByRole('tab', { name: '子智能体' }));
const renderView = (props: Parameters<typeof ExtensionsView>[0]) => render(<Tooltip.Provider><ExtensionsView {...props} /></Tooltip.Provider>);

describe('ExtensionsView agents tab', () => {
  it('lists agents with source badges and filters by search', async () => {
    renderView({ controller: makeController() });
    openAgentsTab();
    expect(await screen.findByText('explore')).toBeInTheDocument();
    expect(screen.getByText('reviewer')).toBeInTheDocument();
    expect(screen.getAllByText('内置')).toHaveLength(2);
    expect(screen.getByText('子智能体 4')).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('搜索子智能体'), { target: { value: 'review' } });
    expect(screen.queryByText('explore')).not.toBeInTheDocument();
    expect(screen.getByText('reviewer')).toBeInTheDocument();
  });

  it('shows effective values from agentDefaults on agents that leave fields blank', async () => {
    renderView({ controller: makeController({ agentDefaults: { model: null, steps: 12, toolBudget: 42, maxParallel: null, maxTotal: null } }) });
    openAgentsTab();
    await screen.findByText('imported-cto');
    expect(screen.getByText(/12 轮 · 42 次预算/)).toBeInTheDocument();
    expect(screen.getByText(/6 轮 · 80 次预算/)).toBeInTheDocument();
  });

  it('routes edit/create clicks through onConfigureAgent', async () => {
    const onConfigureAgent = vi.fn();
    renderView({ controller: makeController(), onConfigureAgent });
    openAgentsTab();
    fireEvent.click((await screen.findAllByRole('button', { name: '编辑' }))[2]);
    expect(onConfigureAgent).toHaveBeenCalledWith('reviewer');
    fireEvent.click(screen.getByRole('button', { name: '新建' }));
    expect(onConfigureAgent).toHaveBeenCalledWith(null);
  });

  it('deletes a custom agent and keeps builtin undeletable', async () => {
    const deleteAgent = vi.fn(async () => null);
    renderView({ controller: makeController({ deleteAgent }) });
    openAgentsTab();
    expect(await screen.findByLabelText('删除 explore')).toBeDisabled();
    fireEvent.click(screen.getByLabelText('删除 reviewer'));
    fireEvent.click(await screen.findByRole('button', { name: '删除' }));
    expect(deleteAgent).toHaveBeenCalledWith(agents[2]);
  });

  it('submits local directory import to the controller', async () => {
    const importAgentsDir = vi.fn(async () => null);
    renderView({ controller: makeController({ importAgentsDir }) });
    openAgentsTab();
    fireEvent.click(await screen.findByRole('button', { name: '本地导入' }));
    fireEvent.change(screen.getByLabelText(/^本地目录/), { target: { value: 'D:\\agents-pack' } });
    fireEvent.click(screen.getByRole('button', { name: '导入' }));
    await waitFor(() => expect(importAgentsDir).toHaveBeenCalledWith('D:\\agents-pack'));
  });

  it('batch-disables checked agents and clears selection', async () => {
    const setAgentsHidden = vi.fn(async () => null);
    renderView({ controller: makeController({ setAgentsHidden }) });
    openAgentsTab();
    fireEvent.click(await screen.findByLabelText('选择 reviewer'));
    fireEvent.click(screen.getByLabelText('选择 imported-cto'));
    const bar = screen.getByRole('toolbar', { name: '批量操作' });
    expect(bar).toHaveTextContent('已选 2');
    fireEvent.click(within(bar).getByRole('button', { name: '禁用' }));
    await waitFor(() => expect(setAgentsHidden).toHaveBeenCalledWith(
      [{ scope: 'global', name: 'reviewer' }, { scope: 'global', name: 'imported-cto' }], true,
    ));
    await waitFor(() => expect(screen.queryByRole('toolbar', { name: '批量操作' })).not.toBeInTheDocument());
  });

  it('batch delete skips builtin and confirms names', async () => {
    const deleteAgents = vi.fn(async () => null);
    renderView({ controller: makeController({ deleteAgents }) });
    openAgentsTab();
    fireEvent.click(await screen.findByLabelText('选择 explore'));
    fireEvent.click(screen.getByLabelText('选择 reviewer'));
    const bar = screen.getByRole('toolbar', { name: '批量操作' });
    fireEvent.click(within(bar).getByRole('button', { name: '删除' }));
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent('reviewer');
    expect(dialog).toHaveTextContent('1 个内置项自动跳过');
    fireEvent.click(within(dialog).getByRole('button', { name: '删除' }));
    await waitFor(() => expect(deleteAgents).toHaveBeenCalledWith([{ scope: 'global', name: 'reviewer' }]));
  });

  it('shows disabled badge, dims the card, and filters by enabled state', async () => {
    const disabled: AgentDefinition[] = [
      ...agents,
      { name: 'retired', description: '旧角色', category: null, mode: 'subagent', model: null, prompt: '', tools: [], steps: null, toolBudget: null, hidden: true, color: null, source: 'global' },
    ];
    renderView({ controller: makeController({ agents: disabled }) });
    openAgentsTab();
    expect(await screen.findByText('已禁用')).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('按启用状态筛选'), { target: { value: 'disabled' } });
    expect(screen.getByText('retired')).toBeInTheDocument();
    expect(screen.queryByText('reviewer')).not.toBeInTheDocument();
  });

  it('single-card switch calls setAgentsHidden for that agent', async () => {
    const setAgentsHidden = vi.fn(async () => null);
    renderView({ controller: makeController({ setAgentsHidden }) });
    openAgentsTab();
    fireEvent.click(await screen.findByLabelText('启用 reviewer'));
    await waitFor(() => expect(setAgentsHidden).toHaveBeenCalledWith([{ scope: 'global', name: 'reviewer' }], true));
  });
});
