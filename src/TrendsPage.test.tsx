import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useState, type ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import TrendsPage from './TrendsPage';
import type { UsageHistoryResponse, UsageHistoryRange } from './types';

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
    'trends.localOnly.title': 'Stored only on this device',
    'trends.localOnly.description': 'History is never uploaded.',
    'trends.collection.label': 'Local history collection',
    'trends.collection.enabled': 'Collection is enabled.',
    'trends.collection.disabled': 'Collection is paused. Existing history remains available.',
    'trends.collection.enableSuccess': 'Collection enabled.',
    'trends.collection.disableSuccess': 'Collection paused.',
    'trends.collection.error': 'Unable to update collection.',
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
    'trends.consumed.label': 'Recent consumption',
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
  range: '24h',
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
      consumedPercent: 8,
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

function emptyHistory(range: UsageHistoryRange): UsageHistoryResponse {
  return {
    range,
    historyEnabled: true,
    storageStatus: 'empty',
    sampleCount: 0,
    earliestSampleAt: null,
    latestSampleAt: null,
    series: [],
  };
}

interface RenderOptions {
  getUsageHistory?: (range: UsageHistoryRange) => Promise<UsageHistoryResponse>;
  onSetHistoryEnabled?: (enabled: boolean) => Promise<void>;
  onClearUsageHistory?: () => Promise<void>;
  listenForUsageHistory?: (handler: () => void) => Promise<() => void>;
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
      />
    );
  }

  render(<StatefulTrendsPage />);
  return { getUsageHistory, onSetHistoryEnabled, onClearUsageHistory };
}

describe('TrendsPage', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders sanitized per-window history, a reset break, metrics, and forecast', async () => {
    const { getUsageHistory } = renderPage();

    expect(await screen.findByText('Weekly limit')).toBeTruthy();
    expect(getUsageHistory).toHaveBeenCalledWith('24h');
    expect(screen.getByText('72% remaining')).toBeTruthy();
    expect(screen.getByText('8% consumed')).toBeTruthy();
    expect(screen.getByText('Expected to last until reset')).toBeTruthy();
    expect(screen.queryByText('private-account@example.com')).toBeNull();
    expect(screen.getByRole('article', { name: 'Weekly limit trend' })).toBeTruthy();
    expect(screen.getByRole('img', { name: 'Weekly limit history chart' })).toBeTruthy();
    expect(screen.getByTestId('area-chart').getAttribute('data-point-count')).toBe('4');
    expect(screen.getByTestId('trend-area').getAttribute('data-connect-nulls')).toBe('false');
    expect(screen.getByTestId('reset-marker').textContent).toBe('Reset');
  });

  it('switches between the fixed 24-hour and 7-day ranges', async () => {
    const getUsageHistory = vi.fn(async (range: UsageHistoryRange) => emptyHistory(range));
    renderPage({ getUsageHistory });
    await screen.findByText('No history yet');

    fireEvent.click(screen.getByRole('button', { name: '7 days' }));
    await waitFor(() => expect(getUsageHistory).toHaveBeenLastCalledWith('7d'));
    expect(screen.getByRole('button', { name: '7 days' }).getAttribute('aria-pressed')).toBe('true');
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

  it('keeps collection state and existing charts when persisting the toggle fails', async () => {
    const onSetHistoryEnabled = vi.fn(async () => {
      throw new Error('private settings failure');
    });
    const { getUsageHistory } = renderPage({ onSetHistoryEnabled });
    await screen.findByText('Weekly limit');

    const collectionSwitch = screen.getByRole('switch', { name: 'Local history collection' });
    fireEvent.click(collectionSwitch);

    expect(await screen.findByText('Unable to update collection.')).toBeTruthy();
    expect((collectionSwitch as HTMLInputElement).checked).toBe(true);
    expect(screen.getByText('Weekly limit')).toBeTruthy();
    expect(screen.queryByText('private settings failure')).toBeNull();
    expect(getUsageHistory).toHaveBeenCalledTimes(1);
  });

  it('shows a safe load error and lets the user retry', async () => {
    const getUsageHistory = vi
      .fn<(range: UsageHistoryRange) => Promise<UsageHistoryResponse>>()
      .mockRejectedValueOnce(new Error('secret raw error'))
      .mockResolvedValueOnce(emptyHistory('24h'));
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
      .fn<(range: UsageHistoryRange) => Promise<UsageHistoryResponse>>()
      .mockResolvedValueOnce(populatedHistory)
      .mockResolvedValueOnce(emptyHistory('24h'));
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
    const getUsageHistory = vi.fn(
      () =>
        new Promise<UsageHistoryResponse>((resolve) => {
          finishInitialRead = resolve;
        }),
    );
    const { onClearUsageHistory } = renderPage({ getUsageHistory });
    await waitFor(() => expect(getUsageHistory).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole('button', { name: 'Clear history' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Delete' }));
    await waitFor(() => expect(onClearUsageHistory).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('No history yet')).toBeTruthy();

    await act(async () => finishInitialRead?.(populatedHistory));
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
      .fn<(range: UsageHistoryRange) => Promise<UsageHistoryResponse>>()
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

    await act(async () => finishInitialRead?.(emptyHistory('24h')));
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
      .fn<(range: UsageHistoryRange) => Promise<UsageHistoryResponse>>()
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
