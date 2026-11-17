import { continuationApi } from './continuationApi';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const tauriMocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: tauriMocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: tauriMocks.listen }));

describe('continuationApi', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauriMocks.invoke.mockResolvedValue(undefined);
  });

  it('routes quota auto-continuation through dedicated commands and its status event', async () => {
    const handler = vi.fn();
    tauriMocks.listen.mockResolvedValue(() => undefined);

    await continuationApi.getQuotaAutoContinueStatus(null);
    await continuationApi.setQuotaAutoContinueEnabled(true, null);
    await continuationApi.testQuotaAutoContinue(null);
    await continuationApi.listenForQuotaAutoContinue(handler, null);

    expect(tauriMocks.invoke).toHaveBeenNthCalledWith(1, 'get_quota_auto_continue_status', {
      accountId: null,
    });
    expect(tauriMocks.invoke).toHaveBeenNthCalledWith(2, 'set_quota_auto_continue_enabled', {
      enabled: true,
      accountId: null,
    });
    expect(tauriMocks.invoke).toHaveBeenNthCalledWith(3, 'test_quota_auto_continue', { accountId: null });
    expect(tauriMocks.listen).toHaveBeenCalledWith('quota-auto-continue-updated', expect.any(Function));
  });
});
