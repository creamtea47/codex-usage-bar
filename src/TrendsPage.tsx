import DeleteOutlineRoundedIcon from '@mui/icons-material/DeleteOutlineRounded';
import QueryStatsRoundedIcon from '@mui/icons-material/QueryStatsRounded';
import StorageRoundedIcon from '@mui/icons-material/StorageRounded';
import {
  Alert,
  Box,
  Button,
  Card,
  CardContent,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  FormControlLabel,
  LinearProgress,
  Snackbar,
  Stack,
  Switch,
  ToggleButton,
  ToggleButtonGroup,
  Typography,
  useTheme,
  type AlertColor,
} from '@mui/material';
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Area,
  AreaChart,
  CartesianGrid,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';

import { formatDateTime, formatDuration } from './format';
import type {
  ForecastStatus,
  QuotaFallbackLabel,
  UsageHistoryPoint,
  UsageHistoryRange,
  UsageHistoryResponse,
  UsageHistorySeries,
} from './types';

type Unlisten = () => void;

export interface TrendsPageProps {
  historyEnabled: boolean;
  getUsageHistory: (range: UsageHistoryRange) => Promise<UsageHistoryResponse>;
  onSetHistoryEnabled: (enabled: boolean) => Promise<void>;
  onClearUsageHistory: () => Promise<void>;
  listenForUsageHistory?: (handler: () => void) => Promise<Unlisten>;
}

type TrendsTranslationKey =
  | 'trends.title'
  | 'trends.subtitle'
  | 'trends.range.aria'
  | 'trends.range.hours24'
  | 'trends.range.days7'
  | 'trends.localOnly.title'
  | 'trends.localOnly.description'
  | 'trends.collection.label'
  | 'trends.collection.enabled'
  | 'trends.collection.disabled'
  | 'trends.collection.enableSuccess'
  | 'trends.collection.disableSuccess'
  | 'trends.collection.error'
  | 'trends.clear.button'
  | 'trends.clear.dialogTitle'
  | 'trends.clear.dialogBody'
  | 'trends.clear.cancel'
  | 'trends.clear.confirm'
  | 'trends.clear.success'
  | 'trends.clear.error'
  | 'trends.events.error'
  | 'trends.loading'
  | 'trends.loadError'
  | 'trends.retry'
  | 'trends.empty.title'
  | 'trends.empty.enabledDescription'
  | 'trends.empty.disabledDescription'
  | 'trends.storage.recovered'
  | 'trends.storage.unavailable'
  | 'trends.summary'
  | 'trends.windowAria'
  | 'trends.current.label'
  | 'trends.current.value'
  | 'trends.consumed.label'
  | 'trends.consumed.value'
  | 'trends.chart.aria'
  | 'trends.chart.remaining'
  | 'trends.chart.reset'
  | 'trends.chart.noData'
  | 'trends.forecast.label'
  | 'trends.forecast.collecting'
  | 'trends.forecast.stable'
  | 'trends.forecast.exhaustsAt'
  | 'trends.forecast.exhaustsBeforeReset'
  | 'trends.forecast.lastsUntilReset'
  | 'trends.forecast.basis';

interface ChartPoint {
  timestamp: number;
  remainingPercent: number | null;
}

interface PreparedChartData {
  points: ChartPoint[];
  resetMarkers: number[];
  domain: [number, number];
}

interface Feedback {
  key: TrendsTranslationKey;
  severity: AlertColor;
}

function clampPercent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(100, Math.max(0, value));
}

function languageForFormatting(language?: string): 'zh-CN' | 'en' {
  return language && /^zh(?:-|$)/i.test(language) ? 'zh-CN' : 'en';
}

function validHistoryPoints(points: UsageHistoryPoint[]) {
  return points
    .map((point) => ({
      timestamp: Date.parse(point.sampledAt),
      remainingPercent: clampPercent(point.remainingPercent),
      breakBefore: point.breakBefore,
    }))
    .filter((point) => Number.isFinite(point.timestamp))
    .sort((left, right) => left.timestamp - right.timestamp);
}

