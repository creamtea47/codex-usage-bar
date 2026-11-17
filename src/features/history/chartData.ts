import { formatDateTime, formatDuration } from '@/shared/format';
import type { ForecastStatus, QuotaFallbackLabel } from '@/contracts/usage';
import type { UsageHistoryPoint, UsageHistoryRequest } from '@/contracts/history';

export type TrendsTranslationKey =
  | 'trends.title'
  | 'trends.subtitle'
  | 'trends.range.aria'
  | 'trends.range.hours24'
  | 'trends.range.days7'
  | 'trends.range.days30'
  | 'trends.range.all'
  | 'trends.range.custom'
  | 'trends.range.customApplied'
  | 'trends.custom.dialogTitle'
  | 'trends.custom.dialogDescription'
  | 'trends.custom.startLabel'
  | 'trends.custom.endLabel'
  | 'trends.custom.cancel'
  | 'trends.custom.apply'
  | 'trends.custom.error.required'
  | 'trends.custom.error.invalid'
  | 'trends.custom.error.future'
  | 'trends.custom.error.reversed'
  | 'trends.custom.error.emptyToday'
  | 'trends.partialCoverage'
  | 'trends.partialCoverageFrom'
  | 'trends.localOnly.title'
  | 'trends.localOnly.description'
  | 'trends.collection.label'
  | 'trends.collection.enabled'
  | 'trends.collection.disabled'
  | 'trends.collection.enableSuccess'
  | 'trends.collection.disableSuccess'
  | 'trends.collection.error'
  | 'trends.clear.button'
  | 'trends.clear.dialogTitle'
  | 'trends.clear.dialogBody'
  | 'trends.clear.cancel'
  | 'trends.clear.confirm'
  | 'trends.clear.success'
  | 'trends.clear.error'
  | 'trends.events.error'
  | 'trends.loading'
  | 'trends.loadError'
  | 'trends.retry'
  | 'trends.empty.title'
  | 'trends.empty.enabledDescription'
  | 'trends.empty.disabledDescription'
  | 'trends.storage.recovered'
  | 'trends.storage.unavailable'
  | 'trends.summary'
  | 'trends.windowAria'
  | 'trends.current.label'
  | 'trends.current.value'
  | 'trends.consumed.label'
  | 'trends.consumed.value'
  | 'trends.chart.aria'
  | 'trends.chart.remaining'
  | 'trends.chart.reset'
  | 'trends.chart.noData'
  | 'trends.forecast.label'
  | 'trends.forecast.collecting'
  | 'trends.forecast.stable'
  | 'trends.forecast.exhaustsAt'
  | 'trends.forecast.exhaustsBeforeReset'
  | 'trends.forecast.lastsUntilReset'
  | 'trends.forecast.basis';

interface ChartPoint {
  timestamp: number;
  remainingPercent: number | null;
  suggestedRemainingPercent?: number | null;
}

interface PreparedChartData {
  points: ChartPoint[];
  resetMarkers: number[];
  domain: [number, number];
}

export function clampPercent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(100, Math.max(0, value));
}

export function safeConsumptionPercent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, value);
}

export function languageForFormatting(language?: string): 'zh-CN' | 'en' {
  return language && /^zh(?:-|$)/i.test(language) ? 'zh-CN' : 'en';
}

export function validHistoryPoints(points: UsageHistoryPoint[]) {
  return points
    .map((point) => ({
      timestamp: Date.parse(point.sampledAt),
      remainingPercent: clampPercent(point.remainingPercent),
      suggestedRemainingPercent: point.suggestedRemainingPercent ?? null,
      breakBefore: point.breakBefore,
    }))
    .filter((point) => Number.isFinite(point.timestamp))
    .sort((left, right) => left.timestamp - right.timestamp);
}

/** Inserts a null between reset cycles so Recharts cannot connect both sides. */
export function prepareChartData(points: UsageHistoryPoint[]): PreparedChartData {
  const valid = validHistoryPoints(points);
  const chartPoints: ChartPoint[] = [];
  const resetMarkers: number[] = [];

  valid.forEach((point, index) => {
    const previous = valid[index - 1];
    if (index > 0 && point.breakBefore && previous) {
      const midpoint =
        point.timestamp > previous.timestamp
          ? previous.timestamp + (point.timestamp - previous.timestamp) / 2
          : point.timestamp - 1;
      chartPoints.push({ timestamp: midpoint, remainingPercent: null });
      resetMarkers.push(point.timestamp);
    }
    chartPoints.push(point);
  });

  const timestamps = valid.map((point) => point.timestamp);
  const minimum = timestamps[0] ?? Date.now() - 30 * 60 * 1_000;
  const maximum = timestamps[timestamps.length - 1] ?? Date.now() + 30 * 60 * 1_000;
  const padding = minimum === maximum ? 30 * 60 * 1_000 : 0;
  return {
    points: chartPoints,
    resetMarkers,
    domain: [minimum - padding, maximum + padding],
  };
}

export function formatAxisTime(
  timestamp: number,
  request: UsageHistoryRequest,
  language: 'zh-CN' | 'en',
): string {
  if (!Number.isFinite(timestamp)) return '';
  return new Intl.DateTimeFormat(language, {
    ...(request.kind === 'preset' && request.preset === '24h'
      ? { hour: '2-digit', minute: '2-digit' }
      : { month: 'numeric', day: 'numeric' }),
  }).format(new Date(timestamp));
}

export function fallbackWindowLabel(
  fallbackLabel: QuotaFallbackLabel,
  windowSeconds: number,
  language: 'zh-CN' | 'en',
  translate: (key: string, options?: Record<string, unknown>) => string,
): string {
  if (fallbackLabel === 'fiveHour') return translate('quota.fallbackLabel.fiveHour');
  if (fallbackLabel === 'weekly') return translate('quota.fallbackLabel.weekly');
  return translate('quota.fallbackLabel.window', {
    duration: formatDuration(windowSeconds, language),
  });
}

export function forecastMessage(
  status: ForecastStatus,
  exhaustsAt: string | null,
  language: 'zh-CN' | 'en',
  translate: (key: TrendsTranslationKey, options?: Record<string, unknown>) => string,
): string {
  if (status === 'collecting') return translate('trends.forecast.collecting');
  if (status === 'stable') return translate('trends.forecast.stable');
  if (status === 'lastsUntilReset') return translate('trends.forecast.lastsUntilReset');
  if (exhaustsAt) {
    return translate('trends.forecast.exhaustsAt', {
      date: formatDateTime(exhaustsAt, language),
    });
  }
  return translate('trends.forecast.exhaustsBeforeReset');
}
