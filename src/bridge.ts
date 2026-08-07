import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import type {
  AppUpdateInfo,
  AppUpdateProgress,
  DashboardSnapshot,
  NotificationEnableResult,
  Settings,
  SettingsUiFaultCode,
  UsageHistoryRange,
  UsageHistoryResponse,
} from './types';

/** 所有敏感 I/O 都经由这层调用 Rust command；前端不直接读取文件或发起认证请求。 */
export const usageBridge = {
  getDashboard: () => invoke<DashboardSnapshot>('get_dashboard'),
  refreshDashboard: () => invoke<DashboardSnapshot>('refresh_dashboard'),
  getSettings: () => invoke<Settings>('get_settings'),
  saveSettings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
  setNotificationsEnabled: (enabled: boolean) =>
    invoke<NotificationEnableResult>('set_notifications_enabled', { enabled }),
  sendTestNotification: () => invoke<void>('send_test_notification'),
  getUsageHistory: (range: UsageHistoryRange) =>
    invoke<UsageHistoryResponse>('get_usage_history', { range }),
  setHistoryEnabled: (enabled: boolean) => invoke<Settings>('set_history_enabled', { enabled }),
  clearUsageHistory: () => invoke<void>('clear_usage_history'),
  reportSettingsUiFault: (code: SettingsUiFaultCode) =>
    invoke<void>('report_settings_ui_fault', { faultCode: code }),
  getDiagnostics: () => invoke<string>('get_diagnostics'),
  getAutostart: () => invoke<boolean>('get_autostart'),
  setAutostart: (enabled: boolean) => invoke<boolean>('set_autostart', { enabled }),
  getAppUpdateInfo: () => invoke<AppUpdateInfo>('get_app_update_info'),
  checkAppUpdate: () => invoke<AppUpdateInfo>('check_app_update'),
  installAppUpdate: () => invoke<void>('install_app_update'),
  /** 由主卡请求 Rust 显示并聚焦独立设置窗口，避免前端跨窗口直接操作。 */
  openSettingsWindow: (section?: 'about') => invoke<void>('open_settings_window', { section }),
  /** 仅在用户真正开始拖动主卡边缘前标记手动尺寸，避免把 DPI/启动 Resize 误判为用户操作。 */
  markMainSizeManual: () => invoke<void>('mark_main_size_manual'),
  listenForDashboard: (handler: (snapshot: DashboardSnapshot) => void) =>
    listen<DashboardSnapshot>('dashboard-updated', (event) => handler(event.payload)),
  /** 设置变更只广播非敏感的展示选项，用于让两个窗口即时保持一致。 */
  listenForSettings: (handler: (settings: Settings) => void) =>
    listen<Settings>('settings-updated', (event) => handler(event.payload)),
  /** 自动检查只广播已脱敏版本摘要；安装进度仅发送给设置窗口。 */
  listenForAppUpdate: (handler: (info: AppUpdateInfo) => void) =>
    listen<AppUpdateInfo>('app-update-updated', (event) => handler(event.payload)),
  listenForAppUpdateProgress: (handler: (progress: AppUpdateProgress) => void) =>
    listen<AppUpdateProgress>('app-update-progress', (event) => handler(event.payload)),
  listenForSettingsNavigation: (handler: (section: 'about') => void) =>
    listen<'about'>('settings-navigate', (event) => handler(event.payload)),
  listenForUsageHistory: (handler: () => void) =>
    listen('usage-history-updated', () => handler()),
};
