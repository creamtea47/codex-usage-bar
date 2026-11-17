import { invoke, listen } from '@/platform/ipc';
import type { QuotaAutoContinueStatus } from '@/contracts/continuation';
export const continuationApi = {
  getQuotaAutoContinueStatus: (accountId: string | null) =>
    invoke<QuotaAutoContinueStatus>('get_quota_auto_continue_status', { accountId }),
  setQuotaAutoContinueEnabled: (enabled: boolean, accountId: string | null) =>
    invoke<QuotaAutoContinueStatus>('set_quota_auto_continue_enabled', { enabled, accountId }),
  testQuotaAutoContinue: (accountId: string | null) =>
    invoke<QuotaAutoContinueStatus>('test_quota_auto_continue', { accountId }),
  listenForQuotaAutoContinue: (
    handler: (status: QuotaAutoContinueStatus) => void,
    accountId: string | null,
  ) =>
    listen<QuotaAutoContinueStatus>('quota-auto-continue-updated', (event) => {
      if (!event.payload.accountId || event.payload.accountId === accountId) handler(event.payload);
    }),
};
