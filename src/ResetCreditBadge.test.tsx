import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ResetCreditBadge } from './ResetCreditBadge';
import type { ResetCreditDetails } from './accountTypes';
import i18n from './i18n';

const query = vi.hoisted(() => vi.fn());
vi.mock('./accountsBridge', () => ({ accountsBridge: { resetCredits: query } }));
const now = Date.parse('2030-09-23T00:00:00');
const details = (accountId = 'one'): ResetCreditDetails => ({
  accountId, status: 'ready', checkedAt: new Date(now).toISOString(), fetchedAt: new Date(now).toISOString(), errorCode: null,
  credits: [{ expiresAt: '2030-10-23T04:30:58', status: 'available', supportedByPlan: true }],
});
beforeEach(async () => { query.mockReset().mockImplementation(async (id: string) => details(id)); await i18n.changeLanguage('zh-CN'); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); });

it('keeps the held 1 / usable 0 distinction and shows the expiry without inventing a threshold', async () => {
  render(<ResetCreditBadge accountId="one" available={1} applicable={0} now={now} />);
  expect(await screen.findByText('最近到期：2030/10/23 04:30')).toBeTruthy();
  fireEvent.mouseOver(screen.getByText('持有重置卡 1 张').closest('[tabindex]')!);
  const tooltip = await screen.findByRole('tooltip');
  expect(tooltip.textContent).toContain('当前可用 0 张');
  expect(tooltip.textContent).toContain('当前满足使用条件的为 0 张');
  expect(tooltip.textContent).toContain('第 1 张：2030/10/23 04:30:58');
  expect(tooltip.textContent).not.toContain('90%');
});

it('chooses the nearest of several expiry dates and retains cached details on failure with a forced retry', async () => {
  query.mockResolvedValue({ ...details(), credits: [...details().credits,
    { expiresAt: null, status: 'available', supportedByPlan: null },
    { expiresAt: '2030-10-01T06:00:00', status: 'available', supportedByPlan: false },
  ] });
  render(<ResetCreditBadge accountId="one" available={3} applicable={0} now={now} />);
  expect(await screen.findByText('最近到期：2030/10/01 06:00')).toBeTruthy();
  query.mockRejectedValueOnce('offline');
  fireEvent(document, new Event('visibilitychange'));
  expect(await screen.findByText('有效期暂时无法获取')).toBeTruthy();
  fireEvent.mouseOver(screen.getByText('持有重置卡 3 张').closest('[tabindex]')!);
  const tooltip = await screen.findByRole('tooltip');
  expect(tooltip.textContent).toContain('以下为上次获取的明细');
  expect(tooltip.textContent).toContain('第 2 张：未返回有效期');
  expect(tooltip.textContent).toContain('当前套餐不支持这张卡');
  fireEvent.click(screen.getByRole('button', { name: '重试重置卡详情' }));
  await waitFor(() => expect(query).toHaveBeenLastCalledWith('one', true));
  expect(await screen.findByText('最近到期：2030/10/01 06:00')).toBeTruthy();
});

it('does not let a delayed old-account response replace the current account and handles missing dates', async () => {
  let finish!: (value: ResetCreditDetails) => void;
  query.mockImplementationOnce(() => new Promise<ResetCreditDetails>((resolve) => { finish = resolve; }));
  const view = render(<ResetCreditBadge accountId="one" available={1} applicable={0} now={now} />);
  query.mockResolvedValueOnce({ ...details('two'), credits: [{ expiresAt: null, status: 'available', supportedByPlan: null }] });
  view.rerender(<ResetCreditBadge accountId="two" available={1} applicable={1} now={now} />);
  expect(await screen.findByText('未返回有效期')).toBeTruthy();
  await act(async () => finish(details()));
  expect(screen.queryByText(/最近到期/)).toBeNull();
  expect(screen.getByText('未返回有效期')).toBeTruthy();
});

it('checks the backend cache while visible, skips empty accounts and refreshes after count changes', async () => {
  vi.useFakeTimers();
  const view = render(<ResetCreditBadge accountId="one" available={0} applicable={0} now={now} />);
  expect(query).not.toHaveBeenCalled();
  view.rerender(<ResetCreditBadge accountId="one" available={1} applicable={0} now={now} />);
  await act(async () => {});
  expect(query).toHaveBeenCalledTimes(1);
  const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
  await act(async () => vi.advanceTimersByTime(60_000));
  expect(query).toHaveBeenCalledTimes(1);
  visibility.mockReturnValue('visible');
  await act(async () => vi.advanceTimersByTime(60_000));
  expect(query).toHaveBeenLastCalledWith('one', false);
  expect(query).toHaveBeenCalledTimes(2);
  view.rerender(<ResetCreditBadge accountId="one" available={2} applicable={0} now={now} />);
  await act(async () => {});
  expect(query).toHaveBeenCalledTimes(3);
});
