export type Theme = 'system' | 'light' | 'dark';
export type Language = 'system' | 'zh-CN' | 'en';
/** 设置窗口仅允许上报固定脱敏故障码，禁止携带异常正文或用户数据。 */
export type SettingsUiFaultCode = 'trends-render-failed';
export interface Settings {
  autoLightweight: boolean;
  lightweightMinutes: number;
  alwaysOnTop: boolean;
  lockPosition: boolean;
  /** 默认拦截主窗口关闭并隐藏到系统托盘。 */
  minimizeToTrayOnClose: boolean;
  refreshIntervalSeconds: number;
  theme: Theme;
  language: Language;
  notifications: NotificationSettings;
  historyEnabled: boolean;
  /** 自动检查只读取公开 HTTPS 更新清单；不会自动下载、安装或重启。 */
  autoCheckUpdates: boolean;
  /** 真实请求开关只能通过专属 IPC 修改，普通设置保存会保留当前值。 */
  quotaAutoContinueEnabled: boolean;
}

export interface NotificationSettings {
  enabled: boolean;
  lowQuotaEnabled: boolean;
  lowQuotaThresholdPercent: number;
  paceEnabled: boolean;
  paceDeficitThresholdPercent: number;
  resetEnabled: boolean;
  resetCreditEnabled: boolean;
  extraCreditEnabled: boolean;
  connectionEnabled: boolean;
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
