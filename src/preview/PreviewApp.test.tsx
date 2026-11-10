import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import PreviewApp from './PreviewApp';
import type { ProposedDiff } from '../ipc';
import type { ExplorerFile } from '../components/workbenchMock';
import { LLM_SETTINGS_KEY } from '../settings';

const ipc = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: ipc.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: ipc.listen }));
vi.mock('../components/MonacoDiff', () => ({ default: ({ diff }: { diff: ProposedDiff }) => <section aria-label={`Diff ${diff.path}`}>{diff.after}</section> }));
vi.mock('../components/MonacoCode', () => ({ default: ({ file }: { file: ExplorerFile }) => <section aria-label={`文件预览 ${file.path}`}>{file.content}</section> }));

beforeEach(() => localStorage.clear());

it('shows explicit preview data without making Rust calls or enabling backend mutations', () => {
  render(<PreviewApp />);
  expect(screen.getByText('Frontend preview')).toBeInTheDocument();
  expect(screen.getByText('示例数据 · 无 Rust IPC')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '发送' })).toBeDisabled();
  expect(screen.getByRole('button', { name: '批准计划' })).toBeDisabled();
  expect(screen.getByRole('button', { name: '在项目 Maestro-Agent 下新建会话' })).toBeEnabled();
  expect(screen.getByLabelText('由 Rust 强制执行')).toHaveTextContent('Code');
  expect(screen.getByRole('tab', { name: 'src/bridge/BridgeSession.h' })).toHaveAttribute('data-state', 'active');
  fireEvent.keyDown(screen.getByRole('tab', { name: 'tests/BridgeSession.test.cpp' }), { key: 'Enter' });
  expect(screen.getByLabelText('Diff tests/BridgeSession.test.cpp')).toHaveTextContent('stop_is_idempotent');
  fireEvent.click(screen.getByRole('button', { name: /Improve event dispatch/ }));
  expect(screen.getByRole('button', { name: /Improve event dispatch/ })).toHaveAttribute('aria-pressed', 'true');
  expect(ipc.invoke).not.toHaveBeenCalled();
  expect(ipc.listen).not.toHaveBeenCalled();
});

it('supports search, settings and navigation without pretending unsupported backend features work', () => {
  render(<PreviewApp />);
  fireEvent.change(screen.getByLabelText('搜索会话'), { target: { value: 'Refactor' } });
  expect(screen.getByRole('button', { name: /Refactor session lifecycle/ })).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: /Improve event dispatch/ })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
  expect(screen.getByRole('dialog')).toHaveTextContent('General');
  expect(screen.getByRole('button', { name: 'Appearance' })).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: '关闭设置' }));
  fireEvent.click(screen.getByRole('button', { name: /Automations/ }));
  expect(screen.getByText('自动任务尚未接入后端，本次重构仅展示入口。')).toBeInTheDocument();
});

it('lists the current project sessions flat, switches projects from the switcher and creates a session inside it', async () => {
  render(<PreviewApp />);
  expect(screen.getByRole('button', { name: /Refactor session lifecycle/ })).toBeInTheDocument();
  expect(screen.getByRole('button', { name: /Improve event dispatch/ })).toBeInTheDocument();
  // Grouped-by-project tree: every expanded project's sessions are listed.
  expect(screen.getByRole('button', { name: /Review database migrations/ })).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: '项目 Agent Tools' }));
  fireEvent.click(screen.getByRole('button', { name: '在项目 Agent Tools 下新建会话' }));
  fireEvent.click(screen.getByRole('button', { name: '项目 Agent Tools' }));
  expect(screen.getByRole('button', { name: /新会话/ })).toHaveAttribute('aria-pressed', 'true');
  expect(screen.getByRole('button', { name: '发送' })).toBeDisabled();
  expect(ipc.invoke).not.toHaveBeenCalled();
  expect(ipc.listen).not.toHaveBeenCalled();
});

