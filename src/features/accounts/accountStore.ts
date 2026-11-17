import { useSyncExternalStore } from 'react';
import { accountsApi } from './accountsApi';
import type { AccountsResponse } from '@/contracts/accounts';
let snapshot: AccountsResponse = { selectedId: null, accounts: [] };
const subscribers = new Set<() => void>();
let revision = 0;
let subscription: { stop?: () => void } | undefined;

export const currentAccountId = () => snapshot.selectedId;
export async function refreshAccounts() {
  const request = ++revision;
  const next = await accountsApi.list();
  if (request !== revision) return;
  if (!next || !Array.isArray(next.accounts)) throw new Error('invalidResponse');
  snapshot = next;
  subscribers.forEach((fn) => fn());
}

function subscribe(fn: () => void) {
  subscribers.add(fn);
  if (subscribers.size === 1) {
    const registration: { stop?: () => void } = {};
    subscription = registration;
    void accountsApi
      .onChange(() => {
        void refreshAccounts().catch(() => {});
      })
      .then((stop) => {
        if (subscription !== registration) {
          stop();
          return;
        }
        registration.stop = stop;
        void refreshAccounts().catch(() => {});
      })
      .catch(() => {});
    void refreshAccounts().catch(() => {});
  }
  return () => {
    subscribers.delete(fn);
    if (subscribers.size === 0) {
      subscription?.stop?.();
      subscription = undefined;
    }
  };
}

export function useAccounts() {
  return useSyncExternalStore(subscribe, () => snapshot);
}
