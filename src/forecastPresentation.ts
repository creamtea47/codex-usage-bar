import { useEffect, useState } from 'react';
import i18n, { resolveSupportedLanguage } from './i18n';
import { formatDateTime } from './format';
import type { QuotaForecast } from './types';

/** 预测仅精确到分钟，不能用秒级倒计时暗示超过采样的精度。 */
export function minuteDuration(seconds: number, language: string): string {
  const total = Math.max(0, Math.floor(seconds / 60));
  const d = Math.floor(total / 1440), h = Math.floor(total % 1440 / 60), m = total % 60;
  return resolveSupportedLanguage(language) === 'zh-CN'
    ? `${d ? `${d}天` : ''}${h ? `${h}小时` : ''}${m || (!d && !h) ? `${m}分` : ''}`
    : `${d ? `${d}d ` : ''}${h ? `${h}h ` : ''}${m || (!d && !h) ? `${m}m` : ''}`.trim();
}

export function forecastText(forecast: QuotaForecast | null, resetAt: string | null, now: number, language: string): string {
  const lng = resolveSupportedLanguage(language);
  if (!forecast || forecast.status === 'collecting') return i18n.t('feature.collecting', { lng });
  if (forecast.status === 'stable') return i18n.t('feature.stable', { lng });
  if (forecast.status === 'lastsUntilReset') return i18n.t('feature.lasts', { lng });
  const empty = Date.parse(forecast.exhaustsAt ?? ''), reset = Date.parse(resetAt ?? '');
  if (!Number.isFinite(empty) || !Number.isFinite(reset)) return i18n.t('feature.collecting', { lng });
  return i18n.t(empty < now ? 'feature.predictionPast' : 'feature.prediction', {
    lng, date: formatDateTime(forecast.exhaustsAt, lng), remaining: minuteDuration((empty - now) / 1000, lng), early: minuteDuration((reset - empty) / 1000, lng),
  });
}

export function useForecastClock() {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);
  return now;
}
