import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import SettingsWindow from './SettingsWindow';
import { applyLanguagePreference } from './i18n';
import {
  defaultSettings,
  type AppUpdateInfo,
  type NotificationEnableResult,
  type Settings,
  type UsageHistoryResponse,
} from './types';

const bridgeMocks = vi.hoisted(() => ({
  getDashboard: vi.fn(),
  refreshDashboard: vi.fn(),
  getSettings: vi.fn(),
  saveSettings: vi.fn(),
  setNotificationsEnabled: vi.fn(),
  sendTestNotification: vi.fn(),
  getUsageHistory: vi.fn(),
  setHistoryEnabled: vi.fn(),
  clearUsageHistory: vi.fn(),
  getDiagnostics: vi.fn(),
  getAutostart: vi.fn(),
  setAutostart: vi.fn(),
  getAppUpdateInfo: vi.fn(),
  checkAppUpdate: vi.fn(),
  installAppUpdate: vi.fn(),
  openSettingsWindow: vi.fn(),
  listenForDashboard: vi.fn(),
  listenForSettings: vi.fn(),
  listenForAppUpdate: vi.fn(),
  listenForAppUpdateProgress: vi.fn(),
  listenForSettingsNavigation: vi.fn(),
  listenForUsageHistory: vi.fn(),
}));

const openerMocks = vi.hoisted(() => ({ openUrl: vi.fn() }));
const clipboardMocks = vi.hoisted(() => ({ writeText: vi.fn() }));
const windowMocks = vi.hoisted(() => ({ setBackgroundColor: vi.fn(), setTheme: vi.fn() }));

vi.mock('./bridge', () => ({ usageBridge: bridgeMocks }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => windowMocks }));
vi.mock('@tauri-apps/plugin-opener', () => openerMocks);
vi.mock('@tauri-apps/plugin-clipboard-manager', () => clipboardMocks);

const zhSettings: Settings = { ...defaultSettings, language: 'zh-CN' };
const enSettings: Settings = { ...defaultSettings, language: 'en' };

const availableUpdate: AppUpdateInfo = {
  currentVersion: '0.2.5',
  latestVersion: '0.2.6',
  updateAvailable: true,
  checkedAt: '2030-01-04T12:00:00Z',
};

const emptyHistory: UsageHistoryResponse = {
  range: '24h',
  historyEnabled: true,
  storageStatus: 'empty',
  sampleCount: 0,
  earliestSampleAt: null,
  latestSampleAt: null,
  series: [],
};

