import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import SettingsWindow from './SettingsWindow';
import { defaultSettings, type AppUpdateInfo, type Settings } from './types';

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
  listenForDashboard: vi.fn(),
  listenForSettings: vi.fn(),
  listenForAppUpdate: vi.fn(),
  listenForAppUpdateProgress: vi.fn(),
  listenForSettingsNavigation: vi.fn(),
}));

const openerMocks = vi.hoisted(() => ({ openUrl: vi.fn() }));

vi.mock('./bridge', () => ({ usageBridge: bridgeMocks }));
vi.mock('@tauri-apps/plugin-opener', () => openerMocks);

const availableUpdate: AppUpdateInfo = {
  currentVersion: '0.2.5',
  latestVersion: '0.2.6',
  updateAvailable: true,
  checkedAt: '2030-01-04T12:00:00Z',
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
  bridgeMocks.getAppUpdateInfo.mockResolvedValue({ ...availableUpdate, updateAvailable: false, latestVersion: '0.2.5' });
  bridgeMocks.checkAppUpdate.mockResolvedValue(availableUpdate);
  bridgeMocks.installAppUpdate.mockResolvedValue(undefined);
  bridgeMocks.listenForSettings.mockResolvedValue(() => undefined);
  bridgeMocks.listenForAppUpdate.mockResolvedValue(() => undefined);
  bridgeMocks.listenForAppUpdateProgress.mockResolvedValue(() => undefined);
  bridgeMocks.listenForSettingsNavigation.mockResolvedValue(() => undefined);
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

  it('persists the optional automatic update check preference', async () => {
    await renderLoaded();
    fireEvent.click(screen.getByRole('button', { name: '关于与更新' }));
    fireEvent.click(await screen.findByRole('switch', { name: '自动检查更新' }));

    await waitFor(() => expect(bridgeMocks.saveSettings).toHaveBeenCalledWith({ ...defaultSettings, autoCheckUpdates: false }));
    expect(await screen.findByText('设置已保存。')).toBeTruthy();
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

  it('shows the safe signature-verification failure without exposing transport details', async () => {
    await renderLoaded();
    bridgeMocks.checkAppUpdate.mockRejectedValueOnce(new Error('更新包的签名验证失败，已取消安装。 https://private.invalid/detail'));
    fireEvent.click(screen.getByRole('button', { name: '关于与更新' }));
    fireEvent.click(await screen.findByRole('button', { name: '检查更新' }));

    expect(await screen.findByText('更新包的签名验证失败，已取消安装。')).toBeTruthy();
    expect(screen.queryByText(/private\.invalid/)).toBeNull();
  });
});
