import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import type { DashboardSnapshot, Settings, UpdateInfo } from './types';

/** 所有敏感 I/O 都经由这层调用 Rust command；前端不直接读取文件或发起认证请求。 */
export const usageBridge = {
  getDashboard: () => invoke<DashboardSnapshot>('get_dashboard'),
  refreshDashboard: () => invoke<DashboardSnapshot>('refresh_dashboard'),
  getSettings: () => invoke<Settings>('get_settings'),
  saveSettings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
  getAutostart: () => invoke<boolean>('get_autostart'),
  setAutostart: (enabled: boolean) => invoke<boolean>('set_autostart', { enabled }),
  checkUpdate: () => invoke<UpdateInfo>('check_update'),
  /** 由主卡请求 Rust 显示并聚焦独立设置窗口，避免前端跨窗口直接操作。 */
  openSettingsWindow: () => invoke<void>('open_settings_window'),
  /** 仅在用户真正开始拖动主卡边缘前标记手动尺寸，避免把 DPI/启动 Resize 误判为用户操作。 */
  markMainSizeManual: () => invoke<void>('mark_main_size_manual'),
  listenForDashboard: (handler: (snapshot: DashboardSnapshot) => void) =>
    listen<DashboardSnapshot>('dashboard-updated', (event) => handler(event.payload)),
  /** 设置变更只广播非敏感的展示选项，用于让两个窗口即时保持一致。 */
  listenForSettings: (handler: (settings: Settings) => void) =>
    listen<Settings>('settings-updated', (event) => handler(event.payload)),
};
