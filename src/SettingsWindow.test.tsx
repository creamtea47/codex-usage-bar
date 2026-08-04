import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import SettingsWindow from './SettingsWindow';
import { defaultSettings, type Settings, type UpdateInfo } from './types';

const bridgeMocks = vi.hoisted(() => ({
  getDashboard: vi.fn(),
  refreshDashboard: vi.fn(),
  getSettings: vi.fn(),
  saveSettings: vi.fn(),
  getAutostart: vi.fn(),
  setAutostart: vi.fn(),
  checkUpdate: vi.fn(),
  openSettingsWindow: vi.fn(),
  listenForDashboard: vi.fn(),
  listenForSettings: vi.fn(),
}));

const openerMocks = vi.hoisted(() => ({ openUrl: vi.fn() }));

vi.mock('./bridge', () => ({ usageBridge: bridgeMocks }));
vi.mock('@tauri-apps/plugin-opener', () => openerMocks);

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

function prepareBridge(settings: Settings = defaultSettings) {
  bridgeMocks.getSettings.mockResolvedValue(settings);
  bridgeMocks.saveSettings.mockImplementation(async (next: Settings) => next);
  bridgeMocks.getAutostart.mockResolvedValue(false);
  bridgeMocks.setAutostart.mockImplementation(async (enabled: boolean) => enabled);
  bridgeMocks.checkUpdate.mockResolvedValue(availableUpdate);
  bridgeMocks.listenForSettings.mockResolvedValue(() => undefined);
}

async function renderLoaded(settings: Settings = defaultSettings) {
  prepareBridge(settings);
  render(<SettingsWindow />);
  await screen.findByRole('heading', { name: '显示' });
}

beforeEach(() => installMatchMedia());

afterEach(() => {
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

describe('SettingsWindow', () => {
  it('provides four real sidebar categories', async () => {
    await renderLoaded();

    expect(screen.getByRole('button', { name: '显示' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '数据与刷新' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '启动' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '关于与更新' })).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: '数据与刷新' }));
    expect(await screen.findByRole('heading', { name: '数据与刷新' })).toBeTruthy();
    expect(screen.getByRole('combobox', { name: '自动刷新' })).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: '启动' }));
    expect(await screen.findByRole('heading', { name: '启动' })).toBeTruthy();
    expect(screen.getByRole('switch', { name: '开机启动' })).toBeTruthy();
  });

  it('persists display settings and reports the result', async () => {
    await renderLoaded();

    fireEvent.click(screen.getByRole('switch', { name: '始终置顶' }));
    await waitFor(() => expect(bridgeMocks.saveSettings).toHaveBeenCalledWith({ ...defaultSettings, alwaysOnTop: true }));
    expect(await screen.findByText('设置已保存。')).toBeTruthy();
  });

  it('checks public releases and opens a release only after user action', async () => {
    await renderLoaded();
    fireEvent.click(screen.getByRole('button', { name: '关于与更新' }));
    fireEvent.click(await screen.findByRole('button', { name: '检查更新' }));

    await waitFor(() => expect(bridgeMocks.checkUpdate).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('发现新版本 v0.2.1')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '打开发布页' }));
    await waitFor(() => expect(openerMocks.openUrl).toHaveBeenCalledWith(availableUpdate.downloadUrl));
  });
});
