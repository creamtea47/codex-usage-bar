import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import App from './App';
import { applyLanguagePreference } from './i18n';
import {
  defaultSettings,
  loadingSnapshot,
  type AppUpdateInfo,
  type DashboardSnapshot,
  type Settings,
} from './types';

const bridgeMocks = vi.hoisted(() => ({
  getDashboard: vi.fn(),
  refreshDashboard: vi.fn(),
  getSettings: vi.fn(),
  saveSettings: vi.fn(),
  getAutostart: vi.fn(),
  setAutostart: vi.fn(),
  getAppUpdateInfo: vi.fn(),
  checkAppUpdate: vi.fn(),
  installAppUpdate: vi.fn(),
  openSettingsWindow: vi.fn(),
  markMainSizeManual: vi.fn(),
  listenForDashboard: vi.fn(),
  listenForSettings: vi.fn(),
  listenForAppUpdate: vi.fn(),
  listenForAppUpdateProgress: vi.fn(),
}));

const windowMocks = vi.hoisted(() => ({
  close: vi.fn(),
  startDragging: vi.fn(),
  startResizeDragging: vi.fn(),
}));

vi.mock('./bridge', () => ({ usageBridge: bridgeMocks }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => windowMocks }));

const zhSettings: Settings = { ...defaultSettings, language: 'zh-CN' };
const enSettings: Settings = { ...defaultSettings, language: 'en' };

const readySnapshot: DashboardSnapshot = {
  status: 'ready',
  accountEmailMasked: 'j***@example.com',
  planLabel: 'Plus',
  refreshedAt: '2030-01-04T12:00:00Z',
  nextRefreshAt: '2030-01-04T12:05:00Z',
  message: null,
  quotaWindows: [
    {
      id: 'week',
      label: '周限额',
      remainingPercent: 72,
      usedPercent: 28,
      windowSeconds: 604800,
      resetAt: '2030-01-08T00:00:00Z',
      resetAfterSeconds: 86400,
      startAt: '2030-01-01T00:00:00Z',
      showPaceMarker: true,
      fallbackLabel: 'weekly',
      forecast: null,
    },
  ],
};

function installMatchMedia() {
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
}

function prepareBridge(snapshot: DashboardSnapshot = readySnapshot, settings: Settings = zhSettings) {
  bridgeMocks.getDashboard.mockResolvedValue(snapshot);
  bridgeMocks.refreshDashboard.mockResolvedValue(snapshot);
  bridgeMocks.getSettings.mockResolvedValue(settings);
  bridgeMocks.saveSettings.mockImplementation(async (next: Settings) => next);
  bridgeMocks.getAutostart.mockResolvedValue(false);
  bridgeMocks.setAutostart.mockImplementation(async (enabled: boolean) => enabled);
  bridgeMocks.getAppUpdateInfo.mockResolvedValue({
    currentVersion: '0.2.6',
    latestVersion: '0.2.6',
    updateAvailable: false,
    checkedAt: null,
  });
  bridgeMocks.checkAppUpdate.mockResolvedValue(null);
  bridgeMocks.installAppUpdate.mockResolvedValue(undefined);
  bridgeMocks.openSettingsWindow.mockResolvedValue(undefined);
  bridgeMocks.markMainSizeManual.mockResolvedValue(undefined);
  bridgeMocks.listenForDashboard.mockResolvedValue(() => undefined);
  bridgeMocks.listenForSettings.mockResolvedValue(() => undefined);
  bridgeMocks.listenForAppUpdate.mockResolvedValue(() => undefined);
  bridgeMocks.listenForAppUpdateProgress.mockResolvedValue(() => undefined);
}

async function renderLoaded(settings: Settings = zhSettings) {
  prepareBridge(readySnapshot, settings);
  const result = render(<App />);
  await screen.findByText('j***@example.com · Plus');
  return result;
}

