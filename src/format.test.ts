import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  formatClock,
  formatCountdownDuration,
  formatDateTime,
  formatDuration,
  resetCountdownSeconds,
  statusLabel,
} from './format';
import { applyLanguagePreference, resolveLanguagePreference, resolveSupportedLanguage } from './i18n';

describe('language resolution and localized formatting', () => {
  afterEach(async () => {
    await applyLanguagePreference('zh-CN');
    vi.useRealTimers();
  });

  it('maps any zh locale to Simplified Chinese and all other locales to English', () => {
    expect(resolveSupportedLanguage('zh')).toBe('zh-CN');
    expect(resolveSupportedLanguage('zh-Hant-HK')).toBe('zh-CN');
    expect(resolveLanguagePreference('system', 'zh-TW')).toBe('zh-CN');
    expect(resolveLanguagePreference('system', 'ja-JP')).toBe('en');
    expect(resolveLanguagePreference('en', 'zh-CN')).toBe('en');
  });

  it('formats durations, dates, clocks, and statuses in the selected language', async () => {
    await applyLanguagePreference('en');
    expect(formatDuration(90)).toBe('1m');
    expect(formatCountdownDuration(90)).toBe('1m 30s');
    expect(formatCountdownDuration(90_061)).toBe('1d 1h 1m 1s');
    expect(statusLabel('ready')).toBe('Usage synced');
    expect(formatDateTime('not-a-date')).toBe('—');
    expect(formatClock(null)).toBe('—');

    await applyLanguagePreference('zh-CN');
    expect(formatDuration(90)).toBe('1分');
    expect(formatCountdownDuration(90)).toBe('1分 30秒');
    expect(formatCountdownDuration(3_661)).toBe('1小时 1分 1秒');
    expect(statusLabel('ready')).toBe('额度已同步');
  });
});

describe('resetCountdownSeconds', () => {
  it('uses a valid absolute reset time before the relative fallback', () => {
    const now = Date.parse('2030-01-01T00:00:00Z');
    expect(resetCountdownSeconds('2030-01-01T00:10:00Z', 60, '2030-01-01T00:00:00Z', now)).toBe(600);
  });

  it('uses resetAfterSeconds anchored to refreshedAt when resetAt is missing or invalid', () => {
    const now = Date.parse('2030-01-01T00:00:30Z');
    expect(resetCountdownSeconds(null, 120, '2030-01-01T00:00:00Z', now)).toBe(90);
    expect(resetCountdownSeconds('invalid', 120, '2030-01-01T00:00:00Z', now)).toBe(90);
  });

  it('does not count upward if the system clock moves backward', () => {
    const now = Date.parse('2029-12-31T23:59:00Z');
    expect(resetCountdownSeconds(null, 120, '2030-01-01T00:00:00Z', now)).toBe(120);
  });

  it('handles timezone offsets, invalid values, and elapsed reset windows safely', () => {
    const sameInstant = Date.parse('2030-01-01T00:00:00Z');
    expect(resetCountdownSeconds('2030-01-01T08:01:00+08:00', 5, null, sameInstant)).toBe(60);
    expect(resetCountdownSeconds(null, Number.NaN, null, sameInstant)).toBeNull();
    expect(resetCountdownSeconds(null, 60, 'invalid', sameInstant)).toBe(60);
    expect(resetCountdownSeconds('2029-12-31T23:59:00Z', 60, null, sameInstant)).toBe(0);
    expect(resetCountdownSeconds(null, 60, '2029-12-31T23:58:00Z', sameInstant)).toBe(0);
  });
});
