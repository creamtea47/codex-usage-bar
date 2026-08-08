import { beforeEach, describe, expect, it, vi } from 'vitest';

const tauriMocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: tauriMocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: tauriMocks.listen }));

import { usageBridge } from './bridge';

describe('usageBridge', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauriMocks.invoke.mockResolvedValue(undefined);
  });

  it('maps the fixed settings UI fault code to the Rust camelCase argument', async () => {
    await usageBridge.reportSettingsUiFault('trends-render-failed');

    expect(tauriMocks.invoke).toHaveBeenCalledWith('report_settings_ui_fault', {
      faultCode: 'trends-render-failed',
    });
  });

  it('routes quota auto-continuation through dedicated commands and its status event', async () => {
    const handler = vi.fn();
    tauriMocks.listen.mockResolvedValue(() => undefined);

    await usageBridge.getQuotaAutoContinueStatus();
    await usageBridge.setQuotaAutoContinueEnabled(true);
    await usageBridge.testQuotaAutoContinue();
    await usageBridge.listenForQuotaAutoContinue(handler);

    expect(tauriMocks.invoke).toHaveBeenNthCalledWith(1, 'get_quota_auto_continue_status');
    expect(tauriMocks.invoke).toHaveBeenNthCalledWith(2, 'set_quota_auto_continue_enabled', { enabled: true });
    expect(tauriMocks.invoke).toHaveBeenNthCalledWith(3, 'test_quota_auto_continue');
    expect(tauriMocks.listen).toHaveBeenCalledWith('quota-auto-continue-updated', expect.any(Function));
  });
});
