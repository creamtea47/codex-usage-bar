import { invoke, listen } from '@/platform/ipc';
import type { SettingsUiFaultCode, Settings } from '@/contracts/settings';
export const settingsApi = {
  getSettings: () => invoke<Settings>('get_settings'),
  saveSettings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
  reportSettingsUiFault: (code: SettingsUiFaultCode) =>
    invoke<void>('report_settings_ui_fault', { faultCode: code }),
  getDiagnostics: () => invoke<string>('get_diagnostics'),
  getAutostart: () => invoke<boolean>('get_autostart'),
  setAutostart: (enabled: boolean) => invoke<boolean>('set_autostart', { enabled }),
  openSettingsWindow: (section?: 'about') => invoke<void>('open_settings_window', { section }),
  markMainSizeManual: () => invoke<void>('mark_main_size_manual'),
  listenForSettings: (handler: (settings: Settings) => void) =>
    listen<Settings>('settings-updated', (event) => handler(event.payload)),
  listenForSettingsNavigation: (handler: (section: 'about') => void) =>
    listen<'about'>('settings-navigate', (event) => handler(event.payload)),
};
