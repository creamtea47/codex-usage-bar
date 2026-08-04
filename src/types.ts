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
  /**
   * 仅由 Rust 从成功响应中提取并脱敏后的展示邮箱，例如 j***@example.com。
   * 前端绝不会接触认证文件、Token、请求头或原始账号资料。
   */
  accountEmailMasked: string | null;
  planLabel: string | null;
  refreshedAt: string | null;
  nextRefreshAt: string | null;
  message: string | null;
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
  /** 自动检查只读取公开 HTTPS 更新清单；不会自动下载、安装或重启。 */
  autoCheckUpdates: boolean;
}

export const defaultSettings: Settings = {
  alwaysOnTop: false,
  lockPosition: false,
  refreshIntervalSeconds: 60,
  theme: 'system',
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