/** Inserts a null between reset cycles so Recharts cannot connect both sides. */
function prepareChartData(points: UsageHistoryPoint[]): PreparedChartData {
  const valid = validHistoryPoints(points);
  const chartPoints: ChartPoint[] = [];
  const resetMarkers: number[] = [];

  valid.forEach((point, index) => {
    const previous = valid[index - 1];
    if (index > 0 && point.breakBefore && previous) {
      const midpoint =
        point.timestamp > previous.timestamp
          ? previous.timestamp + (point.timestamp - previous.timestamp) / 2
          : point.timestamp - 1;
      chartPoints.push({ timestamp: midpoint, remainingPercent: null });
      resetMarkers.push(point.timestamp);
    }
    chartPoints.push({ timestamp: point.timestamp, remainingPercent: point.remainingPercent });
  });

  const timestamps = valid.map((point) => point.timestamp);
  const minimum = timestamps[0] ?? Date.now() - 30 * 60 * 1_000;
  const maximum = timestamps[timestamps.length - 1] ?? Date.now() + 30 * 60 * 1_000;
  const padding = minimum === maximum ? 30 * 60 * 1_000 : 0;
  return {
    points: chartPoints,
    resetMarkers,
    domain: [minimum - padding, maximum + padding],
  };
}

function formatAxisTime(timestamp: number, range: UsageHistoryRange, language: 'zh-CN' | 'en'): string {
  if (!Number.isFinite(timestamp)) return '';
  return new Intl.DateTimeFormat(language, {
    ...(range === '24h'
      ? { hour: '2-digit', minute: '2-digit' }
      : { month: 'numeric', day: 'numeric' }),
  }).format(new Date(timestamp));
}

function fallbackWindowLabel(
  fallbackLabel: QuotaFallbackLabel,
  windowSeconds: number,
  language: 'zh-CN' | 'en',
  translate: (key: string, options?: Record<string, unknown>) => string,
): string {
  if (fallbackLabel === 'fiveHour') return translate('quota.fallbackLabel.fiveHour');
  if (fallbackLabel === 'weekly') return translate('quota.fallbackLabel.weekly');
  return translate('quota.fallbackLabel.window', {
    duration: formatDuration(windowSeconds, language),
  });
}

function forecastMessage(
  status: ForecastStatus,
  exhaustsAt: string | null,
  language: 'zh-CN' | 'en',
  translate: (key: TrendsTranslationKey, options?: Record<string, unknown>) => string,
): string {
  if (status === 'collecting') return translate('trends.forecast.collecting');
  if (status === 'stable') return translate('trends.forecast.stable');
  if (status === 'lastsUntilReset') return translate('trends.forecast.lastsUntilReset');
  if (exhaustsAt) {
    return translate('trends.forecast.exhaustsAt', {
      date: formatDateTime(exhaustsAt, language),
    });
  }
  return translate('trends.forecast.exhaustsBeforeReset');
}

interface SeriesCardProps {
  range: UsageHistoryRange;
  series: UsageHistorySeries;
  language: 'zh-CN' | 'en';
  translate: (key: TrendsTranslationKey, options?: Record<string, unknown>) => string;
  translateAny: (key: string, options?: Record<string, unknown>) => string;
}

