import { Accordion, AccordionDetails, AccordionSummary, Alert, Box, Button, Chip, CircularProgress, Stack, Table, TableBody, TableCell, TableContainer, TableHead, TableRow, Typography } from '@mui/material';
import ExpandMoreIcon from '@mui/icons-material/ExpandMore';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { accountsBridge, currentAccountId } from './accountsBridge';
import type { CycleSamples, CycleSummary, RangeSummary } from './accountTypes';
import type { UsageHistoryRequest } from './types';
import { formatDateTime } from './format';
import { forecastText, minuteDuration } from './forecastPresentation';

export function RangeSummaryView({ summary }: { summary: RangeSummary | null }) {
  const { t, i18n } = useTranslation();
  if (!summary) return <CircularProgress size={16} aria-label={t('trends.loading')} />;
  return <Stack spacing={0.5} aria-live="polite">
    <Typography sx={{ fontWeight: 700 }} variant="body2">{t('feature.range')}: {formatDateTime(summary.startAt, i18n.language)} → {formatDateTime(summary.endAt, i18n.language)}</Typography>
    <Typography variant="body2">{t('feature.duration')}: {minuteDuration(summary.durationSeconds, i18n.language)} · {t('feature.consumed')}: {t('feature.pp', { percent: summary.consumedPercent })}</Typography>
    <Typography variant="caption">{t('feature.firstLast')}: {summary.firstRemainingPercent ?? '—'}% → {summary.lastRemainingPercent ?? '—'}% · {t('feature.cycles', { count: summary.cycleCount })} · {t('feature.samples', { count: summary.sampleCount })}</Typography>
    {summary.partial && <Typography variant="caption" color="text.secondary">{t('feature.rangePartial')}</Typography>}
  </Stack>;
}

function Samples({ windowId, cycle }: { windowId: string; cycle: CycleSummary }) {
  const { t, i18n } = useTranslation();
  const [offset, setOffset] = useState(0);
  const [data, setData] = useState<CycleSamples | null>(null);
  const [error, setError] = useState(false);
  useEffect(() => {
    let disposed = false;
    void accountsBridge.analytics<CycleSamples>({ kind: 'samples', windowId, cycleId: cycle.cycleId, offset }).then((value) => {
      if (!disposed) { setData(value); setError(false); }
    }).catch(() => { if (!disposed) setError(true); });
    return () => { disposed = true; };
  }, [windowId, cycle.cycleId, cycle.sampleCount, offset]);
  if (error) return <Alert severity="error">{t('trends.loadError')}</Alert>;
  if (!data) return <CircularProgress size={16} />;
  return <>
    <Typography variant="caption" color="text.secondary">{t('feature.retained')} {data.bucketSeconds > 0 && t('feature.precision', { duration: minuteDuration(data.bucketSeconds, i18n.language) })}</Typography>
    <TableContainer><Table size="small" aria-label={t('feature.detail')}><TableHead><TableRow><TableCell>{t('feature.detail')}</TableCell><TableCell>{t('feature.remaining')}</TableCell><TableCell>{t('feature.used')}</TableCell></TableRow></TableHead><TableBody>
      {data.points.map((p) => <TableRow key={p.sampledAt}><TableCell>{formatDateTime(p.sampledAt, i18n.language)}</TableCell><TableCell>{p.remainingPercent}%</TableCell><TableCell>{100 - p.remainingPercent}%</TableCell></TableRow>)}
    </TableBody></Table></TableContainer>
    <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mt: 1 }}>
      <Button disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - 50))}>{t('feature.previous')}</Button>
      <Typography variant="caption">{data.offset + 1}–{data.offset + data.points.length} / {data.total}</Typography>
      <Button disabled={offset + 50 >= data.total} onClick={() => setOffset(offset + 50)}>{t('feature.next')}</Button>
    </Stack>
  </>;
}

