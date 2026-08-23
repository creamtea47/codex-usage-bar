import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useState, type ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import TrendsPage from './TrendsPage';
import type { UsageHistoryRequest, UsageHistoryResponse } from './types';

vi.mock('react-i18next', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react-i18next')>();
  const translations: Record<string, string> = {
    'common.close': 'Close',
    'common.unavailable': 'Unavailable',
    'quota.fallbackLabel.fiveHour': '5-hour limit',
    'quota.fallbackLabel.weekly': 'Weekly limit',
    'quota.fallbackLabel.window': '{{duration}} limit',
    'trends.title': 'Usage trends',
    'trends.subtitle': 'See locally stored usage history.',
    'trends.range.aria': 'History range',
    'trends.range.hours24': '24 hours',
    'trends.range.days7': '7 days',
    'trends.range.days30': '30 days',
    'trends.range.custom': 'Custom',
    'trends.range.customApplied': 'Custom range',
    'trends.custom.dialogTitle': 'Custom date range',
    'trends.custom.dialogDescription': 'Choose up to 30 local calendar days.',
    'trends.custom.startLabel': 'Start date',
    'trends.custom.endLabel': 'End date',
    'trends.custom.cancel': 'Cancel',
    'trends.custom.apply': 'Apply',
    'trends.custom.error.required': 'Choose both dates.',
    'trends.custom.error.invalid': 'Enter a valid date.',
    'trends.custom.error.future': 'Future dates are not available.',
    'trends.custom.error.reversed': 'End date must be on or after start date.',
    'trends.custom.error.tooLong': 'Choose no more than 30 calendar days.',
    'trends.custom.error.emptyToday': 'No time has elapsed today yet.',
    'trends.partialCoverage': 'Only part of this range is available in local history.',
    'trends.partialCoverageFrom': 'Only part is available. Data starts {{date}}.',
    'trends.localOnly.title': 'Stored only on this device',
    'trends.localOnly.description': 'History is never uploaded.',
    'trends.collection.label': 'Local history collection',
    'trends.collection.enabled': 'Collection is enabled.',
    'trends.collection.disabled': 'Collection is paused. Existing history remains available.',
    'trends.collection.enableSuccess': 'Collection enabled.',
    'trends.collection.disableSuccess': 'Collection paused.',
    'trends.collection.error': 'Unable to update history collection.',
    'trends.clear.button': 'Clear history',
    'trends.clear.dialogTitle': 'Clear local history?',
    'trends.clear.dialogBody': 'This permanently deletes all local usage samples.',
    'trends.clear.cancel': 'Cancel',
    'trends.clear.confirm': 'Delete',
    'trends.clear.success': 'History cleared.',
    'trends.clear.error': 'Unable to clear history.',
    'trends.events.error': 'Unable to follow history updates.',
    'trends.loading': 'Loading history',
    'trends.loadError': 'Unable to load usage history.',
    'trends.retry': 'Retry',
    'trends.empty.title': 'No history yet',
    'trends.empty.enabledDescription': 'Samples will appear after successful refreshes.',
    'trends.empty.disabledDescription': 'Enable collection to gather new samples.',
    'trends.storage.recovered': 'Damaged history was reset safely.',
    'trends.storage.unavailable': 'Local history storage is unavailable.',
    'trends.summary': '{{count}} samples · latest {{date}}',
    'trends.windowAria': '{{label}} trend',
    'trends.current.label': 'Current remaining',
    'trends.current.value': '{{percent}}% remaining',
    'trends.consumed.label': "Today's consumption",
    'trends.consumed.value': '{{percent}}% consumed',
    'trends.chart.aria': '{{label}} history chart',
    'trends.chart.remaining': 'Remaining',
    'trends.chart.reset': 'Reset',
    'trends.chart.noData': 'Not enough chart data.',
    'trends.forecast.label': 'Forecast',
    'trends.forecast.collecting': 'Collecting data',
    'trends.forecast.stable': 'Recent usage is stable',
    'trends.forecast.exhaustsAt': 'Estimated empty {{date}}',
    'trends.forecast.exhaustsBeforeReset': 'Expected to run out before reset',
    'trends.forecast.lastsUntilReset': 'Expected to last until reset',
    'trends.forecast.basis': '{{count}} samples · {{duration}} · {{percent}}% consumed',
  };

  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, options?: Record<string, unknown>) => {
        let value = translations[key] ?? key;
        for (const [name, replacement] of Object.entries(options ?? {})) {
          value = value.replaceAll(`{{${name}}}`, String(replacement));
        }
        return value;
      },
      i18n: { resolvedLanguage: 'en', language: 'en' },
    }),
  };
});

