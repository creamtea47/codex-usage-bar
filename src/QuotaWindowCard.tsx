import { Box, Card, CardContent, LinearProgress, Stack, Tooltip, Typography } from '@mui/material';

import { formatDateTime, formatDuration } from './format';
import type { QuotaWindow } from './types';

interface QuotaWindowCardProps {
  quotaWindow: QuotaWindow;
  now: number;
}

function paceMarkerPercent(quotaWindow: QuotaWindow, now: number): number | null {
  if (!quotaWindow.showPaceMarker || !quotaWindow.startAt || !quotaWindow.resetAt) return null;
  const start = new Date(quotaWindow.startAt).getTime();
  const reset = new Date(quotaWindow.resetAt).getTime();
  if (Number.isNaN(start) || Number.isNaN(reset) || reset <= start) return null;
  const elapsed = Math.min(Math.max(now - start, 0), reset - start);
  return Math.min(Math.max(100 - (elapsed / (reset - start)) * 100, 0), 100);
}

function quotaColor(remainingPercent: number): 'success' | 'warning' | 'error' {
  if (remainingPercent > 45) return 'success';
  if (remainingPercent > 20) return 'warning';
  return 'error';
}

/** MUI 卡片只呈现已脱敏的动态额度窗口。 */
export function QuotaWindowCard({ quotaWindow, now }: QuotaWindowCardProps) {
  const marker = paceMarkerPercent(quotaWindow, now);
  const color = quotaColor(quotaWindow.remainingPercent);
  const suggestedRemaining = marker === null ? null : Math.round(marker);

  return (
    <Card variant="outlined" component="article" aria-label={quotaWindow.label}>
      <CardContent sx={{ p: 1.5, '&:last-child': { pb: 1.5 } }}>
        <Stack direction="row" spacing={1} sx={{ justifyContent: 'space-between', alignItems: 'baseline' }}>
          <Typography variant="subtitle2" noWrap sx={{ fontWeight: 700 }}>
            {quotaWindow.label}
          </Typography>
          <Typography component="strong" variant="h5" color={`${color}.main`} sx={{ fontWeight: 800 }}>
            {quotaWindow.remainingPercent}%
          </Typography>
        </Stack>

        <Stack direction="row" spacing={1} sx={{ justifyContent: 'space-between', mt: 0.35 }}>
          <Typography variant="caption" color="text.secondary">
            已用 {quotaWindow.usedPercent}%
          </Typography>
          <Typography variant="caption" color="text.secondary" noWrap>
            重置 {formatDateTime(quotaWindow.resetAt)}
          </Typography>
        </Stack>

        <Box sx={{ position: 'relative', mt: 1.2 }}>
          <LinearProgress
            aria-label={`剩余 ${quotaWindow.remainingPercent}%`}
            color={color}
            variant="determinate"
            value={quotaWindow.remainingPercent}
            sx={{ height: 8, borderRadius: 999 }}
          />
          {marker !== null && (
            <Tooltip title="按当前窗口时间进度实时估算的平均消耗节奏，不代表官方阈值" placement="top">
              <Box
                component="span"
                aria-label={`建议控量不低于 ${suggestedRemaining}%`}
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
            剩余 {formatDuration(quotaWindow.resetAfterSeconds)}
          </Typography>
          {marker !== null && (
            <Typography variant="caption" color="text.secondary">
              建议控量 ≥ {suggestedRemaining}%
            </Typography>
          )}
        </Stack>
      </CardContent>
    </Card>
  );
}
