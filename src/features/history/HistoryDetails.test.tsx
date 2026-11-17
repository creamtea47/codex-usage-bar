import { beforeEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { CycleList } from './HistoryDetails';
import i18n from '@/shared/i18n/i18n';
import type { CycleSummary } from '@/contracts/history';

const analytics = vi.hoisted(() => vi.fn());
vi.mock('./historyApi', () => ({ historyApi: { analytics } }));

beforeEach(async () => {
  await i18n.changeLanguage('en');
});
it('expands retained samples in pages and does not invent past exhaustion', async () => {
  const cycle: CycleSummary = {
    cycleId: 'cycle-one',
    startAt: '2030-01-01T00:00:00Z',
    endAt: '2030-01-02T00:00:00Z',
    resetAt: '2030-01-03T00:00:00Z',
    firstRemainingPercent: 100,
    lastRemainingPercent: 60,
    consumedPercent: 40,
    durationSeconds: 86400,
    sampleCount: 51,
    partial: false,
    outsideChartRange: false,
    current: false,
    firstExhaustedAt: null,
    bucketSeconds: 900,
    forecast: null,
  };
  analytics.mockImplementation(async (r) =>
    r.kind === 'cycles'
      ? [cycle]
      : {
          total: 51,
          offset: r.offset,
          bucketSeconds: 900,
          points: [
            {
              sampledAt: r.offset ? cycle.endAt : cycle.startAt,
              remainingPercent: r.offset ? 60 : 100,
              breakBefore: !r.offset,
            },
          ],
        },
  );
  render(
    <CycleList
      accountId="account-a"
      now={Date.parse(cycle.endAt)}
      windowId="weekly"
      request={{ kind: 'preset', preset: 'all' }}
    />,
  );
  await screen.findByText('Empty quota was not observed');
  fireEvent.click(screen.getByRole('button', { name: /100% → 60%/ }));
  await screen.findByRole('table', { name: 'Sample details' });
  fireEvent.click(screen.getByRole('button', { name: 'Next' }));
  await waitFor(() =>
    expect(analytics).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'samples', cycleId: 'cycle-one', offset: 50 }),
      'account-a',
    ),
  );
});
