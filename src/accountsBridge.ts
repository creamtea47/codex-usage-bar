import i18n from './i18n';
import { featureEn } from './featureStrings';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useSyncExternalStore } from 'react';
import type { AccountConfig, AccountsResponse, AnalyticsRequest, ModelOption, ResetCreditDetails } from './accountTypes';

let snapshot: AccountsResponse = { selectedId: null, accounts: [] };
const subscribers = new Set<() => void>();
let revision = 0;
let disconnect: (() => void) | undefined;

export const currentAccountId = () => snapshot.selectedId;
export async function refreshAccounts() {
  const request = ++revision;
  const next = await invoke<AccountsResponse>('get_accounts');
  if (request !== revision) return;
  if (!next || !Array.isArray(next.accounts)) throw new Error("invalidResponse");
  snapshot = next;
  subscribers.forEach((fn) => fn());
}

function subscribe(fn: () => void) {
  subscribers.add(fn);
  if (subscribers.size === 1) {
    void listen('accounts-updated', () => { void refreshAccounts().catch(() => {}); }).then((stop) => {
      if (!subscribers.size) { stop(); return; }
      disconnect = stop;
      void refreshAccounts().catch(() => {});
    }).catch(() => {});
    void refreshAccounts().catch(() => {});
  }
  return () => { subscribers.delete(fn); if (!subscribers.size) { disconnect?.(); disconnect = undefined; } };
}

export function useAccounts() { return useSyncExternalStore(subscribe, () => snapshot); }

export const accountsBridge = {
  resetCredits: (accountId: string, force = false) => invoke<ResetCreditDetails>('get_reset_credit_details', { accountId, force }),
  async select(accountId: string) {
    await invoke('select_account', { accountId });
    await refreshAccounts();
  },
  async import() { const result = await invoke<{ imported: number; errors: string[] }>('import_accounts'); await refreshAccounts(); return result; },
  async update({ id, label, enabled, autoContinue, model }: AccountConfig) {
    // 只发送可编辑字段，不能把账号详情或快照跟随对象展开回传。
    await invoke('update_account', { config: { id, label, enabled, autoContinue, model } });
    await refreshAccounts();
  },
  async remove(accountId: string) { await invoke('remove_account', { accountId }); await refreshAccounts(); },
  async apply(accountId: string) { await invoke('apply_codex_account', { accountId }); await refreshAccounts(); },
  async restore() { await invoke('restore_codex_account'); await refreshAccounts(); },
  models: (accountId: string) => invoke<ModelOption[]>('get_account_models', { accountId }),
  analytics: <T,>(request: AnalyticsRequest, accountId = currentAccountId()) => invoke<T>('get_usage_analytics', { accountId, request }),
};

/** 错误正文只允许固定错误码，不回显第三方异常。 */
export function accountError(error: unknown): string {
  const code = typeof error === 'string' ? error : '';
  return Object.hasOwn(featureEn.errors, code)
    ? i18n.t(`feature.errors.${code as keyof typeof featureEn.errors}`)
    : i18n.t('feature.failed');
}