function SeriesCard({ range, series, language, translate, translateAny }: SeriesCardProps) {
  const theme = useTheme();
  const gradientId = `usage-trend-${useId().replace(/:/g, '')}`;
  const chart = useMemo(() => prepareChartData(series.points), [series.points]);
  const label = fallbackWindowLabel(
    series.fallbackLabel,
    series.windowSeconds,
    language,
    translateAny,
  );
  const current =
    series.currentRemainingPercent === null
      ? translateAny('common.unavailable')
      : translate('trends.current.value', {
          percent: Math.round(clampPercent(series.currentRemainingPercent)),
        });
  const consumed = translate('trends.consumed.value', {
    percent: Math.round(clampPercent(series.consumedPercent)),
  });
  const forecast = forecastMessage(series.forecast.status, series.forecast.exhaustsAt, language, translate);
  const basis = translate('trends.forecast.basis', {
    count: series.forecast.sampleCount,
    duration: formatDuration(series.forecast.observedSpanSeconds, language),
    percent: Math.round(clampPercent(series.forecast.consumedPercent)),
  });

  return (
    <Card
      component="article"
      variant="outlined"
      aria-label={translate('trends.windowAria', { label })}
      sx={{ overflow: 'visible' }}
    >
      <CardContent sx={{ p: 2.25, '&:last-child': { pb: 2.25 } }}>
        <Stack
          direction={{ xs: 'column', sm: 'row' }}
          spacing={1.5}
          sx={{ justifyContent: 'space-between', alignItems: { xs: 'stretch', sm: 'flex-start' } }}
        >
          <Typography variant="h6" sx={{ fontWeight: 800 }}>
            {label}
          </Typography>
          <Stack direction="row" spacing={3} sx={{ flexShrink: 0 }}>
            <Box>
              <Typography variant="caption" color="text.secondary">
                {translate('trends.current.label')}
              </Typography>
              <Typography variant="subtitle1" sx={{ fontWeight: 800 }}>
                {current}
              </Typography>
            </Box>
            <Box>
              <Typography variant="caption" color="text.secondary">
                {translate('trends.consumed.label')}
              </Typography>
              <Typography variant="subtitle1" sx={{ fontWeight: 800 }}>
                {consumed}
              </Typography>
            </Box>
          </Stack>
        </Stack>

        {chart.points.length > 0 ? (
          <Box
            role="img"
            aria-label={translate('trends.chart.aria', { label })}
            sx={{ width: '100%', height: 236, mt: 1.5 }}
          >
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={chart.points} margin={{ top: 14, right: 12, bottom: 4, left: -12 }}>
                <defs>
                  <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
                    <stop offset="5%" stopColor={theme.palette.primary.main} stopOpacity={0.36} />
                    <stop offset="95%" stopColor={theme.palette.primary.main} stopOpacity={0.03} />
                  </linearGradient>
                </defs>
                <CartesianGrid stroke={theme.palette.divider} strokeDasharray="3 3" vertical={false} />
                <XAxis
                  dataKey="timestamp"
                  type="number"
                  scale="time"
                  domain={chart.domain}
                  tickFormatter={(value: number) => formatAxisTime(value, range, language)}
                  tick={{ fill: theme.palette.text.secondary, fontSize: 11 }}
                  tickLine={false}
                  axisLine={{ stroke: theme.palette.divider }}
                  minTickGap={28}
                />
                <YAxis
                  domain={[0, 100]}
                  ticks={[0, 25, 50, 75, 100]}
                  unit="%"
                  width={48}
                  tick={{ fill: theme.palette.text.secondary, fontSize: 11 }}
                  tickLine={false}
                  axisLine={false}
                />
                <Tooltip
                  isAnimationActive={false}
                  formatter={(value) => [
                    `${Math.round(Number(Array.isArray(value) ? value[0] : value))}%`,
                    translate('trends.chart.remaining'),
                  ]}
                  labelFormatter={(value) => formatDateTime(new Date(Number(value)).toISOString(), language)}
                  contentStyle={{
                    borderRadius: theme.shape.borderRadius,
                    borderColor: theme.palette.divider,
                    backgroundColor: theme.palette.background.paper,
                    color: theme.palette.text.primary,
                  }}
                />
                {chart.resetMarkers.map((timestamp) => (
                  <ReferenceLine
                    key={timestamp}
                    x={timestamp}
                    stroke={theme.palette.warning.main}
                    strokeDasharray="4 4"
                    label={{
                      value: translate('trends.chart.reset'),
                      position: 'insideTopRight',
                      fill: theme.palette.text.secondary,
                      fontSize: 11,
                    }}
                  />
                ))}
                <Area
                  type="monotone"
                  dataKey="remainingPercent"
                  connectNulls={false}
                  stroke={theme.palette.primary.main}
                  strokeWidth={2}
                  fill={`url(#${gradientId})`}
                  activeDot={{ r: 4 }}
                  dot={chart.points.filter((point) => point.remainingPercent !== null).length <= 12}
                  isAnimationActive={false}
                />
              </AreaChart>
            </ResponsiveContainer>
          </Box>
        ) : (
          <Alert severity="info" sx={{ mt: 1.5 }}>
            {translate('trends.chart.noData')}
          </Alert>
        )}

        <Box sx={{ mt: 1.5, pt: 1.5, borderTop: 1, borderColor: 'divider' }}>
          <Typography variant="caption" color="text.secondary">
            {translate('trends.forecast.label')}
          </Typography>
          <Typography variant="body2" sx={{ mt: 0.25, fontWeight: 700 }}>
            {forecast}
          </Typography>
          <Typography variant="caption" color="text.secondary" sx={{ display: 'block', mt: 0.45 }}>
            {basis}
          </Typography>
        </Box>
      </CardContent>
    </Card>
  );
}