beforeEach(() => {
  installMatchMedia();
  windowMocks.close.mockResolvedValue(undefined);
  windowMocks.startDragging.mockResolvedValue(undefined);
  windowMocks.startResizeDragging.mockResolvedValue(undefined);
});

afterEach(async () => {
  vi.clearAllMocks();
  vi.unstubAllGlobals();
  await applyLanguagePreference('zh-CN');
});

describe('App', () => {
  it('renders loading, stale, and error dashboard states', async () => {
    prepareBridge(loadingSnapshot);
    const { unmount } = render(<App />);
    expect(await screen.findByText('正在从 Codex 读取额度…')).toBeTruthy();
    unmount();

    prepareBridge({ ...readySnapshot, status: 'stale', message: 'authInvalid' });
    const staleView = render(<App />);
    expect(await screen.findByText('展示上次数据')).toBeTruthy();
    expect(screen.getByText('Codex 登录已失效，请重新登录。')).toBeTruthy();
    expect(screen.getByText(/自动重试 \d+:\d{2} · 更新于/)).toBeTruthy();
    staleView.unmount();

    prepareBridge({ ...loadingSnapshot, status: 'error', message: 'authMissing' });
    render(<App />);
    expect(await screen.findByText('无法读取用量')).toBeTruthy();
    expect(screen.getByText('未找到 Codex 登录信息，请先登录 Codex。')).toBeTruthy();
  });

  it('loads English settings and localizes content, controls, dates, tooltips, and ARIA labels', async () => {
    prepareBridge(
      {
        ...readySnapshot,
        quotaWindows: readySnapshot.quotaWindows.map((quotaWindow) => ({ ...quotaWindow, label: null })),
      },
      enSettings,
    );
    render(<App />);

    expect(await screen.findByText('Codex Usage')).toBeTruthy();
    expect(screen.getByText('Codex quota')).toBeTruthy();
    expect(screen.getByText('Weekly limit')).toBeTruthy();
    expect(screen.getByText(/Next automatic refresh \d+:\d{2} · Updated/)).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Refresh usage' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Open settings' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Quit app' })).toBeTruthy();
    expect(screen.getByRole('region', { name: 'Quota windows' })).toBeTruthy();
    expect(screen.getByLabelText('Usage service online')).toBeTruthy();
    expect(document.documentElement.lang).toBe('en');
  });

  it('applies language changes received from the settings event', async () => {
    let settingsHandler: ((settings: Settings) => void) | undefined;
    prepareBridge(readySnapshot, zhSettings);
    bridgeMocks.listenForSettings.mockImplementation(async (handler: (settings: Settings) => void) => {
      settingsHandler = handler;
      return () => undefined;
    });
    render(<App />);

    expect(await screen.findByRole('button', { name: '刷新用量' })).toBeTruthy();
    await act(async () => settingsHandler?.(enSettings));
    expect(await screen.findByRole('button', { name: 'Refresh usage' })).toBeTruthy();
    expect(document.documentElement.lang).toBe('en');
  });

  it('registers every listener before initial reads and keeps events emitted as reads begin', async () => {
    let dashboardHandler: ((snapshot: DashboardSnapshot) => void) | undefined;
    let settingsHandler: ((settings: Settings) => void) | undefined;
    let updateHandler: ((info: AppUpdateInfo) => void) | undefined;
    const newerUpdate: AppUpdateInfo = {
      currentVersion: '0.2.6',
      latestVersion: '0.2.7',
      updateAvailable: true,
      checkedAt: '2030-01-04T12:00:00Z',
    };
    prepareBridge(loadingSnapshot, zhSettings);
    bridgeMocks.listenForDashboard.mockImplementation(async (handler: (snapshot: DashboardSnapshot) => void) => {
      dashboardHandler = handler;
      return () => undefined;
    });
    bridgeMocks.listenForSettings.mockImplementation(async (handler: (settings: Settings) => void) => {
      settingsHandler = handler;
      return () => undefined;
    });
    bridgeMocks.listenForAppUpdate.mockImplementation(async (handler: (info: AppUpdateInfo) => void) => {
      updateHandler = handler;
      return () => undefined;
    });
    bridgeMocks.getDashboard.mockImplementation(async () => {
      dashboardHandler?.(readySnapshot);
      return loadingSnapshot;
    });
    bridgeMocks.getSettings.mockImplementation(async () => {
      settingsHandler?.(enSettings);
      return zhSettings;
    });
    bridgeMocks.getAppUpdateInfo.mockImplementation(async () => {
      updateHandler?.(newerUpdate);
      return {
        currentVersion: '0.2.6',
        latestVersion: '0.2.6',
        updateAvailable: false,
        checkedAt: null,
      };
    });

    render(<App />);

    expect(await screen.findByText('j***@example.com · Plus')).toBeTruthy();
    expect(await screen.findByRole('button', { name: 'Refresh usage' })).toBeTruthy();
    expect(await screen.findByRole('button', { name: 'Version v0.2.7 is available; open Update settings' })).toBeTruthy();
    const lastListenerCall = Math.max(
      bridgeMocks.listenForDashboard.mock.invocationCallOrder[0],
      bridgeMocks.listenForSettings.mock.invocationCallOrder[0],
      bridgeMocks.listenForAppUpdate.mock.invocationCallOrder[0],
    );
    const firstReadCall = Math.min(
      bridgeMocks.getDashboard.mock.invocationCallOrder[0],
      bridgeMocks.getSettings.mock.invocationCallOrder[0],
      bridgeMocks.getAppUpdateInfo.mock.invocationCallOrder[0],
    );
    expect(lastListenerCall).toBeLessThan(firstReadCall);
  });

  it('cleans up partial listener registration and skips startup reads when a subscription fails', async () => {
    const removeDashboardListener = vi.fn();
    const removeUpdateListener = vi.fn();
    prepareBridge();
    bridgeMocks.listenForDashboard.mockResolvedValue(removeDashboardListener);
    bridgeMocks.listenForSettings.mockRejectedValue(new Error('settings listener failed'));
    bridgeMocks.listenForAppUpdate.mockResolvedValue(removeUpdateListener);

    const { unmount } = render(<App />);

    expect(await screen.findByText('Unable to read usage')).toBeTruthy();
    expect(
      screen.getByText(
        'Unable to connect to the local usage service. Launch this page from the CodexUsageBar desktop app.',
      ),
    ).toBeTruthy();
    expect(removeDashboardListener).toHaveBeenCalledTimes(1);
    expect(removeUpdateListener).toHaveBeenCalledTimes(1);
    expect(bridgeMocks.getDashboard).not.toHaveBeenCalled();
    expect(bridgeMocks.getSettings).not.toHaveBeenCalled();
    expect(bridgeMocks.getAppUpdateInfo).not.toHaveBeenCalled();

    unmount();
    expect(removeDashboardListener).toHaveBeenCalledTimes(1);
    expect(removeUpdateListener).toHaveBeenCalledTimes(1);
  });

  it('uses a localized generic message for an unknown dashboard error code', async () => {
    prepareBridge({
      ...loadingSnapshot,
      status: 'error',
      message: 'futureErrorCode' as unknown as DashboardSnapshot['message'],
    });
    const { unmount } = render(<App />);
    expect(await screen.findByText('暂时无法读取用量，请稍后重试。')).toBeTruthy();
    unmount();

    prepareBridge({
      ...loadingSnapshot,
      status: 'error',
      message: 'toString' as unknown as DashboardSnapshot['message'],
    });
    const prototypeKeyView = render(<App />);
    expect(await screen.findByText('暂时无法读取用量，请稍后重试。')).toBeTruthy();
    prototypeKeyView.unmount();

    prepareBridge(
      {
        ...loadingSnapshot,
        status: 'error',
        message: 'futureErrorCode' as unknown as DashboardSnapshot['message'],
      },
      enSettings,
    );
    render(<App />);
    expect(await screen.findByText('Unable to read usage right now. Try again later.')).toBeTruthy();
  });

  it('shows the masked account, plan, refresh countdown, and no legacy success title', async () => {
    await renderLoaded();

    expect(screen.getByText('j***@example.com · Plus')).toBeTruthy();
    expect(screen.getByText(/下次自动刷新 \d+:\d{2} · 更新于/)).toBeTruthy();
    expect(screen.queryByText('数据已更新')).toBeNull();
  });

  it('falls back to the plan summary when Rust has no masked email', async () => {
    prepareBridge({ ...readySnapshot, accountEmailMasked: null });
    render(<App />);

    expect(await screen.findByText('Plus')).toBeTruthy();
    expect(screen.queryByText('null · Plus')).toBeNull();
  });

  it.each([
    ['system', 'light'],
    ['light', 'light'],
    ['dark', 'dark'],
  ] as const)('applies the %s theme', async (theme, expectedMode) => {
    await renderLoaded({ ...zhSettings, theme });
    await waitFor(() => expect(screen.getByRole('main').getAttribute('data-theme-mode')).toBe(expectedMode));
  });

  it('disables the top refresh icon until a manual refresh completes', async () => {
    await renderLoaded();
    let finishRefresh: ((snapshot: DashboardSnapshot) => void) | undefined;
    bridgeMocks.refreshDashboard.mockImplementation(
      () =>
        new Promise<DashboardSnapshot>((resolve) => {
          finishRefresh = resolve;
        }),
    );

    const refreshButton = screen.getByRole('button', { name: '刷新用量' }) as HTMLButtonElement;
    fireEvent.click(refreshButton);
    await waitFor(() => expect(refreshButton.disabled).toBe(true));

    await act(async () => finishRefresh?.(readySnapshot));
    await waitFor(() => expect(refreshButton.disabled).toBe(false));
  });

  it('opens the independent settings window through the constrained IPC', async () => {
    await renderLoaded();

    fireEvent.click(screen.getByRole('button', { name: '打开设置' }));
    await waitFor(() => expect(bridgeMocks.openSettingsWindow).toHaveBeenCalledTimes(1));
  });

  it('marks the size as manual only when the user starts a resize drag', async () => {
    const { container } = await renderLoaded();

    const eastEdge = container.querySelector('[data-resize-direction="East"]');
    expect(eastEdge).toBeTruthy();
    fireEvent.mouseDown(eastEdge!);

    expect(bridgeMocks.markMainSizeManual).toHaveBeenCalledTimes(1);
    expect(windowMocks.startResizeDragging).toHaveBeenCalledWith('East');
  });

  it('shows a feedback message when opening the settings window fails', async () => {
    await renderLoaded();
    bridgeMocks.openSettingsWindow.mockRejectedValueOnce(new Error('unavailable'));

    fireEvent.click(screen.getByRole('button', { name: '打开设置' }));
    expect(await screen.findByText('无法打开设置窗口，请稍后重试。')).toBeTruthy();
    expect(screen.getByRole('button', { name: '关闭' })).toBeTruthy();
  });

  it('localizes manual-refresh failures without exposing the raw bridge error', async () => {
    await renderLoaded(enSettings);
    bridgeMocks.refreshDashboard.mockRejectedValueOnce(new Error('private raw bridge detail'));

    fireEvent.click(screen.getByRole('button', { name: 'Refresh usage' }));
    expect(await screen.findByText('The refresh did not complete. Try again later.')).toBeTruthy();
    expect(screen.queryByText('private raw bridge detail')).toBeNull();
    expect(screen.getByRole('button', { name: 'Close' })).toBeTruthy();
  });

  it('maps a startup bridge failure to the stable localized bridge error', async () => {
    prepareBridge(readySnapshot, enSettings);
    bridgeMocks.getDashboard.mockRejectedValueOnce(new Error('private local path'));

    render(<App />);

    expect(
      await screen.findByText(
        'Unable to connect to the local usage service. Launch this page from the CodexUsageBar desktop app.',
      ),
    ).toBeTruthy();
    expect(screen.queryByText('private local path')).toBeNull();
  });

  it('shows a persistent top-right update icon and opens settings on demand', async () => {
    let updateHandler: ((info: AppUpdateInfo) => void) | undefined;
    prepareBridge();
    bridgeMocks.listenForAppUpdate.mockImplementation(async (handler: (info: AppUpdateInfo) => void) => {
      updateHandler = handler;
      return () => undefined;
    });
    render(<App />);
    await screen.findByText('j***@example.com · Plus');
    await act(async () => {
      updateHandler?.({
        currentVersion: '0.2.6',
        latestVersion: '0.2.7',
        updateAvailable: true,
        checkedAt: '2030-01-04T12:00:00Z',
      });
    });
    const updateButton = await screen.findByRole('button', { name: '发现新版本 v0.2.7，打开更新设置' });
    expect(screen.getByRole('status').textContent).toBe('发现新版本 v0.2.7，可打开更新设置');
    expect(screen.queryByText('发现新版本 v0.2.7，可在设置中下载并验证签名。')).toBeNull();
    fireEvent.click(updateButton);
    await waitFor(() => expect(bridgeMocks.openSettingsWindow).toHaveBeenCalledWith('about'));
    expect(screen.getByRole('button', { name: '发现新版本 v0.2.7，打开更新设置' })).toBeTruthy();

    await act(async () => {
      updateHandler?.({
        currentVersion: '0.2.7',
        latestVersion: '0.2.7',
        updateAvailable: false,
        checkedAt: '2030-01-04T12:01:00Z',
      });
    });
    expect(screen.queryByRole('button', { name: '发现新版本 v0.2.7，打开更新设置' })).toBeNull();
    expect(screen.getByRole('status').textContent).toBe('');
  });

  it('restores a pending update icon from the startup snapshot', async () => {
    prepareBridge();
    bridgeMocks.getAppUpdateInfo.mockResolvedValue({
      currentVersion: '0.2.6',
      latestVersion: '0.2.7',
      updateAvailable: true,
      checkedAt: '2030-01-04T12:00:00Z',
    });

    render(<App />);

    expect(await screen.findByRole('button', { name: '发现新版本 v0.2.7，打开更新设置' })).toBeTruthy();
  });

  it('does not let an older startup snapshot overwrite a newer update event', async () => {
    let updateHandler: ((info: AppUpdateInfo) => void) | undefined;
    let finishUpdateLookup: ((info: AppUpdateInfo) => void) | undefined;
    prepareBridge();
    bridgeMocks.getAppUpdateInfo.mockImplementation(
      () =>
        new Promise<AppUpdateInfo>((resolve) => {
          finishUpdateLookup = (info) => resolve(info);
        }),
    );
    bridgeMocks.listenForAppUpdate.mockImplementation(async (handler: (info: AppUpdateInfo) => void) => {
      updateHandler = handler;
      return () => undefined;
    });
    render(<App />);
    await waitFor(() => expect(updateHandler).toBeTypeOf('function'));

    await act(async () => {
      updateHandler?.({
        currentVersion: '0.2.6',
        latestVersion: '0.2.6',
        updateAvailable: false,
        checkedAt: '2030-01-04T12:01:00Z',
      });
      finishUpdateLookup?.({
        currentVersion: '0.2.6',
        latestVersion: '0.2.7',
        updateAvailable: true,
        checkedAt: '2030-01-04T12:00:00Z',
      });
    });

    await screen.findByText('j***@example.com · Plus');
    expect(screen.queryByRole('button', { name: '发现新版本 v0.2.7，打开更新设置' })).toBeNull();
  });
});