export function CycleList({ windowId, request, lastSampleAt, onCurrentCycle, now }: { windowId: string; request: UsageHistoryRequest; lastSampleAt?: string; onCurrentCycle?: (cycle: CycleSummary | undefined) => void; now: number }) {
  const { t, i18n } = useTranslation();
  const [cycles, setCycles] = useState<CycleSummary[]>([]);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [visible, setVisible] = useState(20);
  const [error, setError] = useState(false);
  const requestKey = JSON.stringify(request);
  const accountId = currentAccountId();
  useEffect(() => {
    let disposed = false;
    void accountsBridge.analytics<CycleSummary[]>({ kind: 'cycles', windowId, request: JSON.parse(requestKey) as UsageHistoryRequest }, accountId).then((value) => {
      if (!disposed) { setCycles(value); setError(false); onCurrentCycle?.(value.find((c) => c.current)); }
    }).catch(() => { if (!disposed) setError(true); });
    return () => { disposed = true; };
  }, [windowId, requestKey, lastSampleAt, accountId, onCurrentCycle]);
  return <Box sx={{ mt: 2 }}>
    <Typography variant="subtitle1" sx={{ mb: 1, fontWeight: 800 }}>{t('feature.cycleList')}</Typography>
    {error && <Alert severity="error">{t('trends.loadError')}</Alert>}
    {cycles.slice(0, visible).map((cycle) => {
      const empty = cycle.firstExhaustedAt;
      const early = empty && cycle.resetAt ? Math.max(0, (Date.parse(cycle.resetAt) - Date.parse(empty)) / 1000) : 0;
      const description = empty
        ? `${t('feature.exhausted', { date: formatDateTime(empty, i18n.language) })}${early > 0 ? ` (${t('feature.early', { duration: minuteDuration(early, i18n.language) })})` : ''}`
        : cycle.current ? forecastText(cycle.forecast, cycle.resetAt, now, i18n.language) : t('feature.notExhausted');
      return <Accordion key={cycle.cycleId} expanded={expanded === cycle.cycleId} onChange={(_, value) => setExpanded(value ? cycle.cycleId : null)} disableGutters variant="outlined">
        <AccordionSummary expandIcon={<ExpandMoreIcon />}><Stack spacing={0.7} sx={{ minWidth: 0, width: '100%' }}>
          <Typography variant="body2" sx={{ fontWeight: 700 }}>{formatDateTime(cycle.startAt, i18n.language)} → {formatDateTime(cycle.endAt, i18n.language)}</Typography>
          <Stack direction="row" sx={{ flexWrap: 'wrap', gap: 0.5 }}>
            {cycle.current && <Chip size="small" color="primary" label={t('feature.current')} />}
            {cycle.partial && <Chip size="small" label={t('feature.partial')} />}
            {cycle.outsideChartRange && <Chip size="small" label={t('feature.outside')} />}
          </Stack>
          <Typography variant="body2">{t('feature.consumed')}: {t('feature.pp', { percent: cycle.consumedPercent })} · {t('feature.used')}: {100 - cycle.lastRemainingPercent}% · {t('feature.duration')}: {minuteDuration(cycle.durationSeconds, i18n.language)}</Typography>
          <Typography variant="caption">{t('feature.firstLast')}: {cycle.firstRemainingPercent}% → {cycle.lastRemainingPercent}% · {t('feature.reset')}: {formatDateTime(cycle.resetAt, i18n.language)}</Typography>
          <Typography variant="body2" color={early > 0 ? 'warning.main' : 'text.secondary'}>{description}</Typography>
        </Stack></AccordionSummary>
        <AccordionDetails>{expanded === cycle.cycleId && <Samples windowId={windowId} cycle={cycle} />}</AccordionDetails>
      </Accordion>;
    })}
    {visible < cycles.length && <Button onClick={() => setVisible(visible + 20)}>{t('feature.next')}</Button>}
  </Box>;
}
