import { Box, Card, CardContent, LinearProgress, Stack, Tooltip, Typography } from '@mui/material';
import { useTranslation } from 'react-i18next';

import { formatCountdownDuration, formatDuration, formatResetDateTime, resetCountdownSeconds } from './format';
import { resolveSupportedLanguage } from './i18n';
import { hasValidPaceWindow } from './quotaWindowPresentation';
import type { QuotaWindow } from './types';

interface QuotaWindowCardProps {
  quotaWindow: QuotaWindow;
  now: number;
  refreshedAt?: string | null;
}

function paceMarkerPercent(quotaWindow: QuotaWindow, now: number): number | null {
  if (!hasValidPaceWindow(quotaWindow)) return null;
  const start = new Date(quotaWindow.startAt).getTime();
  const reset = new Date(quotaWindow.resetAt).getTime();
  const elapsed = Math.min(Math.max(now - start, 0), reset - start);
  return Math.min(Math.max(100 - (elapsed / (reset - start)) * 100, 0), 100);
}

function quotaColor(remainingPercent: number): 'success' | 'warning' | 'error' {
  if (remainingPercent > 45) return 'success';
  if (remainingPercent > 20) return 'warning';
  return 'error';
}

/** MUI 卡片只呈现已脱敏的动态额度窗口。 */
export function QuotaWindowCard({ quotaWindow, now, refreshedAt = null }: QuotaWindowCardProps) {
  const { t, i18n } = useTranslation();
  const language = resolveSupportedLanguage(i18n.resolvedLanguage ?? i18n.language);
  const marker = paceMarkerPercent(quotaWindow, now);
  const color = quotaColor(quotaWindow.remainingPercent);
  const suggestedRemaining = marker === null ? 0 : Math.round(marker);
  const remainingSeconds = resetCountdownSeconds(
    quotaWindow.resetAt,
    quotaWindow.resetAfterSeconds,
    refreshedAt,
    now,
  );
  const label =
    quotaWindow.label ??
    (quotaWindow.fallbackLabel === 'fiveHour'
      ? t('quota.fallbackLabel.fiveHour')
      : quotaWindow.fallbackLabel === 'weekly'
        ? t('quota.fallbackLabel.weekly')
        : t('quota.fallbackLabel.window', {
            duration: formatDuration(quotaWindow.windowSeconds, language),
          }));
  return (
    <Card
      variant="outlined"
      component="article"
      aria-label={t('quota.cardAria', { label, percent: quotaWindow.remainingPercent })}
    >
      <CardContent sx={{ p: 1.5, '&:last-child': { pb: 1.5 } }}>
        <Stack direction="row" spacing={1} sx={{ justifyContent: 'space-between', alignItems: 'baseline' }}>
          <Typography variant="subtitle2" noWrap sx={{ fontWeight: 700 }}>
            {label}
          </Typography>
          <Typography component="strong" variant="h5" color={`${color}.main`} sx={{ fontWeight: 800 }}>
            {quotaWindow.remainingPercent}%
          </Typography>
        </Stack>

        <Stack direction="row" spacing={1} sx={{ justifyContent: 'space-between', mt: 0.35 }}>
          <Typography variant="caption" color="text.secondary">
            {t('quota.used', { percent: quotaWindow.usedPercent })}
          </Typography>
          <Typography variant="caption" color="text.secondary" noWrap>
            {t('quota.resetAt', { date: formatResetDateTime(quotaWindow.resetAt, language) })}
          </Typography>
        </Stack>

        <Box sx={{ position: 'relative', mt: 1.2 }}>
          <LinearProgress
            aria-label={t('quota.remainingAria', { percent: quotaWindow.remainingPercent })}
            color={color}
            variant="determinate"
            value={quotaWindow.remainingPercent}
            sx={{ height: 8, borderRadius: 999 }}
          />
          {marker !== null && (
            <Tooltip title={t('quota.paceTooltip')} placement="top">
              <Box
                component="span"
                aria-label={t('quota.paceAria', { percent: suggestedRemaining })}
                sx={{
                  position: 'absolute',
                  left: `${marker}%`,
                  top: -3,
                  width: 2,
                  height: 14,
                  borderRadius: 1,
                  bgcolor: 'text.primary',
                  transform: 'translateX(-1px)',
                }}
              />
            </Tooltip>
          )}
        </Box>

        <Stack direction="row" spacing={1} sx={{ justifyContent: 'space-between', mt: 0.9 }}>
          <Typography variant="caption" color="text.secondary">
            {remainingSeconds === null
              ? t('common.unavailable')
              : remainingSeconds === 0
                ? t('quota.resetting')
                : t('quota.remainingDuration', {
                    duration: formatCountdownDuration(remainingSeconds, language),
                  })}
          </Typography>
          {marker !== null && (
            <Typography variant="caption" color="text.secondary" noWrap>
              {t('quota.paceSuggestion', { percent: suggestedRemaining })}
            </Typography>
          )}
        </Stack>
      </CardContent>
    </Card>
  );
}
