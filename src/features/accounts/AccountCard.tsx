import { ExtraCreditBalance } from '@/features/usage/ExtraCreditBalance';
import { ResetCreditBadge } from '@/features/usage/ResetCreditBadge';
import {
  Alert,
  Avatar,
  Box,
  Button,
  Card,
  CardActions,
  CardContent,
  CardHeader,
  Chip,
  Collapse,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  FormControlLabel,
  IconButton,
  LinearProgress,
  Stack,
  Switch,
  TextField,
  Tooltip,
  Typography,
} from '@mui/material';
import EditOutlinedIcon from '@mui/icons-material/EditOutlined';
import ComputerOutlinedIcon from '@mui/icons-material/ComputerOutlined';
import DeleteOutlineRoundedIcon from '@mui/icons-material/DeleteOutlineRounded';
import ExpandMoreRoundedIcon from '@mui/icons-material/ExpandMoreRounded';
import ExpandLessRoundedIcon from '@mui/icons-material/ExpandLessRounded';
import EventAvailableOutlinedIcon from '@mui/icons-material/EventAvailableOutlined';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { accountActions } from './accountActions';
import { accountError } from './accountError';
import type { Account } from '@/contracts/accounts';
import { formatDateTime, formatDuration, planLabel, resetCountdownSeconds } from '@/shared/format';
import { minuteDuration } from '@/features/usage/forecastPresentation';

import { accountTitle, accountNote, recordDate } from './accountPresentation';

import { CopyButton } from './CopyButton';
function DetailRow({
  label,
  value,
  copy = false,
  shorten = false,
}: {
  label: string;
  value?: string | null;
  copy?: boolean;
  shorten?: boolean;
}) {
  if (!value) return null;
  const display = shorten && value.length > 22 ? `${value.slice(0, 12)}…${value.slice(-6)}` : value;
  return (
    <Stack direction="row" spacing={1} sx={{ alignItems: 'center', minWidth: 0 }}>
      <Typography variant="caption" color="text.secondary" sx={{ flexShrink: 0 }}>
        {label}
      </Typography>
      <Tooltip title={value}>
        <Typography
          variant="body2"
          sx={{ flex: 1, minWidth: 0, textAlign: 'right', overflowWrap: 'anywhere' }}
        >
          {display}
        </Typography>
      </Tooltip>
      {copy && <CopyButton value={value} label={label} />}
    </Stack>
  );
}

