import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import LlmSettingsForm from './LlmSettingsForm';
import { LLM_SETTINGS_KEY } from '../settings';

const migrate = vi.fn();
vi.mock('../api/credentials', () => ({
  credentialsApi: { save: vi.fn(), status: vi.fn(async () => []), clear: vi.fn() },
  isLiveDesktop: () => true,
  refreshCredentialStatus: vi.fn(async () => {}),
  migrateLegacyCredentials: (...args: unknown[]) => migrate(...args),
}));

beforeEach(() => {
  localStorage.clear();
  migrate.mockReset();
});

const seedLegacyKey = () => localStorage.setItem(
  LLM_SETTINGS_KEY,
  JSON.stringify({ apiKey: 'legacy-key', baseUrl: 'https://example.test/v1', model: 'm' }),
);

it('offers a retry while legacy keys remain and surfaces the backend reason', async () => {
  seedLegacyKey();
  migrate.mockResolvedValue('部分 API Key 未能迁移：openai（API Key 无效）。已成功的已移入系统凭据，其余原值保留。');
  render(<LlmSettingsForm preview={false} onSaved={vi.fn()} />);

  expect(screen.getByText(/有 1 个旧 Key/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: '重试迁移' }));

  expect(await screen.findByText(/openai（API Key 无效）/)).toBeInTheDocument();
  expect(migrate).toHaveBeenCalledTimes(1);
});

it('hides migration controls in preview', () => {
  seedLegacyKey();
  render(<LlmSettingsForm preview onSaved={vi.fn()} />);
  expect(screen.queryByText(/旧 Key/)).not.toBeInTheDocument();
  expect(screen.queryByRole('button', { name: '重试迁移' })).not.toBeInTheDocument();
});

it('shows no migration banner once every legacy key is gone', () => {
  render(<LlmSettingsForm preview={false} onSaved={vi.fn()} />);
  expect(screen.queryByText(/旧 Key/)).not.toBeInTheDocument();
});
