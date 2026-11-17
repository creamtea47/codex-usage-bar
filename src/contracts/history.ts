import type { QuotaFallbackLabel, QuotaForecast } from './usage';
export type UsageHistoryPreset = '24h' | '7d' | '30d' | 'all';

/**
 * History IPC always uses an explicit tagged request. Custom dates are converted
 * from local calendar days to UTC instants before they cross the Tauri boundary.
 */
export type UsageHistoryRequest =
  | { kind: 'preset'; preset: UsageHistoryPreset }
  | { kind: 'custom'; startAt: string; endAtExclusive: string };

export interface UsageHistoryPoint {
  suggestedRemainingPercent?: number | null;
  sampledAt: string;
  remainingPercent: number;
  breakBefore: boolean;
}

export interface UsageHistorySeries {
  currentResetAt?: string | null;
  /** Anonymous locally salted stream key; used only as a React correlation key and never rendered. */
  windowId: string;
  windowSeconds: number;
  fallbackLabel: QuotaFallbackLabel;
  currentRemainingPercent: number | null;
  /** 用户本地自然日从 00:00 到当前时间的累计消耗；跨额度周期时可超过 100%。 */
  todayConsumedPercent: number;
  points: UsageHistoryPoint[];
  forecast: QuotaForecast;
}

export interface UsageHistoryResponse {
  accountId?: string | null;
  request: UsageHistoryRequest;
  appliedStartAt: string;
  appliedEndAtExclusive: string;
  availableStartAt: string | null;
  availableEndAt: string | null;
  truncatedByRetention: boolean;
  bucketSeconds: number | null;
  historyEnabled: boolean;
  storageStatus: 'ready' | 'empty' | 'recovered' | 'unavailable';
  sampleCount: number;
  earliestSampleAt: string | null;
  latestSampleAt: string | null;
  series: UsageHistorySeries[];
}

export interface RangeSummary {
  startAt: string | null;
  endAt: string | null;
  durationSeconds: number;
  firstRemainingPercent: number | null;
  lastRemainingPercent: number | null;
  consumedPercent: number;
  cycleCount: number;
  sampleCount: number;
  partial: boolean;
  bucketSeconds: number;
}
export interface CycleSummary {
  cycleId: string;
  startAt: string;
  endAt: string;
  resetAt: string | null;
  firstRemainingPercent: number;
  lastRemainingPercent: number;
  consumedPercent: number;
  durationSeconds: number;
  sampleCount: number;
  partial: boolean;
  outsideChartRange: boolean;
  current: boolean;
  firstExhaustedAt: string | null;
  bucketSeconds: number;
  forecast: QuotaForecast | null;
}
export interface CycleSamples {
  points: UsageHistoryPoint[];
  total: number;
  offset: number;
  bucketSeconds: number;
}
export type AnalyticsRequest =
  | { kind: 'range'; windowId: string; startAt: string; endAt: string }
  | { kind: 'cycles'; windowId: string; request: UsageHistoryRequest }
  | { kind: 'samples'; windowId: string; cycleId: string; offset: number };
