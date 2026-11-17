import { invoke, listen } from '@/platform/ipc';
import type { Settings } from '@/contracts/settings';
import type { UsageHistoryRequest, UsageHistoryResponse, AnalyticsRequest } from '@/contracts/history';
export const historyApi = {
  getUsageHistory: (request: UsageHistoryRequest, accountId: string | null) =>
    invoke<UsageHistoryResponse>('get_usage_history', { request, accountId }),
  setHistoryEnabled: (enabled: boolean) => invoke<Settings>('set_history_enabled', { enabled }),
  clearUsageHistory: (accountId: string | null) => invoke<void>('clear_usage_history', { accountId }),
  listenForUsageHistory: (handler: () => void, accountId: string | null) =>
    listen<{ accountId?: string | null }>('usage-history-updated', (event) => {
      if (!event.payload?.accountId || event.payload.accountId === accountId) handler();
    }),
  analytics: <T>(request: AnalyticsRequest, accountId: string | null) =>
    invoke<T>('get_usage_analytics', { accountId, request }),
};
