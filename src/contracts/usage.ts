export type DashboardStatus = 'loading' | 'ready' | 'stale' | 'error';
export type DashboardErrorCode =
  | 'authMissing'
  | 'authInvalid'
  | 'network'
  | 'rateLimited'
  | 'serviceUnavailable'
  | 'invalidResponse'
  | 'localBridge';
export type QuotaFallbackLabel = 'fiveHour' | 'weekly' | 'window';
export type ForecastStatus = 'collecting' | 'stable' | 'exhaustsBeforeReset' | 'lastsUntilReset';

export interface QuotaForecast {
  status: ForecastStatus;
  exhaustsAt: string | null;
  sampleCount: number;
  observedSpanSeconds: number;
  consumedPercent: number;
}

/** Rust 仅返回已脱敏的展示数据，认证信息永远不会出现在此类型中。 */
export interface QuotaWindow {
  id: string;
  /** 服务端明确提供的名称原样显示；fallbackLabel 由前端本地化。 */
  label: string | null;
  fallbackLabel: QuotaFallbackLabel;
  remainingPercent: number;
  usedPercent: number;
  windowSeconds: number;
  resetAt: string | null;
  resetAfterSeconds: number;
  startAt: string | null;
  showPaceMarker: boolean;
  forecast: QuotaForecast | null;
}

export interface ExtraCredits {
  balance: string | null;
  hasCredits: boolean | null;
  unlimited: boolean | null;
}

export interface DashboardSnapshot {
  extraCredits?: ExtraCredits | null;
  accountId?: string | null;
  status: DashboardStatus;
  /**
   * 仅由 Rust 从成功响应中提取并脱敏后的展示邮箱，例如 j***@example.com。
   * 本字段只供主卡脱敏展示；设置窗口可从 AccountDetails 读取白名单身份信息。
   */
  accountEmailMasked: string | null;
  planLabel: string | null;
  refreshedAt: string | null;
  nextRefreshAt: string | null;
  /** Rust 只返回稳定错误代码；所有面向用户的文案都在前端翻译。 */
  message: DashboardErrorCode | null;
  quotaWindows: QuotaWindow[];
}

/** 仅来自 HTTPS 更新清单的安全摘要；下载后才验证更新包签名，且不含 URL、签名、原始清单或设备信息。 */
export interface ResetCreditDetails {
  accountId: string;
  status: 'ready' | 'stale' | 'unavailable';
  fetchedAt: string | null;
  checkedAt: string;
  credits: { expiresAt: string | null; status: 'available'; supportedByPlan: boolean | null }[];
  errorCode: DashboardErrorCode | null;
}
