import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import TrendsErrorBoundary from './TrendsErrorBoundary';

const bridgeMocks = vi.hoisted(() => ({ reportSettingsUiFault: vi.fn() }));

vi.mock('./bridge', () => ({ usageBridge: bridgeMocks }));
vi.mock('react-i18next', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react-i18next')>();
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string) =>
        ({
          'trends.renderError': 'The Trends page is temporarily unavailable.',
          'trends.retry': 'Retry',
        })[key] ?? key,
    }),
  };
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.clearAllMocks();
});

describe('TrendsErrorBoundary', () => {
  it('reports only the fixed fault code and remounts the content on retry', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    bridgeMocks.reportSettingsUiFault.mockRejectedValue(new Error('reporting unavailable'));
    let shouldThrow = true;

    function FlakyTrends() {
      if (shouldThrow) {
        throw new Error('private render details');
      }
      return <div>Recovered trends</div>;
    }

    render(
      <aside aria-label="settings sidebar">
        Sidebar remains
        <TrendsErrorBoundary>
          <FlakyTrends />
        </TrendsErrorBoundary>
      </aside>,
    );

    expect(await screen.findByText('The Trends page is temporarily unavailable.')).toBeTruthy();
    expect(screen.getByText('Sidebar remains')).toBeTruthy();
    await waitFor(() =>
      expect(bridgeMocks.reportSettingsUiFault).toHaveBeenCalledWith('trends-render-failed'),
    );
    expect(screen.queryByText('private render details')).toBeNull();

    shouldThrow = false;
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(await screen.findByText('Recovered trends')).toBeTruthy();
    expect(bridgeMocks.reportSettingsUiFault).toHaveBeenCalledTimes(1);
  });
});
