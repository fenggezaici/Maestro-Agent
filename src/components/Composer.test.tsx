import { render, screen } from '@testing-library/react';
import * as Tooltip from '@radix-ui/react-tooltip';
import { describe, expect, it, vi } from 'vitest';
import Composer from './Composer';
import { defaultUiPreferences } from '../uiPreferences';
import type { RunView, RunMetrics } from '../api/types';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const project = { id: 'p1', name: 'Demo', path: null, sessions: [] };

const run: RunView = {
  id: 42,
  sessionId: 1,
  prompt: '任务',
  mode: 'plan',
  status: 'running',
  output: '',
  error: null,
  createdAt: Math.floor(Date.now() / 1000),
};

// 并行子智能体折入后的后端累加值——远超墙钟且在流间跳变。
const inflated: RunMetrics = {
  runId: 42, sessionId: 1, provider: 'openai', model: 'm',
  inputTokens: null, outputTokens: null, elapsedMs: 11 * 60 * 1000 + 28500, firstTokenMs: null,
};

function setup() {
  return render(<Tooltip.Provider><Composer
    atBottom onScrollBottom={() => {}} onSubmit={() => true}
    attachments={[]} onAttachmentsChange={() => {}} blocked={false}
    preview={false} mode="plan" onSetMode={() => {}}
    currentProject={project} projects={[project]} onNewProject={() => {}}
    preferences={defaultUiPreferences} onPreferences={() => {}} settingsVersion={0}
    onError={() => {}} onCapability={() => {}}
    running pending={false} onCancel={() => {}}
    runs={[run]} metrics={{ 42: inflated }} context={null} sessionId={1} modelLabel="m"
    status="thinking"
  /></Tooltip.Provider>);
}

describe('composer status pill wall-clock semantics', () => {
  it('ignores the parallel-inflated backend elapsed and shows honest wall clock', () => {
    setup();
    const pill = screen.getByRole('status');
    // 墙钟只走了零点几秒——后端累加的 11 分 28 秒绝不该出现在药丸上。
    expect(pill.textContent).not.toContain('分');
    expect(pill.textContent).toContain('思考中');
  });
});
