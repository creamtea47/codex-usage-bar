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
  listenForDashboard: (handler: (snapshot: DashboardSnapshot) => void) =>
    listen<DashboardSnapshot>('dashboard-updated', (event) => handler(event.payload)),
};