it('filters the current project session list by search query', () => {
  render(<PreviewApp />);
  fireEvent.change(screen.getByLabelText('搜索会话'), { target: { value: 'Refactor' } });
  expect(screen.getByRole('button', { name: /Refactor session lifecycle/ })).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: /Improve event dispatch/ })).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('搜索会话'), { target: { value: '' } });
  expect(screen.getByRole('button', { name: /Improve event dispatch/ })).toBeInTheDocument();
});

it('routes Activity to the middle dashboard, keeps Diff state, and restores chat without a modal', async () => {
  render(<PreviewApp />);
  fireEvent.keyDown(screen.getByRole('tab', { name: 'tests/BridgeSession.test.cpp' }), { key: 'Enter' });
  fireEvent.keyDown(screen.getByRole('tab', { name: '行内差异' }), { key: 'Enter' });
  const activity = screen.getByRole('button', { name: 'Activity' });
  expect(activity).toHaveTextContent('120 lines today');
  fireEvent.click(activity);
  const dashboard = await screen.findByRole('region', { name: 'Activity Dashboard' }, { timeout: 8000 });
  expect(activity).toHaveAttribute('aria-current', 'page');
  expect(screen.queryByLabelText('发送消息')).not.toBeInTheDocument();
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  expect(within(dashboard).getByLabelText('Total Lines（Mock）')).toHaveTextContent('12,450');
  expect(within(dashboard).getByLabelText('Plans Approved（Mock）')).toHaveTextContent('45');
  expect(within(dashboard).getByLabelText('Tokens Used（Mock）')).toHaveTextContent('1.2M');
  expect(within(dashboard).getByLabelText('Cost（Mock）')).toHaveTextContent('$3.50');
  expect(screen.getByRole('tab', { name: '行内差异' })).toHaveAttribute('data-state', 'active');
  expect(screen.getByRole('tab', { name: 'tests/BridgeSession.test.cpp' })).toHaveAttribute('data-state', 'active');
  const today = dashboard.querySelector('[data-heatmap-day="2026-11-08"]')!;
  fireEvent.focus(today);
  expect(await screen.findByRole('tooltip')).toHaveTextContent('修改 120 行代码');
  fireEvent.blur(today);
  fireEvent.click(screen.getByRole('button', { name: 'Back to Chat' }));
  expect(screen.getByLabelText('发送消息')).toBeInTheDocument();
  expect(screen.queryByRole('region', { name: 'Activity Dashboard' })).not.toBeInTheDocument();
  expect(ipc.invoke).not.toHaveBeenCalled();
  expect(ipc.listen).not.toHaveBeenCalled();
});

it('returns to chat through Agents, a session, or new-session creation', async () => {
  render(<PreviewApp />);
  async function openDashboard() {
    fireEvent.click(screen.getByRole('button', { name: 'Activity' }));
    await screen.findByRole('region', { name: 'Activity Dashboard' }, { timeout: 8000 });
  }
  await openDashboard();
  fireEvent.click(screen.getByRole('button', { name: /^Agents/ }));
  expect(screen.getByLabelText('发送消息')).toBeInTheDocument();
  await openDashboard();
  fireEvent.click(screen.getByRole('button', { name: /Improve event dispatch/ }));
  expect(screen.getByLabelText('回复内容 1020')).toBeInTheDocument();
  await openDashboard();
  fireEvent.click(screen.getByRole('button', { name: '在项目 Maestro-Agent 下新建会话' }));
  await waitFor(() => expect(screen.getByRole('button', { name: /新会话/ })).toHaveAttribute('aria-pressed', 'true'));
  expect(screen.getByLabelText('发送消息')).toBeInTheDocument();
  expect(ipc.invoke).not.toHaveBeenCalled();
});

