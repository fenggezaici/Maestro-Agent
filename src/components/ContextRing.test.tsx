import { render, screen } from '@testing-library/react';
import * as Tooltip from '@radix-ui/react-tooltip';
import { expect, it } from 'vitest';
import ContextRing from './ContextRing';

const ring = (used: number | null, context: number | null) =>
  render(<Tooltip.Provider><ContextRing used={used} context={context} /></Tooltip.Provider>);

it('keeps the last value when a streaming frame has no usage yet', () => {
  const { rerender } = ring(400_000, 1_000_000);
  expect(screen.getByRole('img', { name: '上下文已用 40%' })).toBeInTheDocument();

  rerender(<Tooltip.Provider><ContextRing used={null} context={1_000_000} /></Tooltip.Provider>);
  expect(screen.getByRole('img', { name: '上下文已用 40%' })).toHaveAttribute('data-stale', 'true');
});

it('renders nothing before any value is known', () => {
  ring(null, null);
  expect(screen.queryByRole('img')).not.toBeInTheDocument();
});

it('keeps one decimal so small real usage does not display as 0%', () => {
  // ZCode Intl percent 格式：2K/1M = 0.2%，不得取整成 0%。
  ring(2_000, 1_000_000);
  expect(screen.getByRole('img', { name: '上下文已用 0.2%' })).toBeInTheDocument();
});
