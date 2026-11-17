import type { AccountConfig } from '@/contracts/accounts';
import { refreshAccounts } from './accountStore';
import { accountsApi } from './accountsApi';

async function reloadAfter<T>(operation: Promise<T>) {
  const value = await operation;
  await refreshAccounts();
  return value;
}

export const accountActions = {
  ...accountsApi,
  select: (id: string) => reloadAfter(accountsApi.select(id)),
  import: () => reloadAfter(accountsApi.import()),
  update: (config: AccountConfig) => reloadAfter(accountsApi.update(config)),
  remove: (id: string) => reloadAfter(accountsApi.remove(id)),
  apply: (id: string) => reloadAfter(accountsApi.apply(id)),
  restore: () => reloadAfter(accountsApi.restore()),
};
