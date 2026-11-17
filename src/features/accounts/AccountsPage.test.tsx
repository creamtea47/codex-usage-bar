import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import AccountsPage from './AccountsPage';
import { AccountCard } from './AccountCard';
import { AccountSelector, AccountSwitcher } from './AccountSelector';
import i18n from '@/shared/i18n/i18n';
import type { Account, AccountsResponse } from '@/contracts/accounts';
import { loadingSnapshot } from '@/features/usage/loadingSnapshot';

const mocks = vi.hoisted(() => ({
  data: null as AccountsResponse | null,
  update: vi.fn(),
  select: vi.fn(),
  refresh: vi.fn(),
  copy: vi.fn(),
  apply: vi.fn(),
  remove: vi.fn(),
  resetCredits: vi.fn(),
  login: vi.fn(),
  cancelLogin: vi.fn(),
}));
vi.mock('./accountStore', () => ({ useAccounts: () => mocks.data, refreshAccounts: mocks.refresh }));
vi.mock('./accountError', () => ({ accountError: (code: unknown) => String(code) }));
vi.mock('@/features/usage/usageApi', () => ({ usageApi: { resetCredits: mocks.resetCredits } }));
vi.mock('./accountActions', () => ({
  accountActions: {
    login: mocks.login,
    cancelLogin: mocks.cancelLogin,
    onLoginFinished: vi.fn().mockResolvedValue(() => {}),
    loginPending: vi.fn().mockResolvedValue(false),
    update: mocks.update,
    select: mocks.select,
    apply: mocks.apply,
    remove: mocks.remove,
  },
}));
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({ writeText: mocks.copy }));

const now = Date.parse('2030-01-01T00:00:00Z');
const account: Account = {
  id: 'local-key',
  label: 'My work account',
  enabled: true,
  autoContinue: false,
  model: null,
  boundToCodex: true,
  authStatus: 'ready',
  expiresAt: '2030-01-02T00:00:00Z',
  canRefresh: true,
  dashboard: {
    ...loadingSnapshot,
    status: 'ready',
    accountEmailMasked: 'l***@example.com',
    planLabel: 'plus',
    refreshedAt: '2030-01-01T00:00:00Z',
    quotaWindows: [
      {
        id: 'week',
        label: null,
        fallbackLabel: 'weekly',
        windowSeconds: 604800,
        remainingPercent: 72,
        usedPercent: 28,
        resetAt: '2030-01-05T00:00:00Z',
        resetAfterSeconds: 345600,
        startAt: '2029-12-29T00:00:00Z',
        showPaceMarker: true,
        forecast: null,
      },
    ],
  },
  details: {
    email: 'long-identifiable-email@example.com',
    name: 'Example User',
    loginProvider: 'google',
    userId: 'full-user-identifier-1234567890',
    accountId: 'full-account-identifier-1234567890',
    planType: 'plus',
    identitySource: 'usage',
    planSource: 'usage',
    credentialObservedAt: '2029-12-31T00:00:00Z',
    usageObservedAt: '2030-01-01T00:00:00Z',
    subscriptionStartedAt: '2029-12-20T00:00:00Z',
    subscriptionEndsAt: '2030-01-20T00:00:00Z',
    subscriptionCheckedAt: '2029-12-31T00:00:00Z',
    managedAuthPath: 'C:\\qa\\accounts\\one\\auth.json',
    resetCredits: { available: 3, applicable: 1 },
    extraCredits: { hasCredits: true, unlimited: false, balance: '5' },
    models: [{ id: 'some-model', available: true, availableAt: null }],
  },
};