function installMatchMedia(prefersDark = false) {
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => ({
      matches: prefersDark,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
}

function prepareBridge(settings: Settings = zhSettings) {
  bridgeMocks.getSettings.mockResolvedValue(settings);
  bridgeMocks.saveSettings.mockImplementation(async (next: Settings) => next);
  bridgeMocks.setNotificationsEnabled.mockImplementation(
    async (enabled: boolean): Promise<NotificationEnableResult> => ({
      enabled,
      permission: 'granted',
      settings: { ...settings, notifications: { ...settings.notifications, enabled } },
    }),
  );
  bridgeMocks.sendTestNotification.mockResolvedValue(undefined);
  bridgeMocks.getUsageHistory.mockResolvedValue(emptyHistory);
  bridgeMocks.setHistoryEnabled.mockImplementation(async (enabled: boolean) => ({ ...settings, historyEnabled: enabled }));
  bridgeMocks.clearUsageHistory.mockResolvedValue(undefined);
  bridgeMocks.getDiagnostics.mockResolvedValue('CodexUsageBar diagnostics schema: 1\nplatform: macos');
  bridgeMocks.getAutostart.mockResolvedValue(false);
  bridgeMocks.setAutostart.mockImplementation(async (enabled: boolean) => enabled);
  bridgeMocks.getAppUpdateInfo.mockResolvedValue({ ...availableUpdate, updateAvailable: false, latestVersion: '0.2.5' });
  bridgeMocks.checkAppUpdate.mockResolvedValue(availableUpdate);
  bridgeMocks.installAppUpdate.mockResolvedValue(undefined);
  windowMocks.setBackgroundColor.mockResolvedValue(undefined);
  windowMocks.setTheme.mockResolvedValue(undefined);
  bridgeMocks.listenForSettings.mockResolvedValue(() => undefined);
  bridgeMocks.listenForAppUpdate.mockResolvedValue(() => undefined);
  bridgeMocks.listenForAppUpdateProgress.mockResolvedValue(() => undefined);
  bridgeMocks.listenForSettingsNavigation.mockResolvedValue(() => undefined);
  bridgeMocks.listenForUsageHistory.mockResolvedValue(() => undefined);
  openerMocks.openUrl.mockResolvedValue(undefined);
  clipboardMocks.writeText.mockResolvedValue(undefined);
}

async function renderLoaded(settings: Settings = zhSettings) {
  prepareBridge(settings);
  render(<SettingsWindow />);
  await screen.findByRole('heading', { name: settings.language === 'en' ? 'Display' : '显示' });
}

beforeEach(() => installMatchMedia());

afterEach(async () => {
  vi.clearAllMocks();
  vi.unstubAllGlobals();
  await applyLanguagePreference('zh-CN');
});

describe('SettingsWindow', () => {
  it.each([
    ['light', 'light'],
    ['dark', 'dark'],
  ] as const)('keeps the native title bar in sync with the %s theme', async (theme, expectedTheme) => {
    await renderLoaded({ ...zhSettings, theme });

    await waitFor(() => expect(windowMocks.setTheme).toHaveBeenLastCalledWith(expectedTheme));
    expect(windowMocks.setBackgroundColor).toHaveBeenLastCalledWith(theme === 'dark' ? '#1b1f24' : '#fbfcff');
  });

  it('leaves the native title bar in automatic mode when following the system theme', async () => {
    installMatchMedia(true);
    await renderLoaded({ ...zhSettings, theme: 'system' });

    await waitFor(() => expect(windowMocks.setTheme).toHaveBeenLastCalledWith(null));
  });

  it('provides five real sidebar categories and keeps refresh intervals fixed', async () => {
    await renderLoaded();

    expect(screen.getByRole('button', { name: '显示' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '数据与刷新' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '趋势' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '启动' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '关于与更新' })).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: '数据与刷新' }));
    expect(await screen.findByRole('heading', { name: '数据与刷新' })).toBeTruthy();
    const refreshSelect = screen.getByRole('combobox', { name: '自动刷新' });
    fireEvent.mouseDown(refreshSelect);
    for (const minutes of [1, 3, 5, 10, 30]) {
      expect(await screen.findByRole('option', { name: `${minutes} 分钟` })).toBeTruthy();
    }
    expect(screen.queryByRole('option', { name: /智能/ })).toBeNull();
    fireEvent.click(screen.getByRole('option', { name: '1 分钟' }));

    fireEvent.click(screen.getByRole('button', { name: '启动' }));
    expect(await screen.findByRole('heading', { name: '启动' })).toBeTruthy();
    expect(screen.getByRole('switch', { name: '开机启动' })).toBeTruthy();
  });

  it('persists display settings and reports the result', async () => {
    await renderLoaded();

    fireEvent.click(screen.getByRole('switch', { name: '始终置顶' }));
    await waitFor(() => expect(bridgeMocks.saveSettings).toHaveBeenCalledWith({ ...zhSettings, alwaysOnTop: true }));
    expect(await screen.findByText('设置已保存。')).toBeTruthy();
  });

  it('registers every listener before authoritative reads and preserves a newer settings event', async () => {
    const order: string[] = [];
    const newerSettings: Settings = { ...zhSettings, theme: 'dark' };
    let settingsListener: ((settings: Settings) => void) | undefined;
    prepareBridge();
    bridgeMocks.listenForSettings.mockImplementation(async (handler: (settings: Settings) => void) => {
      order.push('listen-settings');
      settingsListener = handler;
      return () => undefined;
    });
    bridgeMocks.listenForAppUpdate.mockImplementation(async () => {
      order.push('listen-update');
      return () => undefined;
    });
    bridgeMocks.listenForAppUpdateProgress.mockImplementation(async () => {
      order.push('listen-progress');
      return () => undefined;
    });
    bridgeMocks.listenForSettingsNavigation.mockImplementation(async () => {
      order.push('listen-navigation');
      return () => undefined;
    });
    bridgeMocks.getSettings.mockImplementation(async () => {
      order.push('get-settings');
      settingsListener?.(newerSettings);
      return zhSettings;
    });
    bridgeMocks.getAutostart.mockImplementation(async () => {
      order.push('get-autostart');
      return false;
    });
    bridgeMocks.getAppUpdateInfo.mockImplementation(async () => {
      order.push('get-update');
      return { ...availableUpdate, updateAvailable: false };
    });

    render(<SettingsWindow />);
    await screen.findByRole('heading', { name: '显示' });
    await waitFor(() => expect(windowMocks.setTheme).toHaveBeenLastCalledWith('dark'));

    const firstRead = Math.min(
      order.indexOf('get-settings'),
      order.indexOf('get-autostart'),
      order.indexOf('get-update'),
    );
    for (const listener of ['listen-settings', 'listen-update', 'listen-progress', 'listen-navigation']) {
      expect(order.indexOf(listener)).toBeLessThan(firstRead);
    }
  });

  it('disables settings writes when the authoritative settings read fails', async () => {
    prepareBridge();
    bridgeMocks.getSettings.mockRejectedValueOnce(new Error('bridge unavailable'));
    render(<SettingsWindow />);

    await screen.findByRole('heading', { name: /显示|Display/ });
    expect((await screen.findAllByText(/无法读取当前设置|Unable to read current settings/)).length).toBeGreaterThan(0);
    const alwaysOnTop = screen.getByRole('switch', { name: /始终置顶|Always on top/ });
    expect((alwaysOnTop as HTMLInputElement).disabled).toBe(true);
    fireEvent.click(alwaysOnTop);
    expect(bridgeMocks.saveSettings).not.toHaveBeenCalled();
  });

  it('keeps loaded settings editable when non-critical startup reads fail', async () => {
    prepareBridge();
    bridgeMocks.getAutostart.mockRejectedValueOnce(new Error('autostart unavailable'));
    bridgeMocks.getAppUpdateInfo.mockRejectedValueOnce(new Error('update cache unavailable'));
    render(<SettingsWindow />);

    await screen.findByRole('heading', { name: '显示' });
    const alwaysOnTop = screen.getByRole('switch', { name: '始终置顶' });
    expect((alwaysOnTop as HTMLInputElement).disabled).toBe(false);
    fireEvent.click(alwaysOnTop);
    await waitFor(() => expect(bridgeMocks.saveSettings).toHaveBeenCalledWith({ ...zhSettings, alwaysOnTop: true }));

    fireEvent.click(screen.getByRole('button', { name: '启动' }));
    expect((await screen.findByRole('switch', { name: '开机启动' }) as HTMLInputElement).disabled).toBe(true);
  });

  it('serializes rapid full-settings writes and merges each change into the latest confirmed snapshot', async () => {
    let releaseFirstSave: (() => void) | undefined;
    const firstSaveBlocked = new Promise<void>((resolve) => {
      releaseFirstSave = resolve;
    });
    prepareBridge();
    bridgeMocks.saveSettings.mockImplementation(async (next: Settings) => {
      if (bridgeMocks.saveSettings.mock.calls.length === 1) await firstSaveBlocked;
      return next;
    });
    render(<SettingsWindow />);
    await screen.findByRole('heading', { name: '显示' });

    const alwaysOnTop = screen.getByRole('switch', { name: '始终置顶' });
    const lockPosition = screen.getByRole('switch', { name: '锁定位置与大小' });
    act(() => {
      alwaysOnTop.click();
      lockPosition.click();
    });

    await waitFor(() => expect(bridgeMocks.saveSettings).toHaveBeenCalledTimes(1));
    expect(bridgeMocks.saveSettings).toHaveBeenNthCalledWith(1, { ...zhSettings, alwaysOnTop: true });
    releaseFirstSave?.();
    await waitFor(() => expect(bridgeMocks.saveSettings).toHaveBeenCalledTimes(2));
    expect(bridgeMocks.saveSettings).toHaveBeenNthCalledWith(2, {
      ...zhSettings,
      alwaysOnTop: true,
      lockPosition: true,
    });
  });

  it('serializes notification permission changes with rule edits without dropping fields', async () => {
    let persisted = zhSettings;
    prepareBridge();
    bridgeMocks.setNotificationsEnabled.mockImplementation(async (enabled: boolean) => {
      persisted = { ...persisted, notifications: { ...persisted.notifications, enabled } };
      return { enabled, permission: 'granted', settings: persisted };
    });
    bridgeMocks.saveSettings.mockImplementation(async (next: Settings) => {
      persisted = next;
      return persisted;
    });
    render(<SettingsWindow />);
    await screen.findByRole('heading', { name: '显示' });
    fireEvent.click(screen.getByRole('button', { name: '数据与刷新' }));

    const notificationsEnabled = await screen.findByRole('switch', { name: '启用系统通知' });
    const lowQuotaEnabled = screen.getByRole('switch', { name: '低额度提醒' });
    act(() => {
      notificationsEnabled.click();
      lowQuotaEnabled.click();
    });

    await waitFor(() => expect(bridgeMocks.setNotificationsEnabled).toHaveBeenCalledWith(true));
    await waitFor(() => expect(bridgeMocks.saveSettings).toHaveBeenCalledTimes(1));
    expect(bridgeMocks.saveSettings).toHaveBeenLastCalledWith({
      ...zhSettings,
      notifications: {
        ...zhSettings.notifications,
        enabled: true,
        lowQuotaEnabled: false,
      },
    });
  });

  it('loads English and applies a language selection immediately', async () => {
    await renderLoaded(enSettings);

    expect(screen.getByRole('button', { name: 'Display' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Data & refresh' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Trends' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'About & updates' })).toBeTruthy();
    expect(document.documentElement.lang).toBe('en');

    const languageSelect = screen.getByRole('combobox', { name: 'Language' });
    fireEvent.mouseDown(languageSelect);
    expect(await screen.findByRole('option', { name: 'Use system language' })).toBeTruthy();
    expect(screen.getByRole('option', { name: '简体中文' })).toBeTruthy();
    fireEvent.click(screen.getByRole('option', { name: '简体中文' }));

    expect(await screen.findByRole('heading', { name: '显示' })).toBeTruthy();
    expect(document.documentElement.lang).toBe('zh-CN');
    await waitFor(() =>
      expect(bridgeMocks.saveSettings).toHaveBeenCalledWith({ ...enSettings, language: 'zh-CN' }),
    );
  });

  it('keeps notifications off and explains System Settings when permission is denied', async () => {
    prepareBridge();
    bridgeMocks.setNotificationsEnabled.mockResolvedValue({
      enabled: false,
      permission: 'denied',
      settings: zhSettings,
    });
    render(<SettingsWindow />);
    await screen.findByRole('heading', { name: '显示' });
    fireEvent.click(screen.getByRole('button', { name: '数据与刷新' }));

    const notificationsSwitch = await screen.findByRole('switch', { name: '启用系统通知' });
    fireEvent.click(notificationsSwitch);

    await waitFor(() => expect(bridgeMocks.setNotificationsEnabled).toHaveBeenCalledWith(true));
    expect((notificationsSwitch as HTMLInputElement).checked).toBe(false);
    expect(
      await screen.findByText(
        '系统通知权限未获允许，通知仍保持关闭。请前往系统设置中的通知权限为 CodexUsageBar 开启通知。',
      ),
    ).toBeTruthy();
  });

  it('persists notification rules with bounded thresholds and quiet-hour defaults', async () => {
    await renderLoaded();
    fireEvent.click(screen.getByRole('button', { name: '数据与刷新' }));

    expect((screen.getByRole('spinbutton', { name: '低额度阈值 (%)' }) as HTMLInputElement).value).toBe('20');
    expect((screen.getByRole('spinbutton', { name: '落后建议进度（百分点）' }) as HTMLInputElement).value).toBe('10');
    const quietSwitch = screen.getByRole('switch', { name: '静默时段' });
    fireEvent.click(quietSwitch);
    await waitFor(() =>
      expect(bridgeMocks.saveSettings).toHaveBeenCalledWith({
        ...zhSettings,
        notifications: { ...zhSettings.notifications, quietHoursEnabled: true },
      }),
    );

    const lowThreshold = screen.getByRole('spinbutton', { name: '低额度阈值 (%)' });
    fireEvent.change(lowThreshold, { target: { value: '120' } });
    fireEvent.blur(lowThreshold);
    await waitFor(() =>
      expect(bridgeMocks.saveSettings).toHaveBeenLastCalledWith({
        ...zhSettings,
        notifications: {
          ...zhSettings.notifications,
          quietHoursEnabled: true,
          lowQuotaThresholdPercent: 100,
        },
      }),
    );
  });

  it('sends a test notification only after notifications are enabled', async () => {
    const enabledSettings: Settings = {
      ...zhSettings,
      notifications: { ...zhSettings.notifications, enabled: true },
    };
    await renderLoaded(enabledSettings);
    fireEvent.click(screen.getByRole('button', { name: '数据与刷新' }));

    fireEvent.click(await screen.findByRole('button', { name: '发送测试通知' }));

    await waitFor(() => expect(bridgeMocks.sendTestNotification).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('测试通知已提交给系统。')).toBeTruthy();
  });

  it('lazy-loads the dedicated trends page through sanitized history IPC', async () => {
    await renderLoaded();

    fireEvent.click(screen.getByRole('button', { name: '趋势' }));

    expect(await screen.findByRole('heading', { name: '用量趋势' }, { timeout: 5_000 })).toBeTruthy();
    await waitFor(() => expect(bridgeMocks.getUsageHistory).toHaveBeenCalledWith('24h'));
    expect(bridgeMocks.getDashboard).not.toHaveBeenCalled();
  });

  it('persists the optional automatic update check preference', async () => {
    await renderLoaded();
    fireEvent.click(screen.getByRole('button', { name: '关于与更新' }));
    fireEvent.click(await screen.findByRole('switch', { name: '自动检查更新' }));

    await waitFor(() => expect(bridgeMocks.saveSettings).toHaveBeenCalledWith({ ...zhSettings, autoCheckUpdates: false }));
    expect(await screen.findByText('设置已保存。')).toBeTruthy();
  });

  it('keeps sanitized diagnostics, issue reporting, and release notes as separate actions', async () => {
    const diagnostics = 'CodexUsageBar diagnostics schema: 1\nplatform: macos';
    await renderLoaded();
    bridgeMocks.getDiagnostics.mockResolvedValueOnce(diagnostics);
    fireEvent.click(screen.getByRole('button', { name: '关于与更新' }));

    fireEvent.click(await screen.findByRole('button', { name: '复制脱敏诊断' }));
    await waitFor(() => expect(clipboardMocks.writeText).toHaveBeenCalledWith(diagnostics));
    expect(await screen.findByText('脱敏诊断已复制到剪贴板。')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: '反馈问题' }));
    await waitFor(() =>
      expect(openerMocks.openUrl).toHaveBeenCalledWith(
        'https://github.com/creamtea47/codex-usage-bar/issues/new/choose',
      ),
    );
    expect(openerMocks.openUrl.mock.calls.at(-1)?.[0]).not.toContain('diagnostics');
    expect(openerMocks.openUrl.mock.calls.at(-1)?.[0]).not.toContain('platform');

    fireEvent.click(screen.getByRole('button', { name: '查看更新说明' }));
    await waitFor(() =>
      expect(openerMocks.openUrl).toHaveBeenCalledWith(
        'https://github.com/creamtea47/codex-usage-bar/releases/tag/v0.2.5',
      ),
    );
  });

  it('falls back to the Releases index when the version is not a safe tag', async () => {
    prepareBridge();
    bridgeMocks.getAppUpdateInfo.mockResolvedValue({
      ...availableUpdate,
      currentVersion: '../../private',
      updateAvailable: false,
    });
    render(<SettingsWindow />);
    await screen.findByRole('heading', { name: '显示' });
    fireEvent.click(screen.getByRole('button', { name: '关于与更新' }));
    fireEvent.click(await screen.findByRole('button', { name: '查看更新说明' }));

    await waitFor(() => expect(openerMocks.openUrl).toHaveBeenCalledWith('https://github.com/creamtea47/codex-usage-bar/releases'));
  });

  it('checks signed update metadata and only installs after user confirmation', async () => {
    await renderLoaded();
    fireEvent.click(screen.getByRole('button', { name: '关于与更新' }));
    fireEvent.click(await screen.findByRole('button', { name: '检查更新' }));

    await waitFor(() => expect(bridgeMocks.checkAppUpdate).toHaveBeenCalledTimes(1));
    expect((await screen.findAllByText('发现新版本 v0.2.6')).length).toBeGreaterThan(0);
    expect(screen.getByRole('button', { name: '下载并安装' })).toBeTruthy();
    expect(bridgeMocks.installAppUpdate).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: '发布页' }));
    await waitFor(() => expect(openerMocks.openUrl).toHaveBeenCalledWith('https://github.com/creamtea47/codex-usage-bar/releases'));

    fireEvent.click(screen.getByRole('button', { name: '下载并安装' }));
    await waitFor(() => expect(bridgeMocks.installAppUpdate).toHaveBeenCalledTimes(1));
  });

  it('opens the available-update dialog when the main card navigates to updates', async () => {
    let navigate: ((section: 'about') => void) | undefined;
    prepareBridge();
    bridgeMocks.getAppUpdateInfo.mockResolvedValue(availableUpdate);
    bridgeMocks.listenForSettingsNavigation.mockImplementation(async (handler: (section: 'about') => void) => {
      navigate = handler;
      return () => undefined;
    });
    render(<SettingsWindow />);
    await waitFor(() => expect(navigate).toBeTypeOf('function'));

    await act(async () => navigate?.('about'));

    expect(await screen.findByRole('heading', { name: '发现新版本 v0.2.6' })).toBeTruthy();
    expect(bridgeMocks.installAppUpdate).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: '关闭' }));
    expect(await screen.findByRole('heading', { name: '关于与更新' })).toBeTruthy();
  });

  it('shows the safe signature-verification failure without exposing transport details', async () => {
    await renderLoaded();
    bridgeMocks.checkAppUpdate.mockRejectedValueOnce(new Error('更新包的签名验证失败，已取消安装。 https://private.invalid/detail'));
    fireEvent.click(screen.getByRole('button', { name: '关于与更新' }));
    fireEvent.click(await screen.findByRole('button', { name: '检查更新' }));

    expect(await screen.findByText('更新包的签名验证失败，已取消安装。')).toBeTruthy();
    expect(screen.queryByText(/private\.invalid/)).toBeNull();
  });
});
