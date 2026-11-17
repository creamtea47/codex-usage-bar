import { invoke, listen } from '@/platform/ipc';
import type { DashboardSnapshot, ResetCreditDetails } from '@/contracts/usage';
export const usageApi = {
  getDashboard: (accountId: string | null) => invoke<DashboardSnapshot>('get_dashboard', { accountId }),
  refreshDashboard: (accountId: string | null) =>
    invoke<DashboardSnapshot>('refresh_dashboard', { accountId }),
  listenForDashboard: (handler: (snapshot: DashboardSnapshot) => void) =>
    listen<DashboardSnapshot>('dashboard-updated', (event) => handler(event.payload)),
  resetCredits: (accountId: string, force = false) =>
    invoke<ResetCreditDetails>('get_reset_credit_details', { accountId, force }),
};
