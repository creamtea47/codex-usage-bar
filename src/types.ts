export type DashboardStatus = 'loading' | 'ready' | 'stale' | 'error';
export type Theme = 'system' | 'light' | 'dark';

/** Rust 仅返回已脱敏的展示数据，认证信息永远不会出现在此类型中。 */
export interface QuotaWindow {
  id: string;
  label: string;
  remainingPercent: number;
  usedPercent: number;
  windowSeconds: number;
  resetAt: string | null;
  resetAfterSeconds: number;
  startAt: string | null;
  showPaceMarker: boolean;
}

export interface DashboardSnapshot {
  status: DashboardStatus;
  planLabel: string | null;
  refreshedAt: string | null;
  nextRefreshAt: string | null;
  message: string | null;
  quotaWindows: QuotaWindow[];
}

/** 仅来自公开 GitHub Release 的安全更新摘要，不带请求或设备信息。 */
export interface UpdateInfo {
  currentVersion: string;
  latestVersion: string;
  updateAvailable: boolean;
  releaseUrl: string;
  downloadUrl: string | null;
  publishedAt: string | null;
}

export interface Settings {
  alwaysOnTop: boolean;
  lockPosition: boolean;
  refreshIntervalSeconds: number;
  theme: Theme;
}

export const defaultSettings: Settings = {
  alwaysOnTop: false,
  lockPosition: false,
  refreshIntervalSeconds: 60,
  theme: 'system',
};

export const loadingSnapshot: DashboardSnapshot = {
  status: 'loading',
  planLabel: null,
  refreshedAt: null,
  nextRefreshAt: null,
  message: null,
  quotaWindows: [],
};
