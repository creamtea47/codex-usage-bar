import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { QuotaWindowCard } from './QuotaWindowCard';
import { applyLanguagePreference } from './i18n';
import type { QuotaWindow } from './types';

const weeklyWindow: QuotaWindow = {
  id: 'weekly',
  label: '周限额',
  remainingPercent: 72,
  usedPercent: 28,
  windowSeconds: 604800,
  resetAt: '2030-01-08T00:00:00Z',
  resetAfterSeconds: 86_400,
  startAt: '2030-01-01T00:00:00Z',
  showPaceMarker: true,
  fallbackLabel: 'weekly',
  forecast: null,
};

describe('QuotaWindowCard', () => {
  beforeEach(async () => {
    await applyLanguagePreference('zh-CN');
  });

  afterEach(async () => {
    vi.useRealTimers();
    await applyLanguagePreference('zh-CN');
  });

  it('shows a dynamic quota window and its suggested-remaining pace marker', async () => {
    render(
      <QuotaWindowCard
        quotaWindow={weeklyWindow}
        now={Date.parse('2030-01-04T00:00:00Z')}
        refreshedAt="2030-01-04T00:00:00Z"
      />,
    );
    expect(screen.getByText('周限额')).toBeTruthy();
    expect(screen.getByText('72%')).toBeTruthy();
    expect(screen.getByText('建议控量 ≥ 57%')).toBeTruthy();
    const marker = screen.getByLabelText('建议控量不低于 57%');
    fireEvent.mouseOver(marker);
    expect(await screen.findByText('按当前窗口时间进度实时估算的平均消耗节奏，不代表官方阈值')).toBeTruthy();
    expect(screen.getByRole('progressbar', { name: '剩余 72%' }).getAttribute('aria-valuenow')).toBe('72');
  });

  it('does not show a pace marker or reliable forecast for short windows', () => {
    render(
      <QuotaWindowCard
        quotaWindow={{
          ...weeklyWindow,
          id: 'short',
          label: '短周期限额',
          showPaceMarker: false,
          forecast: {
            status: 'lastsUntilReset',
            exhaustsAt: null,
            sampleCount: 6,
            observedSpanSeconds: 3_600,
            consumedPercent: 4,
          },
        }}
        now={Date.now()}
      />,
    );
    expect(screen.queryByText(/建议控量/)).toBeNull();
    expect(screen.queryByText('预计可撑到重置')).toBeNull();
  });

  it('counts down from resetAt, clamps at zero, and shows the resetting state', () => {
    vi.useFakeTimers();
    vi.setSystemTime('2030-01-08T00:00:00Z');
    const resetAt = '2030-01-08T00:02:00Z';
    const quotaWindow = { ...weeklyWindow, resetAt };
    const { rerender } = render(
      <QuotaWindowCard quotaWindow={quotaWindow} now={Date.now()} refreshedAt="2030-01-08T00:00:00Z" />,
    );

    expect(screen.getByText('剩余 2分 0秒')).toBeTruthy();
    vi.advanceTimersByTime(1_000);
    rerender(<QuotaWindowCard quotaWindow={quotaWindow} now={Date.now()} refreshedAt="2030-01-08T00:00:00Z" />);
    expect(screen.getByText('剩余 1分 59秒')).toBeTruthy();

    vi.advanceTimersByTime(119_000);
    rerender(<QuotaWindowCard quotaWindow={quotaWindow} now={Date.now()} refreshedAt="2030-01-08T00:00:00Z" />);
    expect(screen.getByText('正在重置…')).toBeTruthy();
    expect(screen.queryByText(/剩余 -/)).toBeNull();
  });

  it('anchors the relative fallback to refreshedAt when resetAt is missing', () => {
    vi.useFakeTimers();
    vi.setSystemTime('2030-01-08T00:00:00Z');
    const quotaWindow = { ...weeklyWindow, resetAt: null, resetAfterSeconds: 3_600 };
    const { rerender } = render(
      <QuotaWindowCard quotaWindow={quotaWindow} now={Date.now()} refreshedAt="2030-01-08T00:00:00Z" />,
    );

    expect(screen.getByText('剩余 1小时 0分 0秒')).toBeTruthy();
    vi.advanceTimersByTime(1_000);
    rerender(<QuotaWindowCard quotaWindow={quotaWindow} now={Date.now()} refreshedAt="2030-01-08T00:00:00Z" />);
    expect(screen.getByText('剩余 59分 59秒')).toBeTruthy();
  });

  it('localizes fallback labels, reset copy, accessibility labels, and reliable forecasts', async () => {
    await applyLanguagePreference('en');
    render(
      <QuotaWindowCard
        quotaWindow={{
          ...weeklyWindow,
          label: null,
          forecast: {
            status: 'lastsUntilReset',
            exhaustsAt: null,
            sampleCount: 6,
            observedSpanSeconds: 3_600,
            consumedPercent: 4,
          },
        }}
        now={Date.parse('2030-01-04T00:00:00Z')}
        refreshedAt="2030-01-04T00:00:00Z"
      />,
    );

    expect(screen.getByText('Weekly limit')).toBeTruthy();
    expect(screen.getByText('28% used')).toBeTruthy();
    expect(screen.getByText('Likely lasts until reset')).toBeTruthy();
    expect(screen.queryByText(/Suggested ≥/)).toBeNull();
    expect(screen.getByRole('progressbar', { name: '72% remaining' })).toBeTruthy();
    expect(screen.getByRole('article', { name: 'Weekly limit, 72% remaining' })).toBeTruthy();
  });
});
