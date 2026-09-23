import { Chip, IconButton, Stack, Tooltip, Typography } from '@mui/material';
import RefreshRoundedIcon from '@mui/icons-material/RefreshRounded';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { accountsBridge } from './accountsBridge';
import type { ResetCreditDetails } from './accountTypes';

function expiryDate(value: string, language: string, seconds = false): string {
  return new Intl.DateTimeFormat(language, { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', ...(seconds ? { second: '2-digit' } : {}), hour12: false }).format(new Date(value));
}

/** 仅在账号页挂载且可见时查询。计数取摘要，不以明细条数覆盖持有数或当前可用数。 */
export function ResetCreditBadge({ accountId, available, applicable, now }: { accountId: string; available: number; applicable: number | null; now: number }) {
  const { t, i18n } = useTranslation();
  const [snapshot, setSnapshot] = useState<{ count: number; value: ResetCreditDetails } | null>(null);
  const [retry, setRetry] = useState(0);
  const [busy, setBusy] = useState(false);
  const details = snapshot?.count === available && snapshot.value.accountId === accountId ? snapshot.value : null;

  useEffect(() => {
    if (available < 1) return;
    let disposed = false;
    let pending = false;
    async function load(force = false) {
      if (document.visibilityState === 'hidden' || pending) return;
      pending = true;
      try {
        const result = await accountsBridge.resetCredits(accountId, force);
        if (!disposed && result.accountId === accountId) setSnapshot({ count: available, value: result });
      } catch {
        if (!disposed) setSnapshot((previous) => {
          const old = previous?.count === available && previous.value.accountId === accountId ? previous.value : null;
          return { count: available, value: { accountId, status: old?.fetchedAt ? 'stale' : 'unavailable', checkedAt: new Date().toISOString(), fetchedAt: old?.fetchedAt ?? null, credits: old?.credits ?? [], errorCode: 'localBridge' } };
        });
      } finally { pending = false; if (!disposed) setBusy(false); }
    }
    void load(retry > 0);
    const refresh = () => { void load(); };
    const interval = window.setInterval(refresh, 60_000);
    document.addEventListener('visibilitychange', refresh);
    return () => { disposed = true; window.clearInterval(interval); document.removeEventListener('visibilitychange', refresh); };
  }, [accountId, available, retry]);

  const dated = details?.credits.map((c) => c.expiresAt).filter((v): v is string => !!v && Number.isFinite(Date.parse(v))).sort((a, b) => Date.parse(a) - Date.parse(b)) ?? [];
  const next = dated[0];
  const failed = details && details.status !== 'ready';
  const tooltip = <Stack spacing={0.5}>
    <Typography variant="caption">{t('feature.compact.held', { count: available })}</Typography>
    {applicable !== null && <Typography variant="caption">{t('feature.compact.applicable', { count: applicable })}</Typography>}
    {applicable === 0 && <Typography variant="caption">{t('feature.compact.noApplicable')}</Typography>}
    <Typography variant="caption">{t('feature.compact.conditions')}</Typography>
    {available > 0 && <>
      {failed && <Typography variant="caption">{t('feature.compact.expiryUnavailable')}{details.fetchedAt ? ` · ${t('feature.compact.cached')}` : ''}</Typography>}
      {details?.credits.map((credit, index) => <Stack key={index}>
        <Typography variant="caption">{t('feature.compact.creditExpiry', { index: index + 1, date: credit.expiresAt ? expiryDate(credit.expiresAt, i18n.language, true) : t('feature.compact.expiryUnknown') })}</Typography>
        {credit.supportedByPlan === false && <Typography variant="caption">{t('feature.compact.unsupportedPlan')}</Typography>}
      </Stack>)}
      {!details && <Typography variant="caption">{t('feature.compact.expiryLoading')}</Typography>}
      {details?.status === 'ready' && !details.credits.length && <Typography variant="caption">{t('feature.compact.expiryUnknown')}</Typography>}
      {details?.fetchedAt && <Typography variant="caption">{t('feature.compact.detailsUpdated', { date: expiryDate(details.fetchedAt, i18n.language) })}</Typography>}
    </>}
  </Stack>;
  return <Stack spacing={0.5} sx={{ alignItems: 'flex-start' }}>
    <Tooltip title={tooltip} describeChild><Chip tabIndex={0} size="small" variant="outlined" label={t('feature.compact.held', { count: available })} /></Tooltip>
    {available > 0 && <Stack direction="row" spacing={0.5} sx={{ alignItems: 'center' }}>
      <Typography variant="caption" color={failed ? 'warning.main' : 'text.secondary'}>
        {failed ? t('feature.compact.expiryUnavailable') : next ? t(Date.parse(next) <= now ? 'feature.compact.expiredAt' : 'feature.compact.nearestExpiry', { date: expiryDate(next, i18n.language) }) : details ? t('feature.compact.expiryUnknown') : t('feature.compact.expiryLoading')}
      </Typography>
      {failed && <Tooltip title={t('feature.compact.retry')}><span><IconButton size="small" disabled={busy} aria-label={t('feature.compact.retry')} onClick={() => { setBusy(true); setRetry((v) => v + 1); }}><RefreshRoundedIcon fontSize="small" /></IconButton></span></Tooltip>}
    </Stack>}
  </Stack>;
}