vi.mock('recharts', () => ({
  ResponsiveContainer: ({ children }: { children: ReactNode }) => (
    <div data-testid="responsive-chart">{children}</div>
  ),
  AreaChart: ({ children, data }: { children: ReactNode; data: unknown[] }) => (
    <svg data-testid="area-chart" data-point-count={data.length}>
      {children}
    </svg>
  ),
  Area: ({ connectNulls }: { connectNulls: boolean }) => (
    <path data-testid="trend-area" data-connect-nulls={String(connectNulls)} />
  ),
  CartesianGrid: () => <g />,
  ReferenceLine: ({ label }: { label?: { value?: ReactNode } }) => (
    <text data-testid="reset-marker">{label?.value}</text>
  ),
  Tooltip: () => null,
  XAxis: () => <g />,
  YAxis: () => <g />,
}));

const populatedHistory: UsageHistoryResponse = {
  request: { kind: 'preset', preset: '24h' },
  appliedStartAt: '2029-12-31T02:00:00Z',
  appliedEndAtExclusive: '2030-01-01T02:00:00Z',
  availableStartAt: '2030-01-01T00:00:00Z',
  availableEndAt: '2030-01-01T02:00:00Z',
  truncatedByRetention: false,
  bucketSeconds: 60,
  historyEnabled: true,
  storageStatus: 'ready',
  sampleCount: 3,
  earliestSampleAt: '2030-01-01T00:00:00Z',
  latestSampleAt: '2030-01-01T02:00:00Z',
  series: [
    {
      // Deliberately sensitive-looking: this stable storage key must never be rendered.
      windowId: 'private-account@example.com',
      windowSeconds: 604_800,
      fallbackLabel: 'weekly',
      currentRemainingPercent: 72,
      todayConsumedPercent: 28,
      points: [
        { sampledAt: '2030-01-01T00:00:00Z', remainingPercent: 90, breakBefore: false },
        { sampledAt: '2030-01-01T01:00:00Z', remainingPercent: 100, breakBefore: true },
        { sampledAt: '2030-01-01T02:00:00Z', remainingPercent: 72, breakBefore: false },
      ],
      forecast: {
        status: 'lastsUntilReset',
        exhaustsAt: null,
        sampleCount: 6,
        observedSpanSeconds: 3_600,
        consumedPercent: 8,
      },
    },
  ],
};

function emptyHistory(request: UsageHistoryRequest): UsageHistoryResponse {
  return {
    request,
    appliedStartAt: '2030-01-01T00:00:00Z',
    appliedEndAtExclusive: '2030-01-02T00:00:00Z',
    availableStartAt: null,
    availableEndAt: null,
    truncatedByRetention: false,
    bucketSeconds: 60,
    historyEnabled: true,
    storageStatus: 'empty',
    sampleCount: 0,
    earliestSampleAt: null,
    latestSampleAt: null,
    series: [],
  };
}

interface RenderOptions {
  getUsageHistory?: (request: UsageHistoryRequest) => Promise<UsageHistoryResponse>;
  onSetHistoryEnabled?: (enabled: boolean) => Promise<void>;
  onClearUsageHistory?: () => Promise<void>;
  listenForUsageHistory?: (handler: () => void) => Promise<() => void>;
  now?: () => Date;
}

function renderPage(options: RenderOptions = {}) {
  const getUsageHistory = vi.fn(
    options.getUsageHistory ?? (async () => populatedHistory),
  );
  const onSetHistoryEnabled = vi.fn(options.onSetHistoryEnabled ?? (async () => undefined));
  const onClearUsageHistory = vi.fn(options.onClearUsageHistory ?? (async () => undefined));
  function StatefulTrendsPage() {
    const [historyEnabled, setHistoryEnabled] = useState(true);

    return (
      <TrendsPage
        historyEnabled={historyEnabled}
        getUsageHistory={getUsageHistory}
        onSetHistoryEnabled={async (enabled) => {
          await onSetHistoryEnabled(enabled);
          setHistoryEnabled(enabled);
        }}
        onClearUsageHistory={onClearUsageHistory}
        listenForUsageHistory={options.listenForUsageHistory}
        now={options.now}
      />
    );
  }

  const view = render(<StatefulTrendsPage />);
  return { getUsageHistory, onSetHistoryEnabled, onClearUsageHistory, unmount: view.unmount };
}

