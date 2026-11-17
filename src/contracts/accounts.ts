import type { DashboardSnapshot, ExtraCredits } from './usage';
export interface AccountConfig {
  id: string;
  label: string;
  enabled: boolean;
  autoContinue: boolean;
  model: string | null;
}
export interface Account extends AccountConfig {
  boundToCodex: boolean;
  authStatus: string;
  expiresAt: string | null;
  canRefresh: boolean;
  dashboard: DashboardSnapshot;
  details?: AccountDetails;
}
export interface AccountsResponse {
  selectedId: string | null;
  accounts: Account[];
  codexLogin?: CodexLoginSource;
}
/** 完整资料仅由设置窗口 IPC 提供；主悬浮窗继续使用既有脱敏字段。 */
export interface AccountDetails {
  email: string | null;
  name: string | null;
  loginProvider: string | null;
  userId: string | null;
  accountId: string | null;
  planType: string | null;
  identitySource: 'usage' | 'credentials';
  planSource: 'usage' | 'credentials';
  credentialObservedAt: string | null;
  usageObservedAt: string | null;
  subscriptionStartedAt: string | null;
  subscriptionEndsAt: string | null;
  subscriptionCheckedAt: string | null;
  managedAuthPath: string;
  resetCredits: { available: number; applicable: number | null } | null;
  extraCredits: ExtraCredits | null;
  models: { id: string; available: boolean; availableAt: string | null }[];
}
export interface CodexLoginSource {
  path: string | null;
  storage: string | null;
  status: 'matched' | 'unmanaged' | 'missing' | 'invalid' | 'unsupportedStore' | 'unavailable';
  matchedAccountId: string | null;
  email: string | null;
  checkedAt: string;
  bindingMatches: boolean | null;
}
export interface ModelOption {
  id: string;
  label: string;
}
