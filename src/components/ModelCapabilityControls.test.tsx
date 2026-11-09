import * as Tooltip from '@radix-ui/react-tooltip';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import ComposerControls from './ComposerControls';
import LlmSettingsForm from './LlmSettingsForm';
import { readLlmSettings, saveLlmSettings } from '../settings';
import { defaultUiPreferences } from '../uiPreferences';

const config = { apiKey: 'test-key', baseUrl: 'https://gateway.example/v1', model: 'deepseek-r1' };
beforeEach(() => localStorage.clear());
afterEach(() => vi.restoreAllMocks());

function composer(onError = vi.fn()) {
  return render(<Tooltip.Provider><ComposerControls preview={false} disabled={false} preferences={defaultUiPreferences}
    onPreferences={vi.fn()} settingsVersion={0} onError={onError} onCapability={vi.fn()} /></Tooltip.Provider>);
}

it('shows R1 context and a visible built-in thinking indicator without fake Low/High options', () => {
  saveLlmSettings(config);
  composer();
  expect(screen.getByLabelText('选择模型')).toHaveTextContent('deepseek-r1');
  expect(screen.getByLabelText('上下文长度')).toHaveTextContent('128k context');
  expect(screen.getByLabelText('上下文长度')).toHaveAttribute('title', expect.stringContaining('128,000 tokens'));
  expect(screen.getByLabelText('思考模式：模型内置推理')).toBeVisible();
  expect(screen.queryByLabelText('思考强度')).not.toBeInTheDocument();
  expect(screen.queryByText('Unknown context')).not.toBeInTheDocument();
});

it('changes from fixed R1 to adjustable gpt-5.2, persisting effort that stays supported on gpt-5', () => {
  saveLlmSettings(config);
  composer();
  fireEvent.keyDown(screen.getByLabelText('选择模型'), { key: 'ArrowDown' });
  fireEvent.click(screen.getByRole('option', { name: /gpt-5\.2/ }));
  expect(screen.getByLabelText('上下文长度')).toHaveTextContent('400k context');
  expect(screen.queryByLabelText('思考模式：模型内置推理')).not.toBeInTheDocument();
  fireEvent.keyDown(screen.getByLabelText('思考强度'), { key: 'ArrowDown' });
  fireEvent.click(screen.getByRole('option', { name: 'Extra High' }));
  expect(readLlmSettings()).toMatchObject({ model: 'gpt-5.2', reasoning: 'xhigh' });
  fireEvent.keyDown(screen.getByLabelText('选择模型'), { key: 'ArrowDown' });
  fireEvent.click(screen.getByRole('option', { name: /^gpt-5(?!\.)/ }));
  expect(readLlmSettings()).toMatchObject({ model: 'gpt-5', reasoning: 'medium' });
});

it('keeps the previous thinking selection if localStorage rejects a save', () => {
  saveLlmSettings({ ...config, model: 'gpt-5.2', reasoning: 'medium' });
  const onError = vi.fn();
  composer(onError);
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('blocked'); });
  fireEvent.keyDown(screen.getByLabelText('思考强度'), { key: 'ArrowDown' });
  fireEvent.click(screen.getByRole('option', { name: 'High' }));
  expect(onError).toHaveBeenCalledWith(expect.stringContaining('浏览器存储不可用'));
  expect(screen.getByLabelText('思考强度')).toHaveTextContent('Medium');
  expect(readLlmSettings().reasoning).toBe('medium');
});

it('explains Ollama deployment context without losing the R1 thinking indicator', () => {
  saveLlmSettings({ ...config, apiKey: '', baseUrl: 'http://localhost:11434/v1', provider: 'ollama' });
  composer();
  expect(screen.getByLabelText('上下文长度')).toHaveTextContent('服务端配置');
  expect(screen.getByLabelText('上下文长度')).toHaveAttribute('title', expect.stringContaining('num_ctx'));
  expect(screen.getByLabelText('思考模式：模型内置推理')).toBeVisible();
});

it('shows and saves supported thinking strength in Settings, and updates capability when the model changes', () => {
  saveLlmSettings({ ...config, model: 'gpt-5.2', reasoning: 'low' });
  const onSaved = vi.fn();
  render(<Tooltip.Provider><LlmSettingsForm preview={false} onSaved={onSaved} /></Tooltip.Provider>);
  const summary = screen.getByRole('group', { name: '模型能力' });
  expect(within(summary).getByLabelText('思考强度')).toHaveTextContent('Low');
  fireEvent.keyDown(within(summary).getByLabelText('思考强度'), { key: 'ArrowDown' });
  fireEvent.click(screen.getByRole('option', { name: 'High' }));
  fireEvent.click(screen.getByRole('button', { name: '保存设置' }));
  expect(onSaved).toHaveBeenCalledOnce();
  expect(readLlmSettings()).toMatchObject({ model: 'gpt-5.2', reasoning: 'high' });
  fireEvent.change(screen.getByLabelText('Model'), { target: { value: 'deepseek-r1' } });
  expect(within(summary).getByLabelText('上下文长度')).toHaveTextContent('128k context');
  expect(within(summary).getByLabelText('思考模式：模型内置推理')).toBeVisible();
  expect(within(summary).queryByLabelText('思考强度')).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: '保存设置' }));
  expect(readLlmSettings()).toMatchObject({ model: 'deepseek-r1' });
  expect(readLlmSettings().reasoning).toBeUndefined();
});
