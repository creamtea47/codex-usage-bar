import { CycleList, RangeSummaryView } from './HistoryDetails';
import type { CycleSummary } from '@/contracts/history';
import { forecastText, useForecastClock } from '@/features/usage/forecastPresentation';
import { Alert, Box, Button, Card, CardContent, Stack, Typography, useTheme } from '@mui/material';
import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Area,
  AreaChart,
  CartesianGrid,
  ReferenceLine,
  ReferenceArea,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
  usePlotArea,
  useXAxisInverseDataSnapScale,
} from 'recharts';

import { formatDateTime, formatDuration } from '@/shared/format';
import type { UsageHistoryRequest, UsageHistorySeries } from '@/contracts/history';

import {
  type TrendsTranslationKey,
  prepareChartData,
  validHistoryPoints,
  formatAxisTime,
  clampPercent,
  safeConsumptionPercent,
  fallbackWindowLabel,
  forecastMessage,
} from './chartData';
import { useRangeSelection } from './useRangeSelection';
interface SeriesCardProps {
  accountId: string | null;
  request: UsageHistoryRequest;
  series: UsageHistorySeries;
  language: 'zh-CN' | 'en';
  translate: (key: TrendsTranslationKey, options?: Record<string, unknown>) => string;
  translateAny: (key: string, options?: Record<string, unknown>) => string;
}

/** 捕获绘图区点击，避免曲线填充层拦住选择，也不依赖异步悬浮状态。 */
function RangeClickLayer({ onSelect }: { onSelect: (timestamp: number) => void }) {
  const plot = usePlotArea();
  const inverse = useXAxisInverseDataSnapScale();
  const marker = useRef<SVGGElement>(null);
  useEffect(() => {
    const svg = marker.current?.ownerSVGElement;
    if (!plot || !inverse || !svg) return;
    const select = (event: MouseEvent) => {
      const matrix = svg.getScreenCTM();
      if (!matrix) return;
      const point = svg.createSVGPoint();
      point.x = event.clientX;
      point.y = event.clientY;
      const local = point.matrixTransform(matrix.inverse());
      if (
        local.x < plot.x ||
        local.x > plot.x + plot.width ||
        local.y < plot.y ||
        local.y > plot.y + plot.height
      )
        return;
      onSelect(Number(inverse(local.x)));
    };
    svg.addEventListener('click', select, true);
    return () => svg.removeEventListener('click', select, true);
  }, [plot, inverse, onSelect]);
  return <g ref={marker} aria-hidden="true" />;
}

