export type DashboardStatus = 'loading' | 'ready' | 'stale' | 'error';
export type Theme = 'system' | 'light' | 'dark';
export type Language = 'system' | 'zh-CN' | 'en';
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
/** 设置窗口仅允许上报固定脱敏故障码，禁止携带异常正文或用户数据。 */
export type SettingsUiFaultCode = 'trends-render-failed';

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

export interface DashboardSnapshot {
  status: DashboardStatus;
  /**
   * 仅由 Rust 从成功响应中提取并脱敏后的展示邮箱，例如 j***@example.com。
   * 前端绝不会接触认证文件、Token、请求头或原始账号资料。
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
export interface AppUpdateInfo {
  currentVersion: string;
  latestVersion: string;
  updateAvailable: boolean;
  checkedAt: string | null;
}

export type AppUpdateStage = 'downloading' | 'verifying' | 'installing';

/** 下载进度只用于呈现，不携带下载地址、签名或任何认证资料。 */
export interface AppUpdateProgress {
  stage: AppUpdateStage;
  downloadedBytes: number;
  totalBytes: number | null;
}

export interface Settings {
  alwaysOnTop: boolean;
  lockPosition: boolean;
  refreshIntervalSeconds: number;
  theme: Theme;
  language: Language;
  notifications: NotificationSettings;
  historyEnabled: boolean;
  /** 自动检查只读取公开 HTTPS 更新清单；不会自动下载、安装或重启。 */
  autoCheckUpdates: boolean;
}

export interface NotificationSettings {
  enabled: boolean;
  lowQuotaEnabled: boolean;
  lowQuotaThresholdPercent: number;
  paceEnabled: boolean;
  paceDeficitThresholdPercent: number;
  resetEnabled: boolean;
  quietHoursEnabled: boolean;
  quietHoursStart: string;
  quietHoursEnd: string;
}

export type NotificationPermission = 'granted' | 'denied' | 'prompt' | 'unavailable';

export interface NotificationEnableResult {
  enabled: boolean;
  permission: NotificationPermission;
  settings: Settings;
}

export type UsageHistoryRange = '24h' | '7d';

export interface UsageHistoryPoint {
  sampledAt: string;
  remainingPercent: number;
  breakBefore: boolean;
}

export interface UsageHistorySeries {
  /** Anonymous locally salted stream key; used only as a React correlation key and never rendered. */
  windowId: string;
  windowSeconds: number;
  fallbackLabel: QuotaFallbackLabel;
  currentRemainingPercent: number | null;
  consumedPercent: number;
  points: UsageHistoryPoint[];
  forecast: QuotaForecast;
}

export interface UsageHistoryResponse {
  range: UsageHistoryRange;
  historyEnabled: boolean;
  storageStatus: 'ready' | 'empty' | 'recovered' | 'unavailable';
  sampleCount: number;
  earliestSampleAt: string | null;
  latestSampleAt: string | null;
  series: UsageHistorySeries[];
}

export const defaultSettings: Settings = {
  alwaysOnTop: false,
  lockPosition: false,
  refreshIntervalSeconds: 60,
  theme: 'system',
  language: 'system',
  notifications: {
    enabled: false,
    lowQuotaEnabled: true,
    lowQuotaThresholdPercent: 20,
    paceEnabled: true,
    paceDeficitThresholdPercent: 10,
    resetEnabled: true,
    quietHoursEnabled: false,
    quietHoursStart: '22:00',
    quietHoursEnd: '08:00',
  },
  historyEnabled: true,
  autoCheckUpdates: true,
};

export const loadingSnapshot: DashboardSnapshot = {
  status: 'loading',
  accountEmailMasked: null,
  planLabel: null,
  refreshedAt: null,
  nextRefreshAt: null,
  message: null,
  quotaWindows: [],
};