/** Lazy-load this default export from SettingsWindow so Recharts stays out of the main-card chunk. */
export default function TrendsPage({
  historyEnabled,
  getUsageHistory,
  onSetHistoryEnabled,
  onClearUsageHistory,
  listenForUsageHistory,
}: TrendsPageProps) {
  const { t, i18n } = useTranslation();
  const theme = useTheme();
  const language = languageForFormatting(i18n.resolvedLanguage ?? i18n.language);
  const translate = useCallback(
    (key: TrendsTranslationKey, options?: Record<string, unknown>) =>
      t(key as never, options as never) as unknown as string,
    [t],
  );
  const translateAny = useCallback(
    (key: string, options?: Record<string, unknown>) =>
      t(key as never, options as never) as unknown as string,
    [t],
  );
  const [range, setRange] = useState<UsageHistoryRange>('24h');
  const [history, setHistory] = useState<UsageHistoryResponse | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [loadFailed, setLoadFailed] = useState(false);
  const [reloadVersion, setReloadVersion] = useState(0);
  const [isMutating, setIsMutating] = useState(false);
  const [isClearDialogOpen, setIsClearDialogOpen] = useState(false);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const [historyListenerState, setHistoryListenerState] = useState<{
    source: TrendsPageProps['listenForUsageHistory'];
    ready: boolean;
  }>(() => ({ source: listenForUsageHistory, ready: !listenForUsageHistory }));
  const isHistoryListenerReady =
    !listenForUsageHistory ||
    (historyListenerState.source === listenForUsageHistory && historyListenerState.ready);
  const requestGenerationRef = useRef(0);
  const requestReload = useCallback(() => {
    // 先同步作废所有在途读取，再触发下一次读取，避免旧响应覆盖更新事件后的快照。
    requestGenerationRef.current += 1;
    setIsLoading(true);
    setLoadFailed(false);
    setReloadVersion((version) => version + 1);
  }, []);

  useEffect(() => {
    if (!listenForUsageHistory) {
      return undefined;
    }
    let disposed = false;
    let unlisten: Unlisten | undefined;
    void listenForUsageHistory(() => {
      if (!disposed) {
        requestReload();
      }
    })
      .then((removeListener) => {
        if (disposed) removeListener();
        else {
          unlisten = removeListener;
          setHistoryListenerState({ source: listenForUsageHistory, ready: true });
        }
      })
      .catch(() => {
        if (!disposed) {
          setFeedback({ key: 'trends.events.error', severity: 'error' });
          // Event delivery is optional. A failed subscription must not block
          // the settings window from reading the current history snapshot.
          setHistoryListenerState({ source: listenForUsageHistory, ready: true });
        }
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [listenForUsageHistory, requestReload]);

  useEffect(() => {
    if (!isHistoryListenerReady) return undefined;
    let disposed = false;
    const requestGeneration = ++requestGenerationRef.current;
    void getUsageHistory(range)
      .then((response) => {
        if (!disposed && requestGeneration === requestGenerationRef.current) {
          setHistory(response);
        }
      })
      .catch(() => {
        if (!disposed && requestGeneration === requestGenerationRef.current) {
          setLoadFailed(true);
        }
      })
      .finally(() => {
        if (!disposed && requestGeneration === requestGenerationRef.current) {
          setIsLoading(false);
        }
      });
    return () => {
      disposed = true;
    };
  }, [getUsageHistory, isHistoryListenerReady, range, reloadVersion]);

  const setCollectionEnabled = async (enabled: boolean) => {
    if (isMutating) return;
    setIsMutating(true);
    try {
      await onSetHistoryEnabled(enabled);
      setFeedback({
        key: enabled ? 'trends.collection.enableSuccess' : 'trends.collection.disableSuccess',
        severity: 'success',
      });
    } catch {
      setFeedback({ key: 'trends.collection.error', severity: 'error' });
    } finally {
      setIsMutating(false);
    }
  };

  const clearHistory = async () => {
    if (isMutating) return;
    setIsMutating(true);
    try {
      await onClearUsageHistory();
      // 即使历史事件监听失败，也要同步作废清除前的在途读取，避免旧快照让已删除数据重新出现。
      requestGenerationRef.current += 1;
      setIsLoading(false);
      setLoadFailed(false);
      setHistory((current) =>
        current
          ? {
              ...current,
              storageStatus: 'empty',
              sampleCount: 0,
              earliestSampleAt: null,
              latestSampleAt: null,
              series: [],
            }
          : current,
      );
      setIsClearDialogOpen(false);
      setFeedback({ key: 'trends.clear.success', severity: 'success' });
    } catch {
      setFeedback({ key: 'trends.clear.error', severity: 'error' });
    } finally {
      setIsMutating(false);
    }
  };

  const isEmpty = !isLoading && !loadFailed && (history?.series.length ?? 0) === 0;

  return (
    <Stack component="section" spacing={2.25} aria-labelledby="trends-page-title">
      <Stack
        direction={{ xs: 'column', sm: 'row' }}
        spacing={1.5}
        sx={{ justifyContent: 'space-between', alignItems: { xs: 'stretch', sm: 'center' } }}
      >
        <Box>
          <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
            <QueryStatsRoundedIcon color="primary" />
            <Typography id="trends-page-title" variant="h5" sx={{ fontWeight: 800 }}>
              {translate('trends.title')}
            </Typography>
          </Stack>
          <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
            {translate('trends.subtitle')}
          </Typography>
        </Box>
        <ToggleButtonGroup
          exclusive
          size="small"
          value={range}
          onChange={(_, nextRange: UsageHistoryRange | null) => {
            if (nextRange) {
              setHistory(null);
              setRange(nextRange);
              requestReload();
            }
          }}
          aria-label={translate('trends.range.aria')}
        >
          <ToggleButton value="24h">{translate('trends.range.hours24')}</ToggleButton>
          <ToggleButton value="7d">{translate('trends.range.days7')}</ToggleButton>
        </ToggleButtonGroup>
      </Stack>

      <Card variant="outlined">
        <CardContent sx={{ p: 2, '&:last-child': { pb: 2 } }}>
          <Stack
            direction={{ xs: 'column', sm: 'row' }}
            spacing={2}
            sx={{ justifyContent: 'space-between', alignItems: { xs: 'stretch', sm: 'center' } }}
          >
            <Stack direction="row" spacing={1.25} sx={{ alignItems: 'flex-start' }}>
              <StorageRoundedIcon color="primary" sx={{ mt: 0.2 }} />
              <Box>
                <Typography variant="subtitle2" sx={{ fontWeight: 800 }}>
                  {translate('trends.localOnly.title')}
                </Typography>
                <Typography variant="body2" color="text.secondary" sx={{ mt: 0.25 }}>
                  {translate('trends.localOnly.description')}
                </Typography>
              </Box>
            </Stack>
            <Stack direction="row" spacing={1} sx={{ alignItems: 'center', flexShrink: 0 }}>
              <FormControlLabel
                sx={{ m: 0 }}
                control={
                  <Switch
                    checked={historyEnabled}
                    disabled={isMutating}
                    onChange={(event) => void setCollectionEnabled(event.target.checked)}
                  />
                }
                label={translate('trends.collection.label')}
              />
              <Button
                color="error"
                variant="outlined"
                size="small"
                startIcon={<DeleteOutlineRoundedIcon />}
                disabled={isMutating}
                onClick={() => setIsClearDialogOpen(true)}
              >
                {translate('trends.clear.button')}
              </Button>
            </Stack>
          </Stack>
          <Alert severity={historyEnabled ? 'info' : 'warning'} sx={{ mt: 1.5 }}>
            {translate(historyEnabled ? 'trends.collection.enabled' : 'trends.collection.disabled')}
          </Alert>
        </CardContent>
      </Card>

      {isLoading && <LinearProgress aria-label={translate('trends.loading')} />}

      {history?.storageStatus === 'recovered' && (
        <Alert severity="warning">{translate('trends.storage.recovered')}</Alert>
      )}
      {history?.storageStatus === 'unavailable' && (
        <Alert severity="error">{translate('trends.storage.unavailable')}</Alert>
      )}

      {loadFailed && (
        <Alert
          severity="error"
          action={
            <Button color="inherit" size="small" onClick={requestReload}>
              {translate('trends.retry')}
            </Button>
          }
        >
          {translate('trends.loadError')}
        </Alert>
      )}

      {isLoading && !history && (
        <Stack spacing={1} sx={{ minHeight: 180, alignItems: 'center', justifyContent: 'center' }}>
          <CircularProgress size={28} />
          <Typography variant="body2" color="text.secondary">
            {translate('trends.loading')}
          </Typography>
        </Stack>
      )}

      {isEmpty && (
        <Box sx={{ py: 5, textAlign: 'center' }}>
          <Typography variant="h6" sx={{ fontWeight: 800 }}>
            {translate('trends.empty.title')}
          </Typography>
          <Typography variant="body2" color="text.secondary" sx={{ mt: 0.75 }}>
            {translate(
              historyEnabled ? 'trends.empty.enabledDescription' : 'trends.empty.disabledDescription',
            )}
          </Typography>
        </Box>
      )}

      {!loadFailed && history && history.series.length > 0 && (
        <>
          <Typography variant="caption" color="text.secondary">
            {translate('trends.summary', {
              count: history.sampleCount,
              date: history.latestSampleAt
                ? formatDateTime(history.latestSampleAt, language)
                : translateAny('common.unavailable'),
            })}
          </Typography>
          <Stack spacing={2}>
            {history.series.map((series, index) => (
              <SeriesCard
                key={`${series.windowId}-${index}`}
                range={range}
                series={series}
                language={language}
                translate={translate}
                translateAny={translateAny}
              />
            ))}
          </Stack>
        </>
      )}

      <Dialog
        open={isClearDialogOpen}
        onClose={() => {
          if (!isMutating) setIsClearDialogOpen(false);
        }}
        aria-labelledby="clear-history-dialog-title"
        maxWidth="xs"
        fullWidth
      >
        <DialogTitle id="clear-history-dialog-title">{translate('trends.clear.dialogTitle')}</DialogTitle>
        <DialogContent>
          <Typography variant="body2" color="text.secondary">
            {translate('trends.clear.dialogBody')}
          </Typography>
        </DialogContent>
        <DialogActions>
          <Button disabled={isMutating} onClick={() => setIsClearDialogOpen(false)}>
            {translate('trends.clear.cancel')}
          </Button>
          <Button color="error" variant="contained" disabled={isMutating} onClick={() => void clearHistory()}>
            {translate('trends.clear.confirm')}
          </Button>
        </DialogActions>
      </Dialog>

      <Snackbar
        open={feedback !== null}
        autoHideDuration={3_500}
        onClose={() => setFeedback(null)}
        anchorOrigin={{ vertical: 'bottom', horizontal: 'center' }}
      >
        {feedback ? (
          <Alert
            closeText={translateAny('common.close')}
            severity={feedback.severity}
            variant="filled"
            onClose={() => setFeedback(null)}
            sx={{ color: theme.palette.getContrastText(theme.palette[feedback.severity].main) }}
          >
            {translate(feedback.key)}
          </Alert>
        ) : undefined}
      </Snackbar>
    </Stack>
  );
}