export function AccountCard({
  account,
  selected,
  codexMatched,
  now,
}: {
  account: Account;
  selected: boolean;
  codexMatched: boolean;
  now: number;
}) {
  const { t, i18n: locale } = useTranslation();
  const [note, setNote] = useState(accountNote(account) ?? '');
  const [editing, setEditing] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [confirm, setConfirm] = useState<'apply' | 'remove' | null>(null);
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState<{ text: string; failed: boolean } | null>(null);
  const details = account.details;
  const title = accountTitle(account);
  const plan = planLabel(details?.planType ?? account.dashboard.planLabel);
  const subheader = [details?.name !== title ? details?.name : null, accountNote(account)]
    .filter(Boolean)
    .join(' · ');
  const subscriptionEnd = Date.parse(details?.subscriptionEndsAt ?? '');
  const subscriptionPending = Number.isFinite(subscriptionEnd) && subscriptionEnd <= now;
  const subscriptionDays = Math.max(0, Math.ceil((subscriptionEnd - now) / 86400000));
  const stale =
    account.dashboard.status === 'stale' ||
    (account.dashboard.status === 'error' && !!details?.usageObservedAt);
  const run = async (action: () => Promise<unknown>, success: string = t('feature.saved')) => {
    setBusy(true);
    setFeedback(null);
    try {
      await action();
      setFeedback({ text: success, failed: false });
    } catch (error) {
      setFeedback({ text: accountError(error), failed: true });
    } finally {
      setBusy(false);
      setConfirm(null);
    }
  };
  const issue =
    account.authStatus !== 'ready'
      ? accountError(account.authStatus)
      : account.dashboard.message
        ? t(`dashboardError.${account.dashboard.message}`)
        : null;
  const providerLabels: Record<string, string> = {
    google: 'Google',
    apple: 'Apple',
    microsoft: 'Microsoft',
    auth0: 'OpenAI',
    email: 'Email',
  };
  return (
    <Card
      variant="outlined"
      component="article"
      aria-label={title}
      sx={{ height: '100%', display: 'flex', flexDirection: 'column', minWidth: 0 }}
    >
      <CardHeader
        avatar={
          <Avatar sx={{ bgcolor: 'primary.main', color: 'primary.contrastText' }}>
            {title[0]?.toUpperCase()}
          </Avatar>
        }
        title={
          <Tooltip title={title}>
            <Typography variant="subtitle1" noWrap sx={{ fontWeight: 700 }}>
              {title}
            </Typography>
          </Tooltip>
        }
        subheader={
          subheader ? (
            <Tooltip title={subheader}>
              <Typography variant="body2" color="text.secondary" noWrap>
                {subheader}
              </Typography>
            </Tooltip>
          ) : undefined
        }
        action={
          plan ? (
            <Tooltip
              title={`${plan} · ${details?.planSource === 'credentials' ? t('feature.accountUi.sourceCredentials') : t('feature.accountUi.sourceUsage')}`}
            >
              <Chip label={plan} size="small" color="primary" variant="outlined" sx={{ maxWidth: 128 }} />
            </Tooltip>
          ) : undefined
        }
        slotProps={{ content: { sx: { minWidth: 0 } }, action: { sx: { alignSelf: 'center', m: 0 } } }}
      />
      <CardContent sx={{ pt: 0 }}>
        <Stack spacing={2}>
          {(selected || codexMatched || issue || stale) && (
            <Stack direction="row" sx={{ gap: 0.75, flexWrap: 'wrap' }}>
              {selected && <Chip size="small" variant="outlined" label={t('feature.accountUi.viewing')} />}
              {codexMatched && (
                <Chip
                  size="small"
                  color="success"
                  variant="outlined"
                  icon={<ComputerOutlinedIcon />}
                  label={t('feature.accountUi.codexFile')}
                />
              )}
              {issue && (
                <Tooltip title={issue}>
                  <Chip
                    size="small"
                    color="warning"
                    label={t(
                      account.authStatus !== 'ready'
                        ? 'feature.accountUi.authIssue'
                        : 'feature.accountUi.refreshIssue',
                    )}
                  />
                </Tooltip>
              )}
              {stale && (
                <Chip size="small" color="warning" variant="outlined" label={t('feature.accountUi.stale')} />
              )}
            </Stack>
          )}
          {editing && (
            <Stack direction="row" spacing={0.5} sx={{ alignItems: 'center' }}>
              <TextField
                autoFocus
                size="small"
                fullWidth
                label={t('feature.note')}
                value={note}
                onChange={(e) => setNote(e.target.value)}
                disabled={busy}
              />
              <Button
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await accountActions.update({
                      ...account,
                      label:
                        note.trim() ||
                        account.dashboard.accountEmailMasked ||
                        `Account ${account.id.slice(0, 8)}`,
                    });
                    setEditing(false);
                  })
                }
              >
                {t('feature.save')}
              </Button>
              <Button disabled={busy} onClick={() => setEditing(false)}>
                {t('feature.cancel')}
              </Button>
            </Stack>
          )}
          <ExtraCreditBalance credits={account.dashboard.extraCredits ?? details?.extraCredits} />
          {account.dashboard.quotaWindows.length
            ? account.dashboard.quotaWindows.map((quota) => {
                const label =
                  quota.label ??
                  (quota.fallbackLabel === 'weekly'
                    ? t('quota.fallbackLabel.weekly')
                    : quota.fallbackLabel === 'fiveHour'
                      ? t('quota.fallbackLabel.fiveHour')
                      : t('quota.fallbackLabel.window', {
                          duration: formatDuration(quota.windowSeconds, locale.language),
                        }));
                const countdown = resetCountdownSeconds(
                  quota.resetAt,
                  quota.resetAfterSeconds,
                  account.dashboard.refreshedAt,
                  now,
                );
                const color =
                  quota.remainingPercent <= 20
                    ? 'error'
                    : quota.remainingPercent <= 45
                      ? 'warning'
                      : 'success';
                return (
                  <Stack key={quota.id} spacing={0.75}>
                    <Stack direction="row" sx={{ justifyContent: 'space-between', alignItems: 'baseline' }}>
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        {label}
                      </Typography>
                      <Typography variant="subtitle2" color={`${color}.main`}>
                        {quota.remainingPercent}%
                      </Typography>
                    </Stack>
                    <LinearProgress
                      variant="determinate"
                      value={quota.remainingPercent}
                      color={color}
                      aria-label={`${label}: ${quota.remainingPercent}%`}
                    />
                    <Typography variant="caption" color="text.secondary">
                      {countdown !== null ? `${minuteDuration(countdown, locale.language)} · ` : ''}
                      {t('quota.resetAt', { date: formatDateTime(quota.resetAt, locale.language) })}
                    </Typography>
                  </Stack>
                );
              })
            : !(account.dashboard.extraCredits ?? details?.extraCredits) && (
                <Typography variant="body2" color="text.secondary">
                  {t('feature.accountUi.noQuotas')}
                </Typography>
              )}
          {Number.isFinite(subscriptionEnd) && (
            <Alert
              severity={subscriptionPending ? 'warning' : 'success'}
              variant="outlined"
              icon={<EventAvailableOutlinedIcon fontSize="small" />}
            >
              <Typography variant="body2" sx={{ fontWeight: 600 }}>
                {t('feature.accountUi.subscription')} ·{' '}
                {subscriptionPending
                  ? t('feature.accountUi.verifySubscription')
                  : t('feature.accountUi.daysRemaining', { count: subscriptionDays })}
              </Typography>
              <Typography variant="body2">
                {recordDate(details?.subscriptionEndsAt, locale.language)}
              </Typography>
              <Tooltip
                title={
                  details?.subscriptionCheckedAt
                    ? t('feature.accountUi.subscriptionChecked', {
                        date: recordDate(details.subscriptionCheckedAt, locale.language),
                      })
                    : t('feature.accountUi.subscriptionUnchecked')
                }
              >
                <Typography variant="caption" color="text.secondary">
                  {t('feature.accountUi.credentialRecord')}
                  {details?.subscriptionCheckedAt
                    ? ` · ${formatDateTime(details.subscriptionCheckedAt, locale.language)}`
                    : ''}
                </Typography>
              </Tooltip>
            </Alert>
          )}
          {details?.resetCredits && (
            <ResetCreditBadge
              accountId={account.id}
              available={details.resetCredits.available}
              applicable={details.resetCredits.applicable}
              now={now}
            />
          )}
          <Box>
            <Typography variant="caption" color="text.secondary" sx={{ display: 'block' }}>
              {account.dashboard.refreshedAt
                ? t('feature.accountUi.refreshed', {
                    date: recordDate(account.dashboard.refreshedAt, locale.language),
                  })
                : t('feature.accountUi.noRefresh')}
            </Typography>
            {!account.enabled && (
              <Typography variant="caption" color="text.secondary">
                {t('feature.paused')}
              </Typography>
            )}
          </Box>
          <Button
            size="small"
            endIcon={expanded ? <ExpandLessRoundedIcon /> : <ExpandMoreRoundedIcon />}
            aria-expanded={expanded}
            aria-controls={`details-${account.id}`}
            onClick={() => setExpanded(!expanded)}
            sx={{ alignSelf: 'flex-start' }}
          >
            {t('feature.accountUi.details')}
          </Button>
          <Collapse in={expanded} unmountOnExit id={`details-${account.id}`}>
            <Stack spacing={1}>
              <DetailRow label={t('feature.accountUi.name')} value={details?.name} />
              <DetailRow
                label={t('feature.accountUi.provider')}
                value={
                  details?.loginProvider
                    ? (providerLabels[details.loginProvider] ?? details.loginProvider)
                    : null
                }
              />
              <DetailRow label={t('feature.accountUi.userId')} value={details?.userId} copy shorten />
              <DetailRow label={t('feature.accountUi.accountId')} value={details?.accountId} copy shorten />
              <DetailRow
                label={t('feature.accountUi.tokenExpiry')}
                value={account.expiresAt ? recordDate(account.expiresAt, locale.language) : null}
              />
              <DetailRow label={t('feature.auth')} value={accountError(account.authStatus)} />
              <Typography variant="caption" color="text.secondary">
                {t(account.boundToCodex ? 'feature.bound' : 'feature.managed')} ·{' '}
                {t(account.canRefresh ? 'feature.refreshable' : 'feature.noRefresh')}
              </Typography>
              <DetailRow label={t('feature.accountUi.managedPath')} value={details?.managedAuthPath} copy />
              <DetailRow
                label={t('feature.accountUi.identitySource')}
                value={
                  details?.email
                    ? t(
                        details.identitySource === 'usage'
                          ? 'feature.accountUi.sourceUsage'
                          : 'feature.accountUi.sourceCredentials',
                      )
                    : null
                }
              />
              <DetailRow
                label={t('feature.accountUi.planSource')}
                value={
                  details?.planType
                    ? t(
                        details.planSource === 'usage'
                          ? 'feature.accountUi.sourceUsage'
                          : 'feature.accountUi.sourceCredentials',
                      )
                    : null
                }
              />
              <DetailRow
                label={t('feature.accountUi.recordTime')}
                value={
                  details?.credentialObservedAt
                    ? recordDate(details.credentialObservedAt, locale.language)
                    : null
                }
              />
              <DetailRow
                label={t('feature.accountUi.usageTime')}
                value={details?.usageObservedAt ? recordDate(details.usageObservedAt, locale.language) : null}
              />
              <DetailRow
                label={t('feature.accountUi.subscriptionStart')}
                value={
                  details?.subscriptionStartedAt
                    ? recordDate(details.subscriptionStartedAt, locale.language)
                    : null
                }
              />
              <DetailRow
                label={t('feature.accountUi.extraCredits')}
                value={
                  details?.extraCredits
                    ? details.extraCredits.unlimited
                      ? t('feature.accountUi.unlimited')
                      : (details.extraCredits.balance ??
                        (details.extraCredits.hasCredits === null
                          ? null
                          : t(
                              details.extraCredits.hasCredits
                                ? 'feature.accountUi.available'
                                : 'feature.accountUi.unavailable',
                            )))
                    : null
                }
              />
              {!!details?.models.length && (
                <>
                  <Divider />
                  <Typography variant="caption" color="text.secondary">
                    {t('feature.accountUi.availability')}
                  </Typography>
                  {details.models.map((model) => (
                    <DetailRow
                      key={model.id}
                      label={model.id}
                      value={
                        model.available
                          ? t('feature.accountUi.available')
                          : model.availableAt
                            ? t('feature.accountUi.availableAt', {
                                date: recordDate(model.availableAt, locale.language),
                              })
                            : t('feature.accountUi.unavailable')
                      }
                    />
                  ))}
                </>
              )}
            </Stack>
          </Collapse>
          {feedback && (
            <Alert severity={feedback.failed ? 'error' : 'success'} onClose={() => setFeedback(null)}>
              {feedback.text}
            </Alert>
          )}
        </Stack>
      </CardContent>
      <Divider sx={{ mt: 'auto' }} />
      <CardActions sx={{ justifyContent: 'space-between', flexWrap: 'wrap', gap: 0.5 }}>
        <FormControlLabel
          sx={{ m: 0 }}
          slotProps={{ typography: { variant: 'body2' } }}
          label={t('feature.accountUi.monitor')}
          control={
            <Switch
              size="small"
              checked={account.enabled}
              disabled={busy}
              onChange={(_, enabled) => void run(() => accountActions.update({ ...account, enabled }))}
            />
          }
        />
        <Stack direction="row" spacing={0.5}>
          <Tooltip title={t('feature.accountUi.editNote')}>
            <span>
              <IconButton
                size="small"
                disabled={busy}
                aria-label={t('feature.accountUi.editNote')}
                onClick={() => {
                  setNote(accountNote(account) ?? '');
                  setEditing(!editing);
                }}
              >
                <EditOutlinedIcon fontSize="small" />
              </IconButton>
            </span>
          </Tooltip>
          <Tooltip title={t('feature.apply')}>
            <span>
              <IconButton
                size="small"
                disabled={busy}
                aria-label={t('feature.apply')}
                onClick={() => setConfirm('apply')}
              >
                <ComputerOutlinedIcon fontSize="small" />
              </IconButton>
            </span>
          </Tooltip>
          <Tooltip title={t('feature.remove')}>
            <span>
              <IconButton
                size="small"
                disabled={busy}
                aria-label={t('feature.remove')}
                onClick={() => setConfirm('remove')}
              >
                <DeleteOutlineRoundedIcon fontSize="small" />
              </IconButton>
            </span>
          </Tooltip>
        </Stack>
      </CardActions>
      <Dialog open={confirm !== null} onClose={() => !busy && setConfirm(null)} maxWidth="xs" fullWidth>
        <DialogTitle>{t(confirm === 'apply' ? 'feature.apply' : 'feature.remove')}</DialogTitle>
        <DialogContent>
          <Typography>{t(confirm === 'apply' ? 'feature.applyConfirm' : 'feature.removeConfirm')}</Typography>
          {confirm === 'apply' && (
            <Typography variant="body2" color="text.secondary" sx={{ mt: 1 }}>
              {t('feature.switchNotice')}
            </Typography>
          )}
        </DialogContent>
        <DialogActions>
          <Button disabled={busy} onClick={() => setConfirm(null)}>
            {t('feature.cancel')}
          </Button>
          <Button
            disabled={busy}
            onClick={() =>
              void run(
                () =>
                  confirm === 'apply' ? accountActions.apply(account.id) : accountActions.remove(account.id),
                t(confirm === 'apply' ? 'feature.applied' : 'feature.saved'),
              )
            }
          >
            {t('feature.confirm')}
          </Button>
        </DialogActions>
      </Dialog>
    </Card>
  );
}
