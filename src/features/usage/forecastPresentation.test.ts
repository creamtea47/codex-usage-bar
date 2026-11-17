import { expect, it } from 'vitest';
import { forecastText, minuteDuration } from './forecastPresentation';
import type { QuotaForecast } from '@/contracts/usage';

it('uses reset minus exhaustion and minute precision in both languages', () => {
  const now = Date.parse('2030-01-01T00:00:00Z');
  const forecast: QuotaForecast = {
    status: 'exhaustsBeforeReset',
    exhaustsAt: '2030-01-01T12:00:00Z',
    sampleCount: 8,
    observedSpanSeconds: 3600,
    consumedPercent: 12,
  };
  expect(forecastText(forecast, '2030-01-04T21:31:00Z', now, 'zh-CN')).toContain(
    '约 12小时后，比重置提前 3天9小时31分',
  );
  expect(forecastText(forecast, '2030-01-04T21:31:00Z', now, 'en')).toContain('3d 9h 31m before reset');
  expect(minuteDuration(59, 'en')).toBe('0m');
  expect(forecastText({ ...forecast, status: 'collecting' }, null, now, 'en')).toBe('Insufficient samples');
});
