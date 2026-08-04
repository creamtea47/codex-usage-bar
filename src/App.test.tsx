import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import App from './App';
import { defaultSettings, loadingSnapshot, type DashboardSnapshot, type Settings, type UpdateInfo } from './types';

const bridgeMocks = vi.hoisted(() => ({
  getDashboard: vi.fn(),
  refreshDashboard: vi.fn(),
  getSettings: vi.fn(),
  saveSettings: vi.fn(),
  getAutostart: vi.fn(),
  setAutostart: vi.fn(),
  checkUpdate: vi.fn(),
  listenForDashboard: vi.fn(),
}));

const openerMocks = vi.hoisted(() => ({ openUrl: vi.fn() }));

const windowMocks = vi.hoisted(() => ({
  close: vi.fn(),
  startDragging: vi.fn(),
  startResizeDragging: vi.fn(),
}));

vi.mock('./bridge', () => ({ usageBridge: bridgeMocks }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => windowMocks }));
vi.mock('@tauri-apps/plugin-opener', () => openerMocks);

const readySnapshot: DashboardSnapshot = {
  status: 'ready',
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
    },
  ],
};

const availableUpdate: UpdateInfo = {
  currentVersion: '0.2.0',
  latestVersion: '0.2.1',
  updateAvailable: true,
  releaseUrl: 'https://github.com/creamtea47/codex-usage-bar/releases/tag/v0.2.1',
  downloadUrl: 'https://github.com/creamtea47/codex-usage-bar/releases/download/v0.2.1/CodexUsageBar-x64-setup.exe',
  publishedAt: '2030-01-04T12:00:00Z',
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

function prepareBridge(snapshot: DashboardSnapshot = readySnapshot, settings: Settings = defaultSettings) {
  bridgeMocks.getDashboard.mockResolvedValue(snapshot);
  bridgeMocks.refreshDashboard.mockResolvedValue(snapshot);
  bridgeMocks.getSettings.mockResolvedValue(settings);
  bridgeMocks.saveSettings.mockImplementation(async (next: Settings) => next);
  bridgeMocks.getAutostart.mockResolvedValue(false);
  bridgeMocks.setAutostart.mockImplementation(async (enabled: boolean) => enabled);
  bridgeMocks.checkUpdate.mockResolvedValue(availableUpdate);
  bridgeMocks.listenForDashboard.mockResolvedValue(() => undefined);
}

async function renderLoaded(settings: Settings = defaultSettings) {
  prepareBridge(readySnapshot, settings);
  render(<App />);
  await screen.findByText('数据已更新');
}

beforeEach(() => {
  installMatchMedia();
  windowMocks.close.mockResolvedValue(undefined);
  windowMocks.startDragging.mockResolvedValue(undefined);
  windowMocks.startResizeDragging.mockResolvedValue(undefined);
});

afterEach(() => {
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

describe('App', () => {
  it('renders loading, stale, and error dashboard states', async () => {
    prepareBridge(loadingSnapshot);
    const { unmount } = render(<App />);
    expect(await screen.findByText('正在从 Codex 读取额度…')).toBeTruthy();
    unmount();

    prepareBridge({ ...readySnapshot, status: 'stale', message: '认证已过期，请重新登录 Codex。' });
    render(<App />);
    expect(await screen.findByText('展示上次数据')).toBeTruthy();
    expect(screen.getByText('认证已过期，请重新登录 Codex。')).toBeTruthy();

    prepareBridge({ ...loadingSnapshot, status: 'error', message: '无法读取本地凭据。' });
    render(<App />);
    expect(await screen.findByText('无法读取用量')).toBeTruthy();
    expect(screen.getByText('无法读取本地凭据。')).toBeTruthy();
  });

  it.each([
    ['system', 'light'],
    ['light', 'light'],
    ['dark', 'dark'],
  ] as const)('applies the %s theme', async (theme, expectedMode) => {
    await renderLoaded({ ...defaultSettings, theme });
    await waitFor(() => expect(screen.getByRole('main').getAttribute('data-theme-mode')).toBe(expectedMode));
  });

  it('disables the refresh button until a manual refresh completes', async () => {
    await renderLoaded();
    let finishRefresh: ((snapshot: DashboardSnapshot) => void) | undefined;
    bridgeMocks.refreshDashboard.mockImplementation(
      () =>
        new Promise<DashboardSnapshot>((resolve) => {
          finishRefresh = resolve;
        }),
    );

    const refreshButton = screen.getByRole('button', { name: '立即刷新' }) as HTMLButtonElement;
    fireEvent.click(refreshButton);
    await waitFor(() => expect(refreshButton.disabled).toBe(true));
    expect(refreshButton.textContent).toBe('刷新中…');

    await act(async () => finishRefresh?.(readySnapshot));
    await waitFor(() => expect(refreshButton.disabled).toBe(false));
  });

  it('calls the settings IPC and shows an error message when persistence fails', async () => {
    await renderLoaded();
    bridgeMocks.saveSettings.mockRejectedValueOnce(new Error('unavailable'));

    fireEvent.click(screen.getByRole('button', { name: '打开设置' }));
    fireEvent.click(screen.getByLabelText('始终置顶'));

    await waitFor(() =>
      expect(bridgeMocks.saveSettings).toHaveBeenCalledWith({ ...defaultSettings, alwaysOnTop: true }),
    );
    expect(await screen.findByText('无法保存设置。')).toBeTruthy();
  });

  it('shows confirmation after settings are persisted', async () => {
    await renderLoaded();

    fireEvent.click(screen.getByRole('button', { name: '打开设置' }));
    fireEvent.click(screen.getByLabelText('始终置顶'));

    expect(await screen.findByText('设置已保存。')).toBeTruthy();
  });

  it('exposes MUI settings controls with accessible names', async () => {
    await renderLoaded();

    fireEvent.click(screen.getByRole('button', { name: '打开设置' }));

    expect(screen.getByRole('switch', { name: '始终置顶' })).toBeTruthy();
    expect(screen.getByRole('switch', { name: '锁定位置与大小' })).toBeTruthy();
    expect(screen.getByRole('switch', { name: '开机启动' })).toBeTruthy();
    expect(screen.getByRole('combobox', { name: '自动刷新' })).toBeTruthy();
    expect(screen.getByRole('combobox', { name: '主题' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '检查更新' })).toBeTruthy();
  });

  it('checks the public release and opens the scoped release page only after a user action', async () => {
    await renderLoaded();

    fireEvent.click(screen.getByRole('button', { name: '打开设置' }));
    fireEvent.click(screen.getByRole('button', { name: '检查更新' }));

    await waitFor(() => expect(bridgeMocks.checkUpdate).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('发现新版本 v0.2.1')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '打开发布页' }));
    await waitFor(() => expect(openerMocks.openUrl).toHaveBeenCalledWith(availableUpdate.downloadUrl));
  });
});