describe('account cards', () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    mocks.data = {
      selectedId: account.id,
      accounts: [structuredClone(account)],
      codexLogin: {
        path: 'C:\\qa\\codex\\auth.json',
        storage: 'file',
        status: 'matched',
        matchedAccountId: account.id,
        email: account.details!.email,
        checkedAt: '2030-01-01T00:00:00Z',
        bindingMatches: true,
      },
    };
    for (const mock of [mocks.update, mocks.select, mocks.refresh, mocks.copy, mocks.apply, mocks.remove])
      mock.mockResolvedValue(undefined);
    mocks.resetCredits.mockImplementation(async (accountId: string) => ({
      accountId,
      status: 'ready',
      credits: [],
      fetchedAt: new Date(now).toISOString(),
      checkedAt: new Date(now).toISOString(),
      errorCode: null,
    }));
    await i18n.changeLanguage('en');
  });
  afterEach(cleanup);

  it('imports after browser sign-in and can cancel a pending sign-in', async () => {
    mocks.login.mockResolvedValueOnce(undefined);
    render(<AccountsPage />);
    const signIn = () => screen.getByRole('button', { name: 'Sign in and import' });
    fireEvent.click(signIn());
    expect(await screen.findByText('Account imported.')).toBeTruthy();

    let cancel: () => void = () => {};
    mocks.login.mockImplementationOnce(
      () =>
        new Promise<void>((_, reject) => {
          cancel = () => reject('loginCancelled');
        }),
    );
    mocks.cancelLogin.mockImplementationOnce(async () => cancel());
    fireEvent.click(signIn());
    expect((screen.getByRole('button', { name: 'Import from files' }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Cancel sign-in' }));
    await waitFor(() => expect(signIn()).toBeTruthy());
    expect(mocks.cancelLogin).toHaveBeenCalledOnce();
    expect(mocks.apply).not.toHaveBeenCalled();
  });

  it('makes email the identity, shows quota and subscription, and edits notes only on demand', async () => {
    render(<AccountCard account={account} selected codexMatched now={now} />);
    expect(screen.getByText(account.details!.email!)).toBeTruthy();
    expect(screen.getByText('Plus')).toBeTruthy();
    expect(screen.getByText('Viewing')).toBeTruthy();
    expect(screen.getByText('Codex login file')).toBeTruthy();
    expect(screen.getByRole('progressbar', { name: 'Weekly limit: 72%' })).toBeTruthy();
    expect(screen.getByText(/19 days remaining/)).toBeTruthy();
    expect(screen.queryByRole('textbox')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Edit note' }));
    fireEvent.change(screen.getByRole('textbox', { name: 'Account note' }), {
      target: { value: 'New note' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() =>
      expect(mocks.update).toHaveBeenCalledWith(
        expect.objectContaining({ id: account.id, label: 'New note' }),
      ),
    );
    await waitFor(() => expect(screen.queryByRole('textbox')).toBeNull());
  });

  it('keeps token expiry separate and treats an old subscription date as unverified', async () => {
    const old = {
      ...account,
      dashboard: { ...account.dashboard, status: 'stale' as const },
      details: { ...account.details!, subscriptionEndsAt: '2029-12-20T00:00:00Z' },
    };
    render(<AccountCard account={old} selected={false} codexMatched now={now} />);
    expect(screen.getByText(/Needs verification/)).toBeTruthy();
    expect(screen.getByText('Last successful data')).toBeTruthy();
    expect(screen.queryByText('Access token expires')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Account details' }));
    expect(await screen.findByText('Access token expires')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Copy User ID' }));
    await waitFor(() => expect(mocks.copy).toHaveBeenCalledWith(account.details!.userId));
    expect(screen.getByText('Google')).toBeTruthy();
    expect(screen.queryByText('Viewing')).toBeNull();
  });

  it('renders sparse accounts without fabricated subscription or identity data', async () => {
    const sparse = {
      ...account,
      details: undefined,
      label: 'Legacy note',
      dashboard: { ...loadingSnapshot },
    };
    render(<AccountCard account={sparse} selected={false} codexMatched={false} now={now} />);
    expect(screen.getByRole('article', { name: 'Legacy note' })).toBeTruthy();
    expect(screen.queryByText(/Subscription through/)).toBeNull();
    expect(screen.queryByText('Plus')).toBeNull();
  });

  it('distinguishes the viewed account from the file match and rechecks on page entry', async () => {
    mocks.data!.selectedId = 'other';
    render(<AccountsPage />);
    await waitFor(() => expect(mocks.refresh).toHaveBeenCalled());
    expect(screen.getByText('C:\\qa\\codex\\auth.json')).toBeTruthy();
    const card = screen.getByRole('article', { name: account.details!.email! });
    expect(within(card).queryByText('Viewing')).toBeNull();
    expect(within(card).getByText('Codex login file')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Copy Codex auth.json' }));
    await waitFor(() => expect(mocks.copy).toHaveBeenCalledWith('C:\\qa\\codex\\auth.json'));
  });

  it('uses full identity in sidebar but keeps main selector on its previous label', async () => {
    const view = render(<AccountSelector sidebar />);
    expect(screen.getByRole('combobox').textContent).toContain(account.details!.email);
    view.unmount();
    render(<AccountSelector />);
    expect(screen.getByRole('combobox').textContent).toBe('My work account');
    await act(async () => {});
  });

  it('switches the viewed account through a masked menu and reports failures without changing selection', async () => {
    mocks.data!.accounts.push({
      ...account,
      id: 'other',
      label: 'Second note',
      dashboard: { ...account.dashboard, accountEmailMasked: 's***@example.com' },
    });
    render(<AccountSwitcher />);
    const trigger = screen.getByRole('button', { name: 'Switch viewed account' });
    fireEvent.click(trigger);
    expect(
      screen.getByRole('menuitemradio', { name: /l\*\*\*@example.com/ }).getAttribute('aria-checked'),
    ).toBe('true');
    expect(screen.queryByText(account.details!.email!)).toBeNull();
    fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
    fireEvent.click(trigger);
    mocks.select.mockRejectedValueOnce('network');
    fireEvent.click(screen.getByRole('menuitemradio', { name: /s\*\*\*@example.com/ }));
    expect(await screen.findByRole('alert')).toHaveProperty('textContent', 'network');
    expect(mocks.data!.selectedId).toBe(account.id);
    fireEvent.click(trigger);
    fireEvent.click(screen.getByRole('menuitemradio', { name: /s\*\*\*@example.com/ }));
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
    expect(mocks.select).toHaveBeenLastCalledWith('other');
    expect(mocks.apply).not.toHaveBeenCalled();
  });
});
