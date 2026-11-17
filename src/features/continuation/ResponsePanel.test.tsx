import { beforeEach, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { ResponsePanel } from './ResponsePanel';
import i18n from '@/shared/i18n/i18n';

beforeEach(async () => {
  await i18n.changeLanguage('en');
});
it('shows response evidence as text without interpreting model markup', () => {
  const { container } = render(
    <ResponsePanel
      automatic={false}
      result={{
        attemptedAt: null,
        successAt: null,
        errorCode: 'network',
        model: 'requested',
        slotIndex: null,
        details: {
          startedAt: null,
          responseModel: 'reported',
          httpStatus: 200,
          durationMs: 85,
          text: '<script>unsafe()</script>',
          truncated: true,
          errorMessage: 'Disconnected',
          deliveryUncertain: true,
        },
      }}
    />,
  );
  expect(screen.getByText('<script>unsafe()</script>')).toBeTruthy();
  expect(container.querySelector('script')).toBeNull();
  expect(screen.getByText('Delivery is uncertain. Automatic retry stopped.')).toBeTruthy();
  expect(screen.getByText(/Response model: reported/)).toBeTruthy();
  expect(screen.getByText('Text was truncated.')).toBeTruthy();
});