it('supports composer mock controls, two-column Settings and shortcut rebinding without reading live secrets', () => {
  const sentinel = JSON.stringify({ apiKey: 'private-sentinel', baseUrl: 'https://private.example/v1', model: 'private-model' });
  localStorage.setItem(LLM_SETTINGS_KEY, sentinel);
  localStorage.setItem('maestro.ui.v1.preview', JSON.stringify({ provider: 'openai', previewModel: 'deepseek-chat' }));
  localStorage.setItem('maestro.llm.models.openai', JSON.stringify([{ id: 'gpt-5.2', enabled: true }]));
  render(<PreviewApp />);
  expect(screen.getByLabelText('由 Rust 强制执行')).toHaveTextContent('Code');
  expect(screen.getByLabelText('选择模型')).toHaveTextContent('deepseek-chat');
  expect(screen.getByLabelText('实时会话统计')).toHaveTextContent('Turns: 1');
  expect(screen.queryByRole('button', { name: '模型参数' })).not.toBeInTheDocument();
  fireEvent.click(screen.getByLabelText('选择模型'));
  fireEvent.click(screen.getByRole('button', { name: /gpt-5\.2/ }));
  expect(screen.getByRole('slider')).toHaveAttribute('aria-valuenow', '1');
  expect(screen.getByLabelText('上下文长度')).toHaveTextContent('400k context');
  fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
  fireEvent.click(screen.getByRole('button', { name: 'Models' }));
  expect(screen.getByLabelText('API Key')).toHaveValue('');
  expect(screen.getByLabelText('API Key')).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: 'Anthropic' }));
  expect(screen.getByLabelText('Base URL')).toHaveValue('https://api.anthropic.com/v1');
  fireEvent.click(screen.getByRole('button', { name: 'Shortcuts' }));
  fireEvent.change(screen.getByLabelText('搜索快捷键'), { target: { value: 'Open Activity' } });
  fireEvent.click(screen.getByRole('button', { name: '重新绑定 Open Activity' }));
  fireEvent.keyDown(screen.getByRole('dialog'), { key: 'b', code: 'KeyB', ctrlKey: true, shiftKey: true });
  expect(screen.getByRole('dialog')).toHaveTextContent('Ctrl+Shift+B');
  expect(localStorage.getItem(LLM_SETTINGS_KEY)).toBe(sentinel);
  expect(ipc.invoke).not.toHaveBeenCalled();
  expect(ipc.listen).not.toHaveBeenCalled();
});

it('switches Review/Explorer, folds folders and preserves virtual extension states', () => {
  render(<PreviewApp />);
  fireEvent.keyDown(screen.getByRole('tab', { name: 'Explorer' }), { key: 'Enter' });
  expect(screen.getByLabelText('文件浏览器')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: /BridgeSession.cpp/ }));
  expect(screen.getByLabelText('文件预览 src/bridge/BridgeSession.cpp')).toHaveTextContent('void BridgeSession::stop');
  fireEvent.click(screen.getByRole('button', { name: '文件夹 bridge' }));
  expect(screen.getByRole('button', { name: '文件夹 bridge' })).toHaveAttribute('aria-expanded', 'false');
  fireEvent.keyDown(screen.getByRole('tab', { name: 'Review' }), { key: 'Enter' });
  expect(screen.getByLabelText('Diff src/bridge/BridgeSession.h')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: /^Extensions/ }));
  expect(screen.getByLabelText('Extensions 工作区')).toBeInTheDocument();
  fireEvent.mouseDown(screen.getByRole('tab', { name: 'MCP 服务' }));
  expect(screen.getByRole('switch', { name: '启用 filesystem' })).toHaveAttribute('data-state', 'checked');
  fireEvent.click(screen.getByRole('switch', { name: '启用 github' }));
  expect(screen.getByRole('switch', { name: '启用 github' })).toHaveAttribute('data-state', 'checked');
  fireEvent.mouseDown(screen.getByRole('tab', { name: '发现' }));
  expect(screen.getByRole('button', { name: '安装 FileSystem' })).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: 'Activity' }));
  fireEvent.click(screen.getByRole('button', { name: /^Extensions/ }));
  fireEvent.mouseDown(screen.getByRole('tab', { name: 'MCP 服务' }));
  expect(screen.getByRole('switch', { name: '启用 github' })).toHaveAttribute('data-state', 'checked');
  expect(ipc.invoke).not.toHaveBeenCalled();
});

