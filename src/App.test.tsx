import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import App from './App';
import { defaultSettings, loadingSnapshot, type AppUpdateInfo, type DashboardSnapshot, type Settings } from './types';

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

function prepareBridge(snapshot: DashboardSnapshot = readySnapshot, settings: Settings = defaultSettings) {
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

async function renderLoaded(settings: Settings = defaultSettings) {
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
    const staleView = render(<App />);
    expect(await screen.findByText('展示上次数据')).toBeTruthy();
    expect(screen.getByText('认证已过期，请重新登录 Codex。')).toBeTruthy();
    staleView.unmount();

    prepareBridge({ ...loadingSnapshot, status: 'error', message: '无法读取本地凭据。' });
    render(<App />);
    expect(await screen.findByText('无法读取用量')).toBeTruthy();
    expect(screen.getByText('无法读取本地凭据。')).toBeTruthy();
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
    await renderLoaded({ ...defaultSettings, theme });
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
  });

  it('announces an automatically detected signed update and opens settings on demand', async () => {
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
    expect(await screen.findByText('发现新版本 v0.2.7，可在设置中下载并验证签名。')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '查看更新' }));
    await waitFor(() => expect(bridgeMocks.openSettingsWindow).toHaveBeenCalledWith('about'));
  });
});
