import { fireEvent, render, screen } from '@testing-library/react';
import * as Tooltip from '@radix-ui/react-tooltip';
import { expect, it, vi } from 'vitest';
import ActivityDashboard from './ActivityDashboard';
import { createMockDashboard } from './activityMock';

it('renders the aggregate metrics for the project and navigates back', () => {
  const onBack = vi.fn();
  const data = createMockDashboard({ id: 'p1' });
  // The dashboard uses Radix tooltips, which require a provider.
  render(<Tooltip.Provider><ActivityDashboard data={data} projectName="CheckInReminder" onBack={onBack} /></Tooltip.Provider>);

  expect(screen.getByText(/CheckInReminder/)).toBeInTheDocument();
  for (const label of ['Total Lines', 'Plans Approved', 'Tokens Used']) {
    expect(screen.getByText(label)).toBeInTheDocument();
  }
  fireEvent.click(screen.getByRole('button', { name: /返回|Back/i }));
  expect(onBack).toHaveBeenCalled();
});
