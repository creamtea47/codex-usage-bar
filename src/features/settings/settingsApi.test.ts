import { settingsApi } from './settingsApi';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const tauriMocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: tauriMocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: tauriMocks.listen }));

describe('settingsApi', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauriMocks.invoke.mockResolvedValue(undefined);
  });

  it('maps the fixed settings UI fault code to the Rust camelCase argument', async () => {
    await settingsApi.reportSettingsUiFault('trends-render-failed');

    expect(tauriMocks.invoke).toHaveBeenCalledWith('report_settings_ui_fault', {
      faultCode: 'trends-render-failed',
    });
  });
  it('maps the default and about settings destinations to the constrained command', async () => {
    await settingsApi.openSettingsWindow();
    await settingsApi.openSettingsWindow('about');

    expect(tauriMocks.invoke).toHaveBeenNthCalledWith(1, 'open_settings_window', { section: undefined });
    expect(tauriMocks.invoke).toHaveBeenNthCalledWith(2, 'open_settings_window', { section: 'about' });
  });
});
