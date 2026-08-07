import i18n, { currentLanguage, resolveSupportedLanguage, type SupportedLanguage } from './i18n';

function formatLanguage(locale?: string): SupportedLanguage {
  return locale === undefined ? currentLanguage() : resolveSupportedLanguage(locale);
}

export function formatDuration(totalSeconds: number, locale?: string): string {
  const language = formatLanguage(locale);
  const seconds = Math.max(0, Math.floor(totalSeconds));
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);

  if (days > 0) return i18n.t('duration.daysHours', { lng: language, days, hours });
  if (hours > 0) return i18n.t('duration.hoursMinutes', { lng: language, hours, minutes });
  if (minutes > 0) return i18n.t('duration.minutes', { lng: language, minutes });
  return i18n.t('duration.lessThanMinute', { lng: language });
}

/** Formats a live countdown with seconds so every scheduler tick is visible. */
export function formatCountdownDuration(totalSeconds: number, locale?: string): string {
  const language = formatLanguage(locale);
  const value = Math.max(0, Math.floor(totalSeconds));
  const days = Math.floor(value / 86_400);
  const hours = Math.floor((value % 86_400) / 3_600);
  const minutes = Math.floor((value % 3_600) / 60);
  const seconds = value % 60;

  if (days > 0) {
    return i18n.t('duration.countdownDays', { lng: language, days, hours, minutes, seconds });
  }
  if (hours > 0) {
    return i18n.t('duration.countdownHours', { lng: language, hours, minutes, seconds });
  }
  if (minutes > 0) {
    return i18n.t('duration.countdownMinutes', { lng: language, minutes, seconds });
  }
  return i18n.t('duration.countdownSeconds', { lng: language, seconds });
}

export function formatDateTime(value: string | null, locale?: string): string {
  if (!value) return '—';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '—';
  return new Intl.DateTimeFormat(formatLanguage(locale), {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  }).format(date);
}

/**
 * 将额度重置时间固定为“月/日 时:分 星期”顺序。
 * 这里不复用通用日期布局，避免最小浮窗宽度下因系统语言差异产生换行或顺序漂移。
 */
export function formatResetDateTime(value: string | null, locale?: string): string {
  if (!value) return '—';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '—';

  const language = formatLanguage(locale);
  const weekdays =
    language === 'zh-CN'
      ? ['周日', '周一', '周二', '周三', '周四', '周五', '周六']
      : ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  const hour = String(date.getHours()).padStart(2, '0');
  const minute = String(date.getMinutes()).padStart(2, '0');
  return `${month}/${day} ${hour}:${minute} ${weekdays[date.getDay()]}`;
}

export function formatClock(value: string | null, locale?: string): string {
  if (!value) return '—';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '—';
  return new Intl.DateTimeFormat(formatLanguage(locale), {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  }).format(date);
}

export function statusLabel(status: 'loading' | 'ready' | 'stale' | 'error'): string {
  switch (status) {
    case 'ready':
      return i18n.t('status.ready');
    case 'stale':
      return i18n.t('status.stale');
    case 'error':
      return i18n.t('status.error');
    default:
      return i18n.t('status.loading');
  }
}

/**
 * Computes the local reset countdown without trusting a stale relative value.
 * An absolute reset timestamp wins; relative reset seconds are anchored to the
 * successful snapshot time only when the upstream response has no reset time.
 */
export function resetCountdownSeconds(
  resetAt: string | null,
  resetAfterSeconds: number,
  refreshedAt: string | null,
  now: number,
): number | null {
  if (!Number.isFinite(now)) return null;

  if (resetAt) {
    const resetTime = new Date(resetAt).getTime();
    if (Number.isFinite(resetTime)) return Math.max(0, Math.ceil((resetTime - now) / 1_000));
  }

  if (!Number.isFinite(resetAfterSeconds)) return null;
  const initialSeconds = Math.max(0, Math.floor(resetAfterSeconds));
  if (!refreshedAt) return initialSeconds;

  const refreshedTime = new Date(refreshedAt).getTime();
  if (!Number.isFinite(refreshedTime)) return initialSeconds;
  const elapsedSeconds = Math.max(0, (now - refreshedTime) / 1_000);
  return Math.max(0, Math.ceil(initialSeconds - elapsedSeconds));
}
