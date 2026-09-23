import type { DashboardSnapshot, QuotaForecast, UsageHistoryPoint, UsageHistoryRequest } from './types';
import type { DashboardErrorCode } from './types';

export interface ResetCreditDetails {
  accountId: string;
  status: 'ready' | 'stale' | 'unavailable';
  fetchedAt: string | null;
  checkedAt: string;
  credits: { expiresAt: string | null; status: 'available'; supportedByPlan: boolean | null }[];
  errorCode: DashboardErrorCode | null;
}

export interface AccountConfig { id: string; label: string; enabled: boolean; autoContinue: boolean; model: string | null }
export interface Account extends AccountConfig { boundToCodex: boolean; authStatus: string; expiresAt: string | null; canRefresh: boolean; dashboard: DashboardSnapshot; details?: AccountDetails }
export interface AccountsResponse { selectedId: string | null; accounts: Account[]; codexLogin?: CodexLoginSource }
/** 完整资料仅由设置窗口 IPC 提供；主悬浮窗继续使用既有脱敏字段。 */
export interface AccountDetails {
  email: string | null;
  name: string | null;
  loginProvider: string | null;
  userId: string | null;
  accountId: string | null;
  planType: string | null;
  identitySource: 'usage' | 'credentials';
  planSource: 'usage' | 'credentials';
  credentialObservedAt: string | null;
  usageObservedAt: string | null;
  subscriptionStartedAt: string | null;
  subscriptionEndsAt: string | null;
  subscriptionCheckedAt: string | null;
  managedAuthPath: string;
  resetCredits: { available: number; applicable: number | null } | null;
  extraCredits: { hasCredits: boolean | null; unlimited: boolean | null; balance: string | null } | null;
  models: { id: string; available: boolean; availableAt: string | null }[];
}
export interface CodexLoginSource {
  path: string | null;
  storage: string | null;
  status: 'matched' | 'unmanaged' | 'missing' | 'invalid' | 'unsupportedStore' | 'unavailable';
  matchedAccountId: string | null;
  email: string | null;
  checkedAt: string;
  bindingMatches: boolean | null;
}
export interface ModelOption { id: string; label: string }
export interface ResponseDetails { startedAt: string | null; responseModel: string | null; httpStatus: number | null; durationMs: number; text: string; truncated: boolean; errorMessage: string | null; deliveryUncertain: boolean }
export interface RangeSummary { startAt: string | null; endAt: string | null; durationSeconds: number; firstRemainingPercent: number | null; lastRemainingPercent: number | null; consumedPercent: number; cycleCount: number; sampleCount: number; partial: boolean; bucketSeconds: number }
export interface CycleSummary { cycleId: string; startAt: string; endAt: string; resetAt: string | null; firstRemainingPercent: number; lastRemainingPercent: number; consumedPercent: number; durationSeconds: number; sampleCount: number; partial: boolean; outsideChartRange: boolean; current: boolean; firstExhaustedAt: string | null; bucketSeconds: number; forecast: QuotaForecast | null }
export interface CycleSamples { points: UsageHistoryPoint[]; total: number; offset: number; bucketSeconds: number }
export type AnalyticsRequest = { kind: 'range'; windowId: string; startAt: string; endAt: string } | { kind: 'cycles'; windowId: string; request: UsageHistoryRequest } | { kind: 'samples'; windowId: string; cycleId: string; offset: number };
