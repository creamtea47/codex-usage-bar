import type { DashboardSnapshot } from '@/contracts/usage';
export const loadingSnapshot: DashboardSnapshot = {
  status: 'loading',
  accountEmailMasked: null,
  planLabel: null,
  refreshedAt: null,
  nextRefreshAt: null,
  message: null,
  quotaWindows: [],
};
