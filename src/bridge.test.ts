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
});
