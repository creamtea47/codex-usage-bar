import { act, renderHook } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { currentAccountId, refreshAccounts, useAccounts } from './accountStore';

const { listeners, invoke } = vi.hoisted(() => ({
  invoke: vi.fn(async () => ({ selectedId: null as string | null, accounts: [] })),
  listeners: [] as Array<(stop: () => void) => void>,
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: () => new Promise<() => void>((resolve) => listeners.push(resolve)),
}));
vi.mock('@tauri-apps/api/core', () => ({
  invoke,
}));

it('releases registrations that finish after their final subscriber leaves', async () => {
  const oldStop = vi.fn();
  const newStop = vi.fn();
  const first = renderHook(useAccounts);
  first.unmount();
  const second = renderHook(useAccounts);
  await act(async () => {
    listeners[1](newStop);
  });
  await act(async () => {
    listeners[0](oldStop);
  });
  second.unmount();
  expect(oldStop).toHaveBeenCalledOnce();
  expect(newStop).toHaveBeenCalledOnce();
});

it('keeps the new account selection when the previous read returns last', async () => {
  let finishOld!: (value: { selectedId: string; accounts: never[] }) => void;
  invoke.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishOld = resolve;
      }),
  );
  const oldRead = refreshAccounts();
  invoke.mockResolvedValueOnce({ selectedId: 'new', accounts: [] });
  await refreshAccounts();
  finishOld({ selectedId: 'old', accounts: [] });
  await oldRead;
  expect(currentAccountId()).toBe('new');
});
