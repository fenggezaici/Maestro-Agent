import * as Tooltip from '@radix-ui/react-tooltip';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import ComposerControls from './ComposerControls';
import LlmSettingsForm from './LlmSettingsForm';
import ModelCapabilityControls from './ModelCapabilityControls';
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
  fireEvent.click(screen.getByLabelText('选择模型'));
  expect(screen.getByText('内置推理（模型固定）')).toBeVisible();
  expect(screen.queryByLabelText('思考强度')).not.toBeInTheDocument();
  expect(screen.queryByText('Unknown context')).not.toBeInTheDocument();
});

it('changes from fixed R1 to adjustable gpt-5.2, persisting effort that stays supported on gpt-5', () => {
  saveLlmSettings(config);
  localStorage.setItem('maestro.llm.models.openai', JSON.stringify([
    { id: 'deepseek-r1', enabled: true }, { id: 'gpt-5.2', enabled: true }, { id: 'gpt-5', enabled: true },
  ]));
  composer();
  fireEvent.click(screen.getByLabelText('选择模型'));
  fireEvent.click(screen.getByRole('button', { name: /gpt-5\.2/ }));
  expect(screen.queryByText('内置推理（模型固定）')).not.toBeInTheDocument();
  const slider = screen.getByRole('slider');
  expect(slider).toHaveAttribute('aria-valuenow', '1');
  fireEvent.keyDown(slider, { key: 'End' });
  expect(readLlmSettings()).toMatchObject({ model: 'gpt-5.2', reasoning: 'xhigh' });
  fireEvent.click(screen.getByRole('button', { name: /gpt-5(?!\.)/ }));
  expect(readLlmSettings()).toMatchObject({ model: 'gpt-5', reasoning: 'medium' });
});

it('keeps the previous thinking selection if localStorage rejects a save', () => {
  saveLlmSettings({ ...config, model: 'gpt-5.2', reasoning: 'medium' });
  const onError = vi.fn();
  composer(onError);
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('blocked'); });
  fireEvent.click(screen.getByLabelText('选择模型'));
  fireEvent.keyDown(screen.getByRole('slider'), { key: 'ArrowRight' });
  expect(onError).toHaveBeenCalledWith(expect.stringContaining('浏览器存储不可用'));
  expect(screen.getByRole('slider')).toHaveAttribute('aria-valuenow', '1');
  expect(readLlmSettings().reasoning).toBe('medium');
});

it('explains Ollama deployment context without losing the R1 thinking indicator', () => {
  saveLlmSettings({ ...config, apiKey: '', baseUrl: 'http://localhost:11434/v1', provider: 'ollama' });
  composer();
  fireEvent.click(screen.getByLabelText('选择模型'));
  expect(screen.getByText('内置推理（模型固定）')).toBeVisible();
});

it('shows supported thinking strength in the detailed variant, and updates capability when the model changes', () => {
  const onReasoning = vi.fn();
  const { rerender } = render(<Tooltip.Provider><ModelCapabilityControls model="gpt-5.2" provider="openai" reasoning="low" onReasoning={onReasoning} detailed /></Tooltip.Provider>);
  expect(screen.getByLabelText('思考强度')).toHaveTextContent('Low');
  fireEvent.keyDown(screen.getByLabelText('思考强度'), { key: 'ArrowDown' });
  fireEvent.click(screen.getByRole('option', { name: 'High' }));
  expect(onReasoning).toHaveBeenCalledWith('high');
  rerender(<Tooltip.Provider><ModelCapabilityControls model="deepseek-r1" provider="deepseek" reasoning="low" onReasoning={onReasoning} detailed /></Tooltip.Provider>);
  expect(screen.getByLabelText('思考模式：模型内置推理')).toBeVisible();
  expect(screen.queryByLabelText('思考强度')).not.toBeInTheDocument();
});
