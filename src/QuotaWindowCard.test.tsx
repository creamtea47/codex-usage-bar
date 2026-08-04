import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { QuotaWindowCard } from './QuotaWindowCard';
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
};

describe('QuotaWindowCard', () => {
  it('shows a dynamic quota window and its pace marker', () => {
    render(<QuotaWindowCard quotaWindow={weeklyWindow} now={Date.parse('2030-01-04T00:00:00Z')} />);
    expect(screen.getByText('周限额')).toBeTruthy();
    expect(screen.getByText('72%')).toBeTruthy();
    expect(screen.getByText('黑线为平均进度')).toBeTruthy();
    expect(screen.getByRole('progressbar', { name: '剩余 72%' }).getAttribute('aria-valuenow')).toBe('72');
  });

  it('does not show a pace marker for short windows', () => {
    render(
      <QuotaWindowCard
        quotaWindow={{ ...weeklyWindow, id: 'short', label: '短周期限额', showPaceMarker: false }}
        now={Date.now()}
      />,
    );
    expect(screen.queryByText('黑线为平均进度')).toBeNull();
  });
});
