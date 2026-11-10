import { StrictMode } from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import type { ChatMessage, KernelEvent, PlanCard, ProposedDiff, RunView, SessionDetail, SessionView, Snapshot } from './ipc';
import { LLM_SETTINGS_KEY } from './settings';

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn(), stop: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
vi.mock('./components/MonacoDiff', () => ({
  default: ({ diff, split }: { diff: ProposedDiff; split: boolean }) =>
    <section aria-label={`Diff ${diff.path}`} data-split={split}><pre>{diff.before}</pre><pre>{diff.after}</pre></section>,
}));
vi.mock('./components/MonacoCode', () => ({ default: ({ file }: { file: { path: string; content: string } }) => <section aria-label={`文件预览 ${file.path}`}>{file.content}</section> }));

let snapshot: Snapshot;
let history: RunView[];
let sessions: SessionView[];
let plans: PlanCard[];
let receive: (event: { payload: KernelEvent }) => void;
let channels: Map<string, (event: { payload: unknown }) => void>;

beforeEach(() => {
  localStorage.clear();
  localStorage.setItem(LLM_SETTINGS_KEY, JSON.stringify({ apiKey: 'test-key', baseUrl: 'http://localhost:1234/v1', model: 'test-model' }));
  channels = new Map();
  snapshot = { ready: true, mode: 'plan', activeRun: null, lastRun: null, sequence: 1 };
  history = [];
  sessions = [{ id: 1, title: '测试会话', updatedAt: 1, runCount: 0, projectId: 1, archived: false }];
  plans = [];
  mocks.listen.mockImplementation(async (name, callback) => {
    if (name === 'kernel-event') receive = callback;
    channels.set(name, callback);
    return mocks.stop;
  });
  mocks.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case 'get_snapshot': return structuredClone(snapshot);
      case 'get_workspace_info': return { name: 'Actual Project', path: 'E:/actual-project' };
      case 'select_workspace': return { name: 'Selected Project', path: args!.workspacePath };
      case 'read_directory_tree': return [
        { name: 'empty', path: 'empty', is_dir: true, children: [] },
        { name: 'src', path: 'src', is_dir: true, children: [{ name: 'unique.rs', path: 'src/unique.rs', is_dir: false, children: [] }] },
      ];
      case 'read_workspace_text': return 'actual source from Rust';
      case 'list_runs': return structuredClone(history);
      case 'list_sessions': return structuredClone(sessions);
      case 'list_projects': return [{ id: 1, name: 'Maestro-Agent', path: null, archived: false, createdAt: 1 }];
      case 'get_session': return structuredClone({
        session: sessions.find((session) => session.id === args!.sessionId),
        runs: history.filter((run) => run.sessionId === args!.sessionId),
        plans: plans.filter((plan) => plan.sessionId === args!.sessionId),
      });
      case 'create_session': {
        const session = { id: sessions.length + 1, title: '新会话', updatedAt: 2, runCount: 0, projectId: Number(args?.projectId) || 1, archived: false };
        sessions = [session, ...sessions];
        return session;
      }
      case 'decide_plan': {
        const plan = plans.find((item) => item.id === args!.planId && item.sessionId === args!.sessionId)!;
        if (plan.status !== 'pending') throw new Error('Plan already decided');
        plan.status = args!.decision === 'approve' ? 'approved' : 'rejected';
        snapshot = { ...snapshot, sequence: snapshot.sequence + 1 };
        receive({ payload: { type: 'plan_updated', plan: structuredClone(plan), sequence: snapshot.sequence } });
        return structuredClone(plan);
      }
      case 'set_mode': {
        snapshot = { ...snapshot, mode: args!.mode as Snapshot['mode'], sequence: snapshot.sequence + 1 };
        receive({ payload: { type: 'mode_changed', mode: snapshot.mode, sequence: snapshot.sequence } });
        return;
      }
      case 'stream_chat': {
        const messages = args!.messages as ChatMessage[];
        const run: RunView = { id: 7, sessionId: Number(args!.session_id), prompt: messages.at(-1)!.content, mode: snapshot.mode, status: 'running', output: '', error: null };
        history = [...history, run];
        snapshot = { ...snapshot, activeRun: run, sequence: snapshot.sequence + 1 };
        receive({ payload: { type: 'status', run, sequence: snapshot.sequence } });
        return 7;
      }
      case 'cancel_agent': return;
      default: throw new Error(`Unexpected command ${command}`);
    }
  });
});

