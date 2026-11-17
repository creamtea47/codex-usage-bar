import { invoke, listen } from '@/platform/ipc';
import type { AccountConfig, AccountsResponse, ModelOption } from '@/contracts/accounts';

export const accountsApi = {
  list: () => invoke<AccountsResponse>('get_accounts'),
  onChange: (handler: () => void) => listen('accounts-updated', handler),
  select: (accountId: string) => invoke('select_account', { accountId }),
  login: () => invoke<void>('login_and_import'),
  cancelLogin: () => invoke<void>('cancel_account_login'),
  loginPending: () => invoke<boolean>('account_login_pending'),
  onLoginFinished: (handler: () => void) => listen('account-login-finished', handler),
  import: () => invoke<{ imported: number; errors: string[] }>('import_accounts'),
  update: ({ id, label, enabled, autoContinue, model }: AccountConfig) =>
    invoke('update_account', { config: { id, label, enabled, autoContinue, model } }),
  remove: (accountId: string) => invoke('remove_account', { accountId }),
  apply: (accountId: string) => invoke('apply_codex_account', { accountId }),
  restore: () => invoke('restore_codex_account'),
  models: (accountId: string) => invoke<ModelOption[]>('get_account_models', { accountId }),
};
