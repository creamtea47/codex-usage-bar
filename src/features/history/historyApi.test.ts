import { historyApi } from './historyApi';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const tauriMocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: tauriMocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: tauriMocks.listen }));

describe('historyApi', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauriMocks.invoke.mockResolvedValue(undefined);
  });

  it('passes preset and custom history requests through the structured IPC argument', async () => {
    const presetRequest = { kind: 'preset', preset: '30d' } as const;
    const customRequest = {
      kind: 'custom',
      startAt: '2030-03-09T08:00:00.000Z',
      endAtExclusive: '2030-03-11T07:00:00.000Z',
    } as const;

    await historyApi.getUsageHistory(presetRequest, null);
    await historyApi.getUsageHistory(customRequest, null);

    expect(tauriMocks.invoke).toHaveBeenNthCalledWith(1, 'get_usage_history', {
      request: presetRequest,
      accountId: null,
    });
    expect(tauriMocks.invoke).toHaveBeenNthCalledWith(2, 'get_usage_history', {
      request: customRequest,
      accountId: null,
    });
  });
});