async function ready() {
  await screen.findByText('Rust 内核已就绪');
  await screen.findByRole('button', { name: /测试会话/ });
}

describe('desktop IPC interface', () => {
  it('renders the product and subscribes before querying the snapshot', async () => {
    render(<App />);
    await ready();
    expect(screen.getByRole('heading', { name: 'Maestro-Agent' })).toBeInTheDocument();
    expect(screen.getByLabelText('会话列表')).toBeInTheDocument();
    expect(screen.getByLabelText('Agent 对话')).toBeInTheDocument();
    expect(screen.getByLabelText('Diff 审查面板')).toBeInTheDocument();
    expect(screen.queryByText('架构边界')).not.toBeInTheDocument();
    expect(screen.getByLabelText('由 Rust 强制执行')).toHaveTextContent('Plan');
    expect(mocks.listen.mock.invocationCallOrder[0]).toBeLessThan(mocks.invoke.mock.invocationCallOrder[0]);
  });

  it('starts via invoke, renders tokens once, and cancels the matching run', async () => {
    render(<App />);
    await ready();
    fireEvent.change(screen.getByLabelText('发送消息'), { target: { value: 'a request' } });
    fireEvent.click(screen.getByRole('button', { name: '发送' }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('stream_chat', { session_id: 1, messages: [{ role: 'user', content: 'a request' }], api_key: 'test-key', base_url: 'http://localhost:1234/v1', model: 'test-model' }));
    await waitFor(() => expect(screen.getByRole('button', { name: '停止生成' })).toBeEnabled());
    act(() => {
      receive({ payload: { type: 'content', runId: 7, sessionId: 1, token: '演示 token', sequence: 3 } });
      receive({ payload: { type: 'content', runId: 7, sessionId: 1, token: '演示 token', sequence: 3 } });
    });
    expect(screen.getByLabelText('回复内容 7')).toHaveTextContent(/^演示 token$/);
    expect(screen.getByLabelText('由 Rust 强制执行')).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: '停止生成' }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('cancel_agent', { runId: 7 }));
  });

  it('refreshes persisted history after a terminal event', async () => {
    render(<App />);
    await ready();
    const run: RunView = { id: 10, sessionId: 1, prompt: 'saved request', mode: 'plan', status: 'completed', output: 'persisted output', error: null };
    history = [run];
    act(() => receive({ payload: { type: 'status', run, sequence: 2 } }));
    await screen.findByText('saved request');
    expect(screen.getByLabelText('回复内容 10')).toHaveTextContent('persisted output');
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('get_session', { sessionId: 1 }));
  });

  it('only changes mode through a Rust command', async () => {
    render(<App />);
    await ready();
    fireEvent.keyDown(screen.getByLabelText('由 Rust 强制执行'), { key: 'ArrowDown' });
    fireEvent.click(await screen.findByRole('option', { name: /Auto/ }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('set_mode', { mode: 'auto' }));
    expect(screen.getByLabelText('由 Rust 强制执行')).toHaveTextContent('Auto');
  });

  it('shows a clear error rather than faking a backend in a plain browser', async () => {
    mocks.listen.mockRejectedValueOnce(new Error('IPC unavailable'));
    render(<App />);
    expect(await screen.findByRole('alert')).toHaveTextContent('npm run tauri dev');
    expect(screen.getByRole('button', { name: '发送' })).toBeDisabled();
  });

  it('buffers tokens arriving before the initial snapshot reply on reload', async () => {
    let resolveSnapshot!: (value: Snapshot) => void;
    const pendingSnapshot = new Promise<Snapshot>((resolve) => { resolveSnapshot = resolve; });
    mocks.invoke.mockImplementationOnce(() => pendingSnapshot);
    render(<App />);
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('get_snapshot'));
    act(() => receive({ payload: { type: 'content', runId: 7, sessionId: 1, token: ' + new', sequence: 6 } }));
    await act(async () => resolveSnapshot({
      ...snapshot, mode: 'code', sequence: 5,
      activeRun: { id: 7, sessionId: 1, prompt: 'reload', mode: 'execute', status: 'running', output: 'baseline', error: null },
    }));
    await ready();
    expect(screen.getByLabelText('回复内容 7')).toHaveTextContent('baseline + new');
    expect(screen.getByLabelText('由 Rust 强制执行')).toHaveTextContent('Code');
  });

  it('cleans up both StrictMode listener registrations', async () => {
    const view = render(<StrictMode><App /></StrictMode>);
    await ready();
    expect(mocks.listen.mock.calls.filter(([name]) => name === 'kernel-event')).toHaveLength(2);
    view.unmount();
    expect(mocks.stop.mock.calls.length).toBe(mocks.listen.mock.calls.length);
  });

  it('creates and selects a durable session', async () => {
    render(<App />);
    await ready();
    fireEvent.click(screen.getByRole('button', { name: /新建会话/ }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('create_session', { projectId: 1 }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('get_session', { sessionId: 2 }));
    expect(screen.getByRole('button', { name: /新会话.*0 轮对话/ })).toHaveAttribute('aria-pressed', 'true');
  });

  it('approves a specific plan, preserves Plan Mode, and never invokes execution', async () => {
    const run: RunView = { id: 3, sessionId: 1, prompt: 'request', mode: 'plan', status: 'completed', output: 'answer', error: null };
    history = [run];
    plans = [{ id: 9, sessionId: 1, runId: 3, status: 'pending', draft: {
      title: '实现计划', summary: 'proposed only', steps: ['review', 'test'],
      diffs: [{ path: 'docs/plan.md', before: 'old line\n', after: 'new line\n' }],
    } }];
    render(<App />);
    await ready();
    const approve = await screen.findByRole('button', { name: '批准计划' });
    expect(screen.getByLabelText('Diff docs/plan.md')).toHaveTextContent('new line');
    expect(screen.getByLabelText('Diff docs/plan.md')).toHaveTextContent('old line');
    fireEvent.click(approve);
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('decide_plan', { sessionId: 1, planId: 9, decision: 'approve' }));
    await screen.findByText('计划已批准，尚未执行任何修改。');
    expect(screen.getByLabelText('由 Rust 强制执行')).toHaveTextContent('Plan');
    expect(mocks.invoke).not.toHaveBeenCalledWith('set_mode', { mode: 'execute' });
    expect(screen.queryByRole('button', { name: '批准计划' })).not.toBeInTheDocument();
  });

  it('rejects a plan and disables decision controls before its run completes', async () => {
    const run: RunView = { id: 3, sessionId: 1, prompt: 'request', mode: 'plan', status: 'running', output: 'partial', error: null };
    history = [run];
    snapshot = { ...snapshot, activeRun: run };
    plans = [{ id: 9, sessionId: 1, runId: 3, status: 'pending', draft: { title: '实现计划', summary: 'proposal', steps: ['review'], diffs: [] } }];
    render(<App />);
    await ready();
    expect(await screen.findByRole('button', { name: '批准计划' })).toBeDisabled();
    history = [{ ...run, status: 'completed' }];
    snapshot = { ...snapshot, activeRun: null, lastRun: history[0], sequence: 2 };
    act(() => receive({ payload: { type: 'status', run: history[0], sequence: 2 } }));
    await waitFor(() => expect(screen.getByRole('button', { name: '拒绝计划' })).toBeEnabled());
    fireEvent.click(screen.getByRole('button', { name: '拒绝计划' }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('decide_plan', { sessionId: 1, planId: 9, decision: 'reject' }));
    await waitFor(() => expect(screen.getByLabelText('计划审批 9')).toHaveTextContent('已拒绝'));
  });

  it('ignores an old session reply after switching sessions', async () => {
    const other = { id: 2, title: '会话二', updatedAt: 2, runCount: 1, projectId: 1, archived: false };
    sessions.push(other);
    const otherRun: RunView = { id: 20, sessionId: 2, prompt: 'second', mode: 'plan', status: 'completed', output: 'other session reply', error: null };
    history = [otherRun];
    let resolveOld!: (detail: SessionDetail) => void;
    const oldReply = new Promise<SessionDetail>((resolve) => { resolveOld = resolve; });
    const original = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === 'get_session' && args!.sessionId === 1) return oldReply;
      return original(command, args);
    });
    render(<App />);
    await ready();
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('get_session', { sessionId: 1 }));
    fireEvent.click(screen.getByRole('button', { name: /会话二/ }));
    await screen.findByLabelText('回复内容 20');
    await act(async () => resolveOld({ session: sessions[0], runs: [], plans: [] }));
    expect(screen.getByLabelText('回复内容 20')).toHaveTextContent('other session reply');
    expect(screen.getByRole('button', { name: /会话二/ })).toHaveAttribute('aria-pressed', 'true');
  });

  it('does not let a delayed new-session reply replace a newer user selection', async () => {
    sessions.push({ id: 2, title: '会话二', updatedAt: 2, runCount: 0, projectId: 1, archived: false });
    let resolveNew!: (session: SessionView) => void;
    const creation = new Promise<SessionView>((resolve) => { resolveNew = resolve; });
    const original = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => command === 'create_session' ? creation : original(command, args));
    render(<App />);
    await ready();
    fireEvent.click(screen.getByRole('button', { name: /新建会话/ }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('create_session', { projectId: 1 }));
    fireEvent.click(screen.getByRole('button', { name: /会话二/ }));
    const created = { id: 3, title: '新会话', updatedAt: 3, runCount: 0, projectId: 1, archived: false };
    sessions.push(created);
    await act(async () => resolveNew(created));
    await screen.findByRole('button', { name: /新会话.*0 轮对话/ });
    expect(screen.getByRole('button', { name: /会话二/ })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('button', { name: /新会话.*0 轮对话/ })).toHaveAttribute('aria-pressed', 'false');
  });

  it('keeps a live connection but presents dashboard data as Mock without additional IPC', async () => {
    render(<App />);
    await ready();
    await waitFor(() => expect(screen.getByLabelText('发送消息')).toBeInTheDocument());
    fireEvent.change(screen.getByLabelText('发送消息'), { target: { value: 'keep this draft' } });
    const callCount = mocks.invoke.mock.calls.length;
    fireEvent.click(screen.getByRole('button', { name: 'Activity' }));
    await screen.findByRole('region', { name: 'Activity Dashboard' }, { timeout: 8000 });
    expect(screen.getByLabelText('Total Lines（Mock）')).toHaveTextContent('12,450');
    expect(screen.getByText('Rust 内核已就绪')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(mocks.invoke.mock.calls.length).toBe(callCount);
    fireEvent.click(screen.getByRole('button', { name: 'Back to Chat' }));
    expect(screen.getByLabelText('发送消息')).toHaveValue('keep this draft');
  });

  it('keeps receiving stream events while dashboard is displayed', async () => {
    render(<App />);
    await ready();
    fireEvent.change(screen.getByLabelText('发送消息'), { target: { value: 'stream in background' } });
    fireEvent.click(screen.getByRole('button', { name: '发送' }));
    await waitFor(() => expect(screen.getByRole('button', { name: '停止生成' })).toBeEnabled());
    const listeners = mocks.listen.mock.calls.length;
    fireEvent.click(screen.getByRole('button', { name: 'Activity' }));
    await screen.findByRole('region', { name: 'Activity Dashboard' }, { timeout: 8000 });
    act(() => receive({ payload: { type: 'content', runId: 7, sessionId: 1, token: 'arrived while dashboard was open', sequence: 3 } }));
    fireEvent.click(screen.getByRole('button', { name: 'Back to Chat' }));
    expect(screen.getByLabelText('回复内容 7')).toHaveTextContent('arrived while dashboard was open');
    expect(mocks.listen.mock.calls.length).toBe(listeners);
  });

  it('persists real Settings with a masked key and uses Ctrl+Enter to invoke stream_chat', async () => {
    render(<App />);
    await ready();
    localStorage.setItem('maestro.llm.models.openai', JSON.stringify([{ id: 'configured-model', enabled: true }]));
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    fireEvent.click(screen.getByRole('button', { name: 'Models' }));
    expect(screen.getByLabelText('API Key')).toHaveAttribute('type', 'password');
    fireEvent.change(screen.getByLabelText('API Key'), { target: { value: 'new-test-key' } });
    fireEvent.change(screen.getByLabelText('Base URL'), { target: { value: 'https://example.test/v1' } });
    fireEvent.click(screen.getByRole('button', { name: '设为当前模型 configured-model' }));
    fireEvent.click(screen.getByRole('button', { name: '保存设置' }));
    expect(JSON.parse(localStorage.getItem(LLM_SETTINGS_KEY)!)).toMatchObject({ apiKey: 'new-test-key', model: 'configured-model' });
    fireEvent.change(screen.getByLabelText('发送消息'), { target: { value: 'configured request' } });
    fireEvent.keyDown(screen.getByLabelText('发送消息'), { key: 'Enter', ctrlKey: true });
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('stream_chat', expect.objectContaining({ api_key: 'new-test-key', base_url: 'https://example.test/v1', model: 'configured-model' })));
    expect(screen.getByLabelText('发送消息')).toHaveValue('');
  });

  it('appends chat-token once and handles chat-error as one terminal Toast', async () => {
    render(<App />);
    await ready();
    fireEvent.change(screen.getByLabelText('发送消息'), { target: { value: 'stream request' } });
    fireEvent.click(screen.getByRole('button', { name: '发送' }));
    await waitFor(() => expect(screen.getByRole('button', { name: '停止生成' })).toBeEnabled());
    act(() => {
      receive({ payload: { type: 'content', runId: 7, sessionId: 1, token: 'Hello ', sequence: 3 } });
      receive({ payload: { type: 'content', runId: 7, sessionId: 1, token: 'world', sequence: 4 } });
      receive({ payload: { type: 'content', runId: 7, sessionId: 1, token: 'world', sequence: 4 } });
    });
    expect(screen.getByLabelText('回复内容 7')).toHaveTextContent('Hello world');
    const failed: RunView = { ...history[0], status: 'failed', output: 'Hello world', error: 'provider failed' };
    history = [failed];
    snapshot = { ...snapshot, activeRun: null, lastRun: failed, sequence: 5 };
    act(() => {
      receive({ payload: { type: 'status', sequence: 5, run: failed } });
    });
    expect(await screen.findByRole('alert')).toHaveTextContent('provider failed');
    expect(screen.getAllByRole('alert')).toHaveLength(1);
    expect(screen.queryByRole('button', { name: '停止生成' })).not.toBeInTheDocument();
    expect(screen.getByText('Rust 内核已就绪')).toBeInTheDocument();
  });

  it('keeps early tokens and chat-end before invoke resolves without duplicate final output', async () => {
    let accept!: (id: number) => void;
    const accepted = new Promise<number>((resolve) => { accept = resolve; });
    const original = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === 'stream_chat') {
        void original(command, args);
        receive({ payload: { type: 'content', runId: 7, sessionId: 1, token: 'early response', sequence: 3 } });
        return accepted;
      }
      return original(command, args);
    });
    render(<App />);
    await ready();
    fireEvent.change(screen.getByLabelText('发送消息'), { target: { value: 'early request' } });
    fireEvent.click(screen.getByRole('button', { name: '发送' }));
    expect(await screen.findByLabelText('回复内容 7')).toHaveTextContent('early response');
    const completed: RunView = { ...history[0], status: 'completed', output: 'early response' };
    history = [completed];
    snapshot = { ...snapshot, activeRun: null, lastRun: completed, sequence: 4 };
    act(() => receive({ payload: { type: 'status', sequence: 4, run: completed } }));
    await act(async () => accept(7));
    expect(screen.getByLabelText('回复内容 7')).toHaveTextContent(/^early response$/);
    expect(screen.getByLabelText('回复内容 7')).toHaveAttribute('aria-busy', 'false');
    expect(screen.queryByRole('button', { name: '停止生成' })).not.toBeInTheDocument();
  });

  it('requires saved settings before requesting a model and restores input on validation failure', async () => {
    localStorage.removeItem(LLM_SETTINGS_KEY);
    render(<App />);
    await ready();
    fireEvent.change(screen.getByLabelText('发送消息'), { target: { value: 'keep invalid request' } });
    fireEvent.click(screen.getByRole('button', { name: '发送' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('API Key');
    expect(screen.getByLabelText('发送消息')).toHaveValue('keep invalid request');
    expect(mocks.invoke.mock.calls.some(([command]) => command === 'stream_chat')).toBe(false);
  });

  it('persists model thinking strength while keeping unsupported parameters out of IPC', async () => {
    localStorage.setItem('maestro.llm.models.openai', JSON.stringify([{ id: 'gpt-5.2', enabled: true }]));
    render(<App />);
    await ready();
    fireEvent.click(screen.getByLabelText('选择模型'));
    fireEvent.click(screen.getByRole('button', { name: /gpt-5\.2/ }));
    expect(JSON.parse(localStorage.getItem(LLM_SETTINGS_KEY)!)).toMatchObject({ apiKey: 'test-key', baseUrl: 'http://localhost:1234/v1', model: 'gpt-5.2' });
    fireEvent.keyDown(screen.getByRole('slider'), { key: 'ArrowRight' });
    fireEvent.change(screen.getByLabelText('发送消息'), { target: { value: 'model selection request' } });
    fireEvent.click(screen.getByRole('button', { name: '发送' }));
    await waitFor(() => expect(mocks.invoke.mock.calls.some(([command]) => command === 'stream_chat')).toBe(true));
    const payload = mocks.invoke.mock.calls.find(([command]) => command === 'stream_chat')![1];
    expect(payload).toMatchObject({ model: 'gpt-5.2', api_key: 'test-key', reasoning_effort: 'high' });
    expect(payload).not.toHaveProperty('temperature');
    expect(payload).not.toHaveProperty('context_window');
  });

  it('loads the actual directory tree and file text only when live Explorer is opened', async () => {
    render(<App />);
    await ready();
    expect(mocks.invoke.mock.calls.some(([name]) => name === 'read_directory_tree')).toBe(false);
    const footer = document.querySelector('.sidebar-bottom')!;
    expect(footer.contains(screen.getByRole('button', { name: 'Settings' }))).toBe(true);
    fireEvent.keyDown(screen.getByRole('tab', { name: 'Explorer' }), { key: 'Enter' });
    const file = await screen.findByRole('button', { name: '文件 src/unique.rs' });
    expect(mocks.invoke).toHaveBeenCalledWith('read_directory_tree', { workspacePath: 'E:/actual-project' });
    expect(screen.queryByRole('button', { name: /BridgeSession.cpp/ })).not.toBeInTheDocument();
    expect(screen.getByText('Empty folder')).toBeInTheDocument();
    fireEvent.click(file);
    expect(await screen.findByLabelText('文件预览 src/unique.rs')).toHaveTextContent('actual source from Rust');
    expect(mocks.invoke).toHaveBeenCalledWith('read_workspace_text', { workspacePath: 'E:/actual-project', path: 'src/unique.rs' });
    fireEvent.keyDown(screen.getByRole('tab', { name: 'Review' }), { key: 'Enter' });
    fireEvent.keyDown(screen.getByRole('tab', { name: 'Explorer' }), { key: 'Enter' });
    expect(screen.getByLabelText('文件预览 src/unique.rs')).toHaveTextContent('actual source from Rust');
  });

  it('keeps sent attachments as local bubble placeholders without uploading them', async () => {
    Object.defineProperty(URL, 'createObjectURL', { configurable: true, value: vi.fn(() => 'blob:attachment-local') });
    Object.defineProperty(URL, 'revokeObjectURL', { configurable: true, value: vi.fn() });
    render(<App />);
    await ready();
    fireEvent.change(screen.getByLabelText('选择本地媒体附件'), { target: { files: [new File(['local-image'], 'attachment.png', { type: 'image/png' })] } });
    fireEvent.change(screen.getByLabelText('发送消息'), { target: { value: 'text request with local placeholder' } });
    fireEvent.click(screen.getByRole('button', { name: '发送' }));
    await waitFor(() => expect(screen.getByRole('button', { name: '停止生成' })).toBeEnabled());
    const image = await screen.findByRole('img', { name: 'attachment.png' });
    expect(image.closest('.user-message')).not.toBeNull();
    expect(image.closest('.user-message')?.querySelector('.sent-attachments')).not.toBeNull();
    const payload = mocks.invoke.mock.calls.find(([command]) => command === 'stream_chat')![1];
    expect(JSON.stringify(payload)).not.toContain('attachment.png');
    expect(JSON.stringify(payload)).not.toContain('blob:');
  });
});