export function SeriesCard({
  accountId,
  request,
  series,
  language,
  translate,
  translateAny,
}: SeriesCardProps) {
  const { t: featureT } = useTranslation();
  const forecastNow = useForecastClock();
  const { selection, setSelection, cursor, setCursor, rangeSummary, rangeError, start, end, selectPoint } =
    useRangeSelection(series, accountId);
  const [currentCycle, setCurrentCycle] = useState<CycleSummary | undefined>();
  const theme = useTheme();
  const gradientId = `usage-trend-${useId().replace(/:/g, '')}`;
  const chart = useMemo(() => prepareChartData(series.points), [series.points]);
  const label = fallbackWindowLabel(series.fallbackLabel, series.windowSeconds, language, translateAny);
  const current =
    series.currentRemainingPercent === null
      ? translateAny('common.unavailable')
      : translate('trends.current.value', {
          percent: Math.round(clampPercent(series.currentRemainingPercent)),
        });
  const consumed = translate('trends.consumed.value', {
    percent: Math.round(safeConsumptionPercent(series.todayConsumedPercent)),
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

        {chart.points.some((point) => point.suggestedRemainingPercent != null) && (
          <Typography variant="caption" color="text.secondary">
            {featureT('feature.paceLegend')}
          </Typography>
        )}
        {chart.points.length > 0 ? (
          <Box
            role="group"
            tabIndex={0}
            onKeyDown={(event) => {
              const points = validHistoryPoints(series.points);
              if (event.key === 'Escape') setSelection([]);
              if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {
                event.preventDefault();
                setCursor((i) =>
                  Math.max(0, Math.min(points.length - 1, i + (event.key === 'ArrowRight' ? 1 : -1))),
                );
              }
              if (event.key === 'Enter' && points[cursor]) {
                event.preventDefault();
                selectPoint(points[cursor].timestamp);
              }
            }}
            aria-label={translate('trends.chart.aria', { label })}
            aria-description={featureT('feature.rangeHint')}
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
                  tickFormatter={(value: number) => formatAxisTime(value, request, language)}
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
                {start !== null && end !== null && (
                  <ReferenceArea x1={start} x2={end} fill={theme.palette.primary.main} fillOpacity={0.15} />
                )}
                {selection.map((point, index) => (
                  <ReferenceLine
                    key={`selection-${index}`}
                    x={point}
                    stroke={theme.palette.primary.main}
                    strokeWidth={2}
                  />
                ))}
                <Tooltip
                  content={
                    selection.length === 2 && !rangeError
                      ? () => (
                          <Box
                            sx={{
                              p: 1.5,
                              bgcolor: 'background.paper',
                              border: 1,
                              borderColor: 'divider',
                              borderRadius: 1,
                              maxWidth: 380,
                            }}
                          >
                            <RangeSummaryView summary={rangeSummary} />
                          </Box>
                        )
                      : undefined
                  }
                  isAnimationActive={false}
                  formatter={(value, name) => [
                    `${Math.round(Number(Array.isArray(value) ? value[0] : value))}%`,
                    name === 'suggestedRemainingPercent'
                      ? featureT('feature.suggestedRemaining')
                      : translate('trends.chart.remaining'),
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
                <Area
                  type="linear"
                  dataKey="suggestedRemainingPercent"
                  connectNulls={false}
                  stroke={theme.palette.text.secondary}
                  strokeDasharray="6 4"
                  fill="none"
                  dot={false}
                  activeDot={false}
                  isAnimationActive={false}
                />
                <RangeClickLayer onSelect={selectPoint} />
              </AreaChart>
            </ResponsiveContainer>
          </Box>
        ) : (
          <Alert severity="info" sx={{ mt: 1.5 }}>
            {translate('trends.chart.noData')}
          </Alert>
        )}

        <Stack direction="row" sx={{ alignItems: 'center', gap: 1 }}>
          <Typography variant="caption" color="text.secondary">
            {featureT('feature.rangeHint')}
          </Typography>
          <Button size="small" disabled={!selection.length} onClick={() => setSelection([])}>
            {featureT('feature.clear')}
          </Button>
        </Stack>
        {selection.length === 2 && (
          <Box sx={{ mt: 1 }}>
            {rangeError ? (
              <Alert severity="error">{translate('trends.loadError')}</Alert>
            ) : (
              <RangeSummaryView summary={rangeSummary} />
            )}
          </Box>
        )}
        <Box sx={{ mt: 1.5, pt: 1.5, borderTop: 1, borderColor: 'divider' }}>
          <Typography variant="caption" color="text.secondary">
            {translate('trends.forecast.label')}
          </Typography>
          <Typography variant="body2" sx={{ mt: 0.25, fontWeight: 700 }}>
            {series.currentResetAt || currentCycle
              ? forecastText(
                  series.forecast,
                  series.currentResetAt ?? currentCycle?.resetAt ?? null,
                  forecastNow,
                  language,
                )
              : forecast}
          </Typography>
          <Typography variant="caption" color="text.secondary" sx={{ display: 'block', mt: 0.45 }}>
            {basis}
          </Typography>
        </Box>
        <CycleList
          accountId={accountId}
          now={forecastNow}
          windowId={series.windowId}
          request={request}
          lastSampleAt={series.points.at(-1)?.sampledAt}
          onCurrentCycle={setCurrentCycle}
        />
      </CardContent>
    </Card>
  );
}
