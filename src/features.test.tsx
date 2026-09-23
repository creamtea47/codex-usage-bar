import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { forecastText, minuteDuration } from './forecastPresentation';
import { ResponsePanel } from './ResponsePanel';
import { CycleList } from './HistoryDetails';
import i18n from './i18n';
import type { CycleSummary } from './accountTypes';
import type { QuotaForecast } from './types';

const analytics = vi.hoisted(() => vi.fn());
vi.mock('./accountsBridge', () => ({ currentAccountId: () => 'account-a', accountsBridge: { analytics } }));

describe('usage details', () => {
  beforeEach(async () => { analytics.mockReset(); await i18n.changeLanguage('en'); });

  it('uses reset minus exhaustion and minute precision in both languages', () => {
    const now = Date.parse('2030-01-01T00:00:00Z');
    const forecast: QuotaForecast = { status: 'exhaustsBeforeReset', exhaustsAt: '2030-01-01T12:00:00Z', sampleCount: 8, observedSpanSeconds: 3600, consumedPercent: 12 };
    expect(forecastText(forecast, '2030-01-04T21:31:00Z', now, 'zh-CN')).toContain('约 12小时后，比重置提前 3天9小时31分');
    expect(forecastText(forecast, '2030-01-04T21:31:00Z', now, 'en')).toContain('3d 9h 31m before reset');
    expect(minuteDuration(59, 'en')).toBe('0m');
    expect(forecastText({ ...forecast, status: 'collecting' }, null, now, 'en')).toBe('Insufficient samples');
  });

  it('shows response evidence as text without interpreting model markup', () => {
    const { container } = render(<ResponsePanel automatic={false} result={{ attemptedAt: null, successAt: null, errorCode: 'network', model: 'requested', slotIndex: null,
      details: { startedAt: null, responseModel: 'reported', httpStatus: 200, durationMs: 85, text: '<script>unsafe()</script>', truncated: true, errorMessage: 'Disconnected', deliveryUncertain: true } }} />);
    expect(screen.getByText('<script>unsafe()</script>')).toBeTruthy();
    expect(container.querySelector('script')).toBeNull();
    expect(screen.getByText('Delivery is uncertain. Automatic retry stopped.')).toBeTruthy();
    expect(screen.getByText(/Response model: reported/)).toBeTruthy();
    expect(screen.getByText('Text was truncated.')).toBeTruthy();
  });

  it('expands retained samples in pages and does not invent past exhaustion', async () => {
    const cycle: CycleSummary = { cycleId: 'cycle-one', startAt: '2030-01-01T00:00:00Z', endAt: '2030-01-02T00:00:00Z', resetAt: '2030-01-03T00:00:00Z', firstRemainingPercent: 100, lastRemainingPercent: 60, consumedPercent: 40, durationSeconds: 86400, sampleCount: 51, partial: false, outsideChartRange: false, current: false, firstExhaustedAt: null, bucketSeconds: 900, forecast: null };
    analytics.mockImplementation(async (r) => r.kind === 'cycles' ? [cycle] : { total: 51, offset: r.offset, bucketSeconds: 900, points: [{ sampledAt: r.offset ? cycle.endAt : cycle.startAt, remainingPercent: r.offset ? 60 : 100, breakBefore: !r.offset }] });
    render(<CycleList now={Date.parse(cycle.endAt)} windowId="weekly" request={{ kind: 'preset', preset: 'all' }} />);
    await screen.findByText('Empty quota was not observed');
    fireEvent.click(screen.getByRole('button', { name: /100% → 60%/ }));
    await screen.findByRole('table', { name: 'Sample details' });
    fireEvent.click(screen.getByRole('button', { name: 'Next' }));
    await waitFor(() => expect(analytics).toHaveBeenCalledWith(expect.objectContaining({ kind: 'samples', cycleId: 'cycle-one', offset: 50 })));
  });
});