describe('TrendsPage', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders sanitized per-window history, a reset break, metrics, and forecast', async () => {
    const { getUsageHistory } = renderPage();

    expect(await screen.findByText('Weekly limit')).toBeTruthy();
    expect(getUsageHistory).toHaveBeenCalledWith({ kind: 'preset', preset: '24h' });
    expect(screen.getByText('72% remaining')).toBeTruthy();
    expect(screen.getByText("Today's consumption")).toBeTruthy();
    expect(screen.getByText('28% consumed')).toBeTruthy();
    expect(screen.getByText((content) => content.endsWith('· 8% consumed'))).toBeTruthy();
    expect(screen.getByText('Expected to last until reset')).toBeTruthy();
    expect(screen.queryByText('private-account@example.com')).toBeNull();
    expect(screen.getByRole('article', { name: 'Weekly limit trend' })).toBeTruthy();
    expect(screen.getByRole('img', { name: 'Weekly limit history chart' })).toBeTruthy();
    expect(screen.getByTestId('area-chart').getAttribute('data-point-count')).toBe('4');
    expect(screen.getByTestId('trend-area').getAttribute('data-connect-nulls')).toBe('false');
    expect(screen.getByTestId('reset-marker').textContent).toBe('Reset');
  });

  it('switches between the fixed 24-hour and 7-day ranges', async () => {
    const getUsageHistory = vi.fn(async (request: UsageHistoryRequest) => emptyHistory(request));
    renderPage({ getUsageHistory });
    await screen.findByText('No history yet');

    fireEvent.click(screen.getByRole('button', { name: '7 days' }));
    await waitFor(() =>
      expect(getUsageHistory).toHaveBeenLastCalledWith({ kind: 'preset', preset: '7d' }),
    );
    expect(screen.getByRole('button', { name: '7 days' }).getAttribute('aria-pressed')).toBe('true');
  });

  it('requests the fixed 30-day preset and keeps 24 hours as a fresh-mount default', async () => {
    const getUsageHistory = vi.fn(async (request: UsageHistoryRequest) => emptyHistory(request));
    const firstView = renderPage({ getUsageHistory });
    await screen.findByText('No history yet');

    fireEvent.click(screen.getByRole('button', { name: '30 days' }));
    await waitFor(() =>
      expect(getUsageHistory).toHaveBeenLastCalledWith({ kind: 'preset', preset: '30d' }),
    );
    expect(screen.getByRole('button', { name: '30 days' }).getAttribute('aria-pressed')).toBe('true');

    firstView.unmount();
    renderPage({ getUsageHistory });
    await waitFor(() =>
      expect(getUsageHistory).toHaveBeenLastCalledWith({ kind: 'preset', preset: '24h' }),
    );
    expect(screen.getByRole('button', { name: '24 hours' }).getAttribute('aria-pressed')).toBe('true');
  });

  it('keeps custom dates as a draft until Apply and sends a half-open UTC request', async () => {
    const getUsageHistory = vi.fn(async (request: UsageHistoryRequest) => emptyHistory(request));
    renderPage({ getUsageHistory, now: () => new Date(2030, 0, 20, 15, 30) });
    await screen.findByText('No history yet');

    const trigger = screen.getByRole('button', { name: 'Custom' });
    fireEvent.click(trigger);
    const dialog = await screen.findByRole('dialog', { name: 'Custom date range' });
    expect(dialog.getAttribute('aria-describedby')).toBeTruthy();
    expect(getUsageHistory).toHaveBeenCalledTimes(1);

    const start = document.getElementById('custom-history-start-date') as HTMLInputElement;
    const end = document.getElementById('custom-history-end-date') as HTMLInputElement;
    fireEvent.change(start, { target: { value: '01/10/2030' } });
    fireEvent.change(end, { target: { value: '01/12/2030' } });
    expect(getUsageHistory).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole('button', { name: 'Apply' }));
    await waitFor(() => {
      const request = getUsageHistory.mock.calls.at(-1)?.[0];
      expect(request).toMatchObject({ kind: 'custom' });
      if (!request || request.kind !== 'custom') throw new Error('expected custom request');
      expect(request.startAt).toBe(new Date(2030, 0, 10).toISOString());
      expect(request.endAtExclusive).toBe(new Date(2030, 0, 13).toISOString());
    });
    await waitFor(() =>
      expect(screen.queryByRole('dialog', { name: 'Custom date range' })).toBeNull(),
    );
    expect(trigger.getAttribute('aria-pressed')).toBe('true');
    await waitFor(() => expect(document.activeElement).toBe(trigger));

    fireEvent.click(screen.getByRole('button', { name: 'Custom range' }));
    expect(await screen.findByRole('dialog', { name: 'Custom date range' })).toBeTruthy();
    expect(getUsageHistory).toHaveBeenCalledTimes(2);
  });

  it('caps a custom range ending today at the instant Apply is clicked', async () => {
    const applyInstant = new Date(2030, 0, 20, 15, 30);
    const getUsageHistory = vi.fn(async (request: UsageHistoryRequest) => emptyHistory(request));
    renderPage({ getUsageHistory, now: () => applyInstant });
    await screen.findByText('No history yet');

    fireEvent.click(screen.getByRole('button', { name: 'Custom' }));
    await screen.findByRole('dialog', { name: 'Custom date range' });
    fireEvent.click(screen.getByRole('button', { name: 'Apply' }));

    await waitFor(() => {
      const request = getUsageHistory.mock.calls.at(-1)?.[0];
      expect(request).toMatchObject({ kind: 'custom' });
      if (!request || request.kind !== 'custom') throw new Error('expected custom request');
      expect(request.endAtExclusive).toBe(applyInstant.toISOString());
    });
  });

  it('disables Apply for a today-only custom range at exact local midnight', async () => {
    const localMidnight = new Date(2030, 0, 20, 0, 0, 0, 0);
    const getUsageHistory = vi.fn(async (request: UsageHistoryRequest) => emptyHistory(request));
    renderPage({ getUsageHistory, now: () => localMidnight });
    await screen.findByText('No history yet');

    fireEvent.click(screen.getByRole('button', { name: 'Custom' }));
    const start = document.getElementById('custom-history-start-date') as HTMLInputElement;
    fireEvent.change(start, { target: { value: '01/20/2030' } });

    expect(await screen.findByText('No time has elapsed today yet.')).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(true);
    expect(getUsageHistory).toHaveBeenCalledTimes(1);
  });

  it('cancels custom draft changes without querying and validates future/reversed/overlong dates', async () => {
    const getUsageHistory = vi.fn(async (request: UsageHistoryRequest) => emptyHistory(request));
    renderPage({ getUsageHistory, now: () => new Date(2030, 0, 31, 12, 0) });
    await screen.findByText('No history yet');
    fireEvent.click(screen.getByRole('button', { name: 'Custom' }));

    const start = document.getElementById('custom-history-start-date') as HTMLInputElement;
    const end = document.getElementById('custom-history-end-date') as HTMLInputElement;
    fireEvent.change(start, { target: { value: '01/01/2030' } });
    fireEvent.change(end, { target: { value: '02/01/2030' } });
    expect(await screen.findByText('Future dates are not available.')).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(true);

    fireEvent.change(start, { target: { value: '01/20/2030' } });
    fireEvent.change(end, { target: { value: '01/10/2030' } });
    expect(await screen.findByText('End date must be on or after start date.')).toBeTruthy();
    fireEvent.change(start, { target: { value: '01/01/2030' } });
    fireEvent.change(end, { target: { value: '01/31/2030' } });
    expect(await screen.findByText('Choose no more than 30 calendar days.')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(getUsageHistory).toHaveBeenCalledTimes(1);
    await waitFor(() =>
      expect(screen.queryByRole('dialog', { name: 'Custom date range' })).toBeNull(),
    );
    expect(screen.getByRole('button', { name: '24 hours' }).getAttribute('aria-pressed')).toBe('true');
  });

  it('announces partial local coverage returned by the backend', async () => {
    const response = emptyHistory({ kind: 'preset', preset: '30d' });
    response.truncatedByRetention = true;
    const getUsageHistory = vi.fn(async () => response);
    renderPage({ getUsageHistory });
    await screen.findByText('No history yet');

    fireEvent.click(screen.getByRole('button', { name: '30 days' }));
    expect(await screen.findByText('Only part of this range is available in local history.')).toBeTruthy();
  });

  it('shows the actual available start for a leading coverage gap', async () => {
    const response = emptyHistory({ kind: 'preset', preset: '30d' });
    response.appliedStartAt = '2030-01-01T00:00:00Z';
    response.availableStartAt = new Date(2030, 0, 3, 20, 30).toISOString();
    const getUsageHistory = vi.fn(async () => response);

    renderPage({ getUsageHistory });

    expect(await screen.findByText(/Only part is available\. Data starts/)).toBeTruthy();
    expect(screen.getByText(/01\/03.*20:30/)).toBeTruthy();
  });

  it('does not report partial coverage only because the latest sample predates query now', async () => {
    const response = emptyHistory({ kind: 'preset', preset: '24h' });
    response.appliedStartAt = '2030-01-01T00:00:00Z';
    response.appliedEndAtExclusive = '2030-01-02T00:00:00Z';
    response.availableStartAt = '2030-01-01T00:00:00Z';
    response.availableEndAt = '2030-01-01T23:59:00Z';
    const getUsageHistory = vi.fn(async () => response);

    renderPage({ getUsageHistory });
    await screen.findByText('No history yet');

    expect(screen.queryByText(/Only part/)).toBeNull();
  });

  it('pauses collection without rereading or hiding existing history, and clears only after confirmation', async () => {
    const { getUsageHistory, onSetHistoryEnabled, onClearUsageHistory } = renderPage();
    await screen.findByText('Weekly limit');

    const collectionSwitch = screen.getByRole('switch', { name: 'Local history collection' });
    fireEvent.click(collectionSwitch);
    await waitFor(() => expect(onSetHistoryEnabled).toHaveBeenCalledWith(false));
    await waitFor(() => expect((collectionSwitch as HTMLInputElement).checked).toBe(false));
    expect(screen.getByText('Collection is paused. Existing history remains available.')).toBeTruthy();
    expect(screen.getByText('Weekly limit')).toBeTruthy();
    expect(screen.getByRole('img', { name: 'Weekly limit history chart' })).toBeTruthy();
    expect(screen.getByText(/3 samples/)).toBeTruthy();
    expect(screen.getByText('Expected to last until reset')).toBeTruthy();
    expect(getUsageHistory).toHaveBeenCalledTimes(1);
    expect(await screen.findByRole('button', { name: 'Close' })).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Clear history' }));
    expect(await screen.findByRole('dialog', { name: 'Clear local history?' })).toBeTruthy();
    expect(onClearUsageHistory).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await waitFor(() => expect(onClearUsageHistory).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('No history yet')).toBeTruthy();
    expect(getUsageHistory).toHaveBeenCalledTimes(1);
  });

  it('re-enables while the previous success message is visible without hiding stored history', async () => {
    const { getUsageHistory, onSetHistoryEnabled } = renderPage();
    await screen.findByText('Weekly limit');
    const collectionSwitch = screen.getByRole('switch', { name: 'Local history collection' });

    fireEvent.click(collectionSwitch);
    await waitFor(() => expect((collectionSwitch as HTMLInputElement).checked).toBe(false));
    expect(await screen.findByText('Collection paused.')).toBeTruthy();
    expect(screen.getByText('Weekly limit')).toBeTruthy();
    expect(screen.getByRole('img', { name: 'Weekly limit history chart' })).toBeTruthy();

    fireEvent.click(collectionSwitch);
    await waitFor(() => expect((collectionSwitch as HTMLInputElement).checked).toBe(true));
    expect(await screen.findByText('Collection enabled.')).toBeTruthy();
    expect(onSetHistoryEnabled.mock.calls).toEqual([[false], [true]]);
    expect(screen.getByText('Weekly limit')).toBeTruthy();
    expect(screen.getByRole('img', { name: 'Weekly limit history chart' })).toBeTruthy();
    expect(screen.getByText(/3 samples/)).toBeTruthy();
    expect(getUsageHistory).toHaveBeenCalledTimes(1);
  });

  it('ignores clickaway from the previous success message while re-enabling is delayed', async () => {
    let finishEnable: (() => void) | undefined;
    const onSetHistoryEnabled = vi
      .fn<(enabled: boolean) => Promise<void>>()
      .mockResolvedValueOnce(undefined)
      .mockImplementationOnce(
        () =>
          new Promise<void>((resolve) => {
            finishEnable = resolve;
          }),
      );
    const { getUsageHistory } = renderPage({ onSetHistoryEnabled });
    await screen.findByText('Weekly limit');
    const collectionSwitch = screen.getByRole('switch', { name: 'Local history collection' });

    fireEvent.click(collectionSwitch);
    expect(await screen.findByText('Collection paused.')).toBeTruthy();
    fireEvent.click(collectionSwitch);
    await waitFor(() => expect(onSetHistoryEnabled).toHaveBeenCalledTimes(2));
    await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
    fireEvent.mouseDown(document.body);
    fireEvent.click(document.body);

    expect(screen.getByText('Collection paused.')).toBeTruthy();
    await act(async () => finishEnable?.());
    expect(await screen.findByText('Collection enabled.')).toBeTruthy();
    expect((collectionSwitch as HTMLInputElement).checked).toBe(true);
    expect(screen.getByText('Weekly limit')).toBeTruthy();
    expect(getUsageHistory).toHaveBeenCalledTimes(1);
  });

  it('keeps toggle mutations single-flight under rapid repeated events', async () => {
    let finishDisable: (() => void) | undefined;
    let finishEnable: (() => void) | undefined;
    const onSetHistoryEnabled = vi
      .fn<(enabled: boolean) => Promise<void>>()
      .mockImplementationOnce(
        () =>
          new Promise<void>((resolve) => {
            finishDisable = resolve;
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise<void>((resolve) => {
            finishEnable = resolve;
          }),
      );
    const { getUsageHistory } = renderPage({ onSetHistoryEnabled });
    await screen.findByText('Weekly limit');
    const collectionSwitch = screen.getByRole('switch', { name: 'Local history collection' });

    act(() => {
      collectionSwitch.click();
      collectionSwitch.click();
    });
    expect(onSetHistoryEnabled.mock.calls).toEqual([[false]]);
    expect((collectionSwitch as HTMLInputElement).disabled).toBe(true);
    await act(async () => finishDisable?.());
    await waitFor(() => expect((collectionSwitch as HTMLInputElement).checked).toBe(false));

    act(() => {
      collectionSwitch.click();
      collectionSwitch.click();
    });
    expect(onSetHistoryEnabled.mock.calls).toEqual([[false], [true]]);
    expect((collectionSwitch as HTMLInputElement).disabled).toBe(true);
    await act(async () => finishEnable?.());
    await waitFor(() => expect((collectionSwitch as HTMLInputElement).checked).toBe(true));
    expect(screen.getByText('Weekly limit')).toBeTruthy();
    expect(screen.getByRole('img', { name: 'Weekly limit history chart' })).toBeTruthy();
    expect(getUsageHistory).toHaveBeenCalledTimes(1);
  });

  it('keeps collection state and existing charts when persisting the toggle fails', async () => {
    const onSetHistoryEnabled = vi.fn(async () => {
      throw new Error('private settings failure');
    });
    const { getUsageHistory } = renderPage({ onSetHistoryEnabled });
    await screen.findByText('Weekly limit');

    const collectionSwitch = screen.getByRole('switch', { name: 'Local history collection' });
    fireEvent.click(collectionSwitch);

    expect(await screen.findByText('Unable to update history collection.')).toBeTruthy();
    expect((collectionSwitch as HTMLInputElement).checked).toBe(true);
    expect(screen.getByText('Weekly limit')).toBeTruthy();
    expect(screen.queryByText('private settings failure')).toBeNull();
    expect(getUsageHistory).toHaveBeenCalledTimes(1);
  });

  it('shows a safe load error and lets the user retry', async () => {
    const getUsageHistory = vi
      .fn<(request: UsageHistoryRequest) => Promise<UsageHistoryResponse>>()
      .mockRejectedValueOnce(new Error('secret raw error'))
      .mockResolvedValueOnce(emptyHistory({ kind: 'preset', preset: '24h' }));
    renderPage({ getUsageHistory });

    expect(await screen.findByText('Unable to load usage history.')).toBeTruthy();
    expect(screen.queryByText('secret raw error')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(getUsageHistory).toHaveBeenCalledTimes(2));
    expect(await screen.findByText('No history yet')).toBeTruthy();
  });

  it('rereads after the clear-history event while avoiding a duplicate manual reload', async () => {
    let historyHandler: (() => void) | undefined;
    const listenForUsageHistory = vi.fn(async (handler: () => void) => {
      historyHandler = handler;
      return () => undefined;
    });
    const getUsageHistory = vi
      .fn<(request: UsageHistoryRequest) => Promise<UsageHistoryResponse>>()
      .mockResolvedValueOnce(populatedHistory)
      .mockResolvedValueOnce(emptyHistory({ kind: 'preset', preset: '24h' }));
    const onClearUsageHistory = vi.fn(async () => {
      historyHandler?.();
    });
    renderPage({ getUsageHistory, listenForUsageHistory, onClearUsageHistory });
    await screen.findByText('Weekly limit');

    fireEvent.click(screen.getByRole('button', { name: 'Clear history' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Delete' }));

    await waitFor(() => expect(getUsageHistory).toHaveBeenCalledTimes(2));
    expect(await screen.findByText('No history yet')).toBeTruthy();
  });

  it('does not let an in-flight read restore data after history is cleared without an event listener', async () => {
    let finishInitialRead: ((history: UsageHistoryResponse) => void) | undefined;
    const responseWithPartialCoverage = {
      ...populatedHistory,
      appliedStartAt: '2029-12-01T00:00:00Z',
      availableStartAt: '2030-01-01T00:00:00Z',
    };
    const getUsageHistory = vi
      .fn<() => Promise<UsageHistoryResponse>>()
      .mockResolvedValueOnce(responseWithPartialCoverage)
      .mockImplementationOnce(
        () =>
          new Promise<UsageHistoryResponse>((resolve) => {
            finishInitialRead = resolve;
          }),
      );
    const { onClearUsageHistory } = renderPage({ getUsageHistory });
    expect(await screen.findByText(/Only part is available/)).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Clear history' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Delete' }));
    await waitFor(() => expect(onClearUsageHistory).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('No history yet')).toBeTruthy();
    expect(screen.queryByText(/Only part/)).toBeNull();

    await act(async () => finishInitialRead?.(responseWithPartialCoverage));
    expect(screen.queryByText('Weekly limit')).toBeNull();
    expect(screen.getByText('No history yet')).toBeTruthy();
  });

  it('subscribes before the initial read and ignores an older read after an update event', async () => {
    let historyHandler: (() => void) | undefined;
    let finishInitialRead: ((history: UsageHistoryResponse) => void) | undefined;
    const listenForUsageHistory = vi.fn(async (handler: () => void) => {
      historyHandler = handler;
      return () => undefined;
    });
    const getUsageHistory = vi
      .fn<(request: UsageHistoryRequest) => Promise<UsageHistoryResponse>>()
      .mockImplementationOnce(
        () =>
          new Promise<UsageHistoryResponse>((resolve) => {
            finishInitialRead = resolve;
          }),
      )
      .mockResolvedValueOnce(populatedHistory);

    renderPage({ getUsageHistory, listenForUsageHistory });
    await waitFor(() => expect(getUsageHistory).toHaveBeenCalledTimes(1));
    expect(listenForUsageHistory.mock.invocationCallOrder[0]).toBeLessThan(
      getUsageHistory.mock.invocationCallOrder[0],
    );

    await act(async () => historyHandler?.());
    expect(await screen.findByText('Weekly limit')).toBeTruthy();
    expect(getUsageHistory).toHaveBeenCalledTimes(2);

    await act(async () =>
      finishInitialRead?.(emptyHistory({ kind: 'preset', preset: '24h' })),
    );
    expect(screen.getByText('Weekly limit')).toBeTruthy();
    expect(screen.queryByText('No history yet')).toBeNull();
  });

  it('ignores a stale request failure after a newer history event succeeds', async () => {
    let historyHandler: (() => void) | undefined;
    let failInitialRead: ((error: Error) => void) | undefined;
    const listenForUsageHistory = vi.fn(async (handler: () => void) => {
      historyHandler = handler;
      return () => undefined;
    });
    const getUsageHistory = vi
      .fn<(request: UsageHistoryRequest) => Promise<UsageHistoryResponse>>()
      .mockImplementationOnce(
        () =>
          new Promise<UsageHistoryResponse>((_resolve, reject) => {
            failInitialRead = reject;
          }),
      )
      .mockResolvedValueOnce(populatedHistory);

    renderPage({ getUsageHistory, listenForUsageHistory });
    await waitFor(() => expect(getUsageHistory).toHaveBeenCalledTimes(1));
    await act(async () => historyHandler?.());
    expect(await screen.findByText('Weekly limit')).toBeTruthy();

    await act(async () => failInitialRead?.(new Error('stale private failure')));
    expect(screen.queryByText('Unable to load usage history.')).toBeNull();
    expect(screen.getByText('Weekly limit')).toBeTruthy();
  });
});
