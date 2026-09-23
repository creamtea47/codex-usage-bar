import { ResetCreditBadge } from './ResetCreditBadge';
import AccountCircleOutlinedIcon from '@mui/icons-material/AccountCircleOutlined';
import { Alert, Autocomplete, Avatar, Box, Button, Card, CardActions, CardContent, CardHeader, Chip, Collapse, Dialog, DialogActions, DialogContent, DialogTitle, Divider, FormControlLabel, Grid, IconButton, LinearProgress, ListItemText, Menu, MenuItem, Paper, Snackbar, Stack, Switch, TextField, Tooltip, Typography } from '@mui/material';
import ContentCopyRoundedIcon from '@mui/icons-material/ContentCopyRounded';
import CheckRoundedIcon from '@mui/icons-material/CheckRounded';
import EditOutlinedIcon from '@mui/icons-material/EditOutlined';
import ComputerOutlinedIcon from '@mui/icons-material/ComputerOutlined';
import DeleteOutlineRoundedIcon from '@mui/icons-material/DeleteOutlineRounded';
import ExpandMoreRoundedIcon from '@mui/icons-material/ExpandMoreRounded';
import ExpandLessRoundedIcon from '@mui/icons-material/ExpandLessRounded';
import EventAvailableOutlinedIcon from '@mui/icons-material/EventAvailableOutlined';
import FileUploadOutlinedIcon from '@mui/icons-material/FileUploadOutlined';
import RestoreRoundedIcon from '@mui/icons-material/RestoreRounded';
import RefreshRoundedIcon from '@mui/icons-material/RefreshRounded';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { accountsBridge, useAccounts, accountError, refreshAccounts } from './accountsBridge';
import type { Account, CodexLoginSource, ModelOption } from './accountTypes';
import { formatDateTime, formatDuration, resetCountdownSeconds } from './format';
import { minuteDuration, useForecastClock } from './forecastPresentation';

function planLabel(value: string | null | undefined): string | null {
  if (!value) return null;
  const labels: Record<string, string> = { free: 'Free', plus: 'Plus', pro: 'Pro', prolite: 'Pro Lite', team: 'Team', business: 'Business', enterprise: 'Enterprise', edu: 'Edu' };
  return labels[value] ?? value;
}

function accountTitle(account: Account): string {
  return account.details?.email ?? account.details?.name ?? account.label;
}

/** 自动生成的旧标签仍保存在配置中，只在展示上避免与身份信息重复。 */
function accountNote(account: Account): string | null {
  const email = account.details?.email;
  const masked = email ? `${email[0]}***@${email.split('@').at(-1)}` : null;
  return [email, masked, `Account ${account.id.slice(0, 8)}`].includes(account.label) ? null : account.label;
}

function recordDate(value: string | null | undefined, language: string): string {
  if (!value || !Number.isFinite(Date.parse(value))) return '—';
  return new Intl.DateTimeFormat(language, { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false }).format(new Date(value));
}

function CopyButton({ value, label }: { value: string; label: string }) {
  const { t } = useTranslation();
  const [state, setState] = useState<'idle' | 'copied' | 'failed'>('idle');
  const title = state === 'copied' ? t('feature.accountUi.copied') : state === 'failed' ? t('feature.accountUi.copyFailed') : t('feature.accountUi.copy', { label });
  return <Tooltip title={title}><IconButton size="small" aria-label={t('feature.accountUi.copy', { label })} color={state === 'failed' ? 'error' : 'default'}
    onClick={() => { void writeText(value).then(() => setState('copied')).catch(() => setState('failed')); }}>
    {state === 'copied' ? <CheckRoundedIcon fontSize="small" /> : <ContentCopyRoundedIcon fontSize="small" />}
  </IconButton></Tooltip>;
}

function DetailRow({ label, value, copy = false, shorten = false }: { label: string; value?: string | null; copy?: boolean; shorten?: boolean }) {
  if (!value) return null;
  const display = shorten && value.length > 22 ? `${value.slice(0, 12)}…${value.slice(-6)}` : value;
  return <Stack direction="row" spacing={1} sx={{ alignItems: 'center', minWidth: 0 }}>
    <Typography variant="caption" color="text.secondary" sx={{ flexShrink: 0 }}>{label}</Typography>
    <Tooltip title={value}><Typography variant="body2" sx={{ flex: 1, minWidth: 0, textAlign: 'right', overflowWrap: 'anywhere' }}>{display}</Typography></Tooltip>
    {copy && <CopyButton value={value} label={label} />}
  </Stack>;
}

/** sidebar 展示完整身份；主卡保持原有布局、备注与脱敏数据。 */
export function AccountSelector({ sidebar = false }: { sidebar?: boolean }) {
  const { t } = useTranslation();
  const data = useAccounts();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  if (!data.accounts.length) return null;
  return <Box data-no-drag sx={{ py: sidebar ? 0 : 1 }}>
    <TextField select size="small" fullWidth label={t('feature.selected')} value={data.selectedId ?? ''} disabled={busy}
      slotProps={{ select: { renderValue: (value) => {
        const account = data.accounts.find((a) => a.id === value);
        if (!account) return '';
        return <Tooltip title={sidebar ? accountTitle(account) : account.label}><Typography variant="body2" noWrap>{sidebar ? accountTitle(account) : account.label}</Typography></Tooltip>;
      }, MenuProps: { slotProps: { paper: { sx: { maxWidth: 420 } } } } } }}
      onChange={(event) => { setBusy(true); setError(null); void accountsBridge.select(event.target.value).catch((e) => setError(accountError(e))).finally(() => setBusy(false)); }}>
      {data.accounts.map((a) => <MenuItem key={a.id} value={a.id} sx={{ gap: 1, minWidth: 0 }}>
        <ListItemText primary={sidebar ? accountTitle(a) : a.label}
          secondary={sidebar ? [accountNote(a), planLabel(a.details?.planType ?? a.dashboard.planLabel)].filter(Boolean).join(' · ') : undefined}
          slotProps={{ primary: { noWrap: true }, secondary: { noWrap: true } }} />
        {sidebar && data.codexLogin?.matchedAccountId === a.id && <Tooltip title={t('feature.accountUi.codexFile')}><ComputerOutlinedIcon color="primary" fontSize="small" /></Tooltip>}
      </MenuItem>)}
    </TextField>
    {error && <Alert severity="error" sx={{ mt: 1 }}>{error}</Alert>}
  </Box>;
}

/** 标题栏只保留图标；菜单和错误提示不参与主卡高度计算。 */
export function AccountSwitcher() {
  const { t } = useTranslation();
  const data = useAccounts();
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const selected = data.accounts.find((a) => a.id === data.selectedId);
  const identity = (a: Account) => a.dashboard.accountEmailMasked ?? a.label;
  const title = selected ? t('feature.compact.currentAccount', { account: identity(selected) }) : t('feature.compact.noAccounts');
  return <>
    <Tooltip title={title}><span data-no-drag><IconButton size="small" color="inherit" aria-label={t('feature.compact.switchAccount')} aria-haspopup="menu" aria-expanded={!!anchor} aria-controls={anchor ? 'account-switch-menu' : undefined}
      disabled={busy || data.accounts.length < 2} onClick={(e) => setAnchor(e.currentTarget)}><AccountCircleOutlinedIcon fontSize="small" /></IconButton></span></Tooltip>
    <Menu id="account-switch-menu" data-no-drag anchorEl={anchor} open={!!anchor} onClose={() => !busy && setAnchor(null)} slotProps={{ list: { 'aria-label': t('feature.compact.switchAccount') }, paper: { sx: { maxWidth: 320 } } }}>
      {data.accounts.map((account) => <MenuItem key={account.id} role="menuitemradio" aria-checked={account.id === data.selectedId} selected={account.id === data.selectedId} disabled={busy} sx={{ gap: 1 }}
        onClick={() => {
          if (account.id === data.selectedId) { setAnchor(null); return; }
          setBusy(true); setError(null);
          void accountsBridge.select(account.id).then(() => setAnchor(null)).catch((e) => { setError(accountError(e)); setAnchor(null); }).finally(() => setBusy(false));
        }}>
        <ListItemText primary={identity(account)} secondary={[account.label !== identity(account) ? account.label : null, account.dashboard.planLabel].filter(Boolean).join(' · ')} slotProps={{ primary: { noWrap: true }, secondary: { noWrap: true } }} />
        {account.id === data.selectedId && <CheckRoundedIcon fontSize="small" />}
      </MenuItem>)}
    </Menu>
    <Snackbar open={!!error} autoHideDuration={5000} onClose={() => setError(null)}><Alert severity="error" onClose={() => setError(null)}>{error}</Alert></Snackbar>
  </>;
}

export function ModelPicker({ account, disabled = false }: { account: Account; disabled?: boolean }) {
  const { t } = useTranslation();
  const [models, setModels] = useState<ModelOption[]>([]);
  const [model, setModel] = useState(account.model ?? '');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  async function load() {
    setBusy(true); setError(null);
    try { setModels(await accountsBridge.models(account.id)); }
    catch { setError(t('feature.noModels')); }
    finally { setBusy(false); }
  }
  return <Stack spacing={1.25}>
    <Typography variant="subtitle2">{t('feature.model')}</Typography>
    <Autocomplete freeSolo options={models.map((v) => v.id)} inputValue={model} value={model || null}
      disabled={busy || disabled} onInputChange={(_, value) => { setModel(value); setSaved(false); }}
      renderInput={(params) => <TextField {...params} label={t('feature.modelId')} placeholder={t('feature.auto')} size="small" />} />
    <Stack direction="row" spacing={1} sx={{ flexWrap: 'wrap', gap: 1 }}>
      <Button disabled={busy || disabled} onClick={() => void load()}>{t('feature.refreshModels')}</Button>
      <Button disabled={busy || disabled} onClick={() => { setModel(''); setSaved(false); }}>{t('feature.auto')}</Button>
      <Button variant="outlined" disabled={busy || disabled} onClick={() => {
        setBusy(true); setError(null);
        void accountsBridge.update({ ...account, model: model.trim() || null }).then(() => setSaved(true)).catch((e) => setError(accountError(e))).finally(() => setBusy(false));
      }}>{t('feature.save')}</Button>
    </Stack>
    <Typography variant="caption" color="text.secondary">{t('feature.modelHelp')}</Typography>
    {error && <Alert severity="warning">{error}</Alert>}
    {saved && <Alert severity="success">{t('feature.saved')}</Alert>}
  </Stack>;
}

/** 身份、额度和订阅优先呈现；认证细节折叠，底部只保留已有操作。 */
export function AccountCard({ account, selected, codexMatched, now }: { account: Account; selected: boolean; codexMatched: boolean; now: number }) {
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
  const subheader = [details?.name !== title ? details?.name : null, accountNote(account)].filter(Boolean).join(' · ');
  const subscriptionEnd = Date.parse(details?.subscriptionEndsAt ?? '');
  const subscriptionPending = Number.isFinite(subscriptionEnd) && subscriptionEnd <= now;
  const subscriptionDays = Math.max(0, Math.ceil((subscriptionEnd - now) / 86400000));
  const stale = account.dashboard.status === 'stale' || (account.dashboard.status === 'error' && !!details?.usageObservedAt);
  const run = async (action: () => Promise<unknown>, success: string = t('feature.saved')) => {
    setBusy(true); setFeedback(null);
    try { await action(); setFeedback({ text: success, failed: false }); }
    catch (error) { setFeedback({ text: accountError(error), failed: true }); }
    finally { setBusy(false); setConfirm(null); }
  };
  const issue = account.authStatus !== 'ready' ? accountError(account.authStatus)
    : account.dashboard.message ? t(`dashboardError.${account.dashboard.message}`) : null;
  const providerLabels: Record<string, string> = { google: 'Google', apple: 'Apple', microsoft: 'Microsoft', auth0: 'OpenAI', email: 'Email' };
  return <Card variant="outlined" component="article" aria-label={title} sx={{ height: '100%', display: 'flex', flexDirection: 'column', minWidth: 0 }}>
    <CardHeader avatar={<Avatar sx={{ bgcolor: 'primary.main', color: 'primary.contrastText' }}>{title[0]?.toUpperCase()}</Avatar>}
      title={<Tooltip title={title}><Typography variant="subtitle1" noWrap sx={{ fontWeight: 700 }}>{title}</Typography></Tooltip>}
      subheader={subheader ? <Tooltip title={subheader}><Typography variant="body2" color="text.secondary" noWrap>{subheader}</Typography></Tooltip> : undefined}
      action={plan ? <Tooltip title={`${plan} · ${details?.planSource === 'credentials' ? t('feature.accountUi.sourceCredentials') : t('feature.accountUi.sourceUsage')}`}><Chip label={plan} size="small" color="primary" variant="outlined" sx={{ maxWidth: 128 }} /></Tooltip> : undefined}
      slotProps={{ content: { sx: { minWidth: 0 } }, action: { sx: { alignSelf: 'center', m: 0 } } }} />
    <CardContent sx={{ pt: 0 }}><Stack spacing={2}>
      {(selected || codexMatched || issue || stale) && <Stack direction="row" sx={{ gap: 0.75, flexWrap: 'wrap' }}>
        {selected && <Chip size="small" variant="outlined" label={t('feature.accountUi.viewing')} />}
        {codexMatched && <Chip size="small" color="success" variant="outlined" icon={<ComputerOutlinedIcon />} label={t('feature.accountUi.codexFile')} />}
        {issue && <Tooltip title={issue}><Chip size="small" color="warning" label={t(account.authStatus !== 'ready' ? 'feature.accountUi.authIssue' : 'feature.accountUi.refreshIssue')} /></Tooltip>}
        {stale && <Chip size="small" color="warning" variant="outlined" label={t('feature.accountUi.stale')} />}
      </Stack>}
      {editing && <Stack direction="row" spacing={0.5} sx={{ alignItems: 'center' }}>
        <TextField autoFocus size="small" fullWidth label={t('feature.note')} value={note} onChange={(e) => setNote(e.target.value)} disabled={busy} />
        <Button disabled={busy} onClick={() => void run(async () => {
          await accountsBridge.update({ ...account, label: note.trim() || account.dashboard.accountEmailMasked || `Account ${account.id.slice(0, 8)}` });
          setEditing(false);
        })}>{t('feature.save')}</Button>
        <Button disabled={busy} onClick={() => setEditing(false)}>{t('feature.cancel')}</Button>
      </Stack>}
      {account.dashboard.quotaWindows.length ? account.dashboard.quotaWindows.map((quota) => {
        const label = quota.label ?? (quota.fallbackLabel === 'weekly' ? t('quota.fallbackLabel.weekly') : quota.fallbackLabel === 'fiveHour' ? t('quota.fallbackLabel.fiveHour') : t('quota.fallbackLabel.window', { duration: formatDuration(quota.windowSeconds, locale.language) }));
        const countdown = resetCountdownSeconds(quota.resetAt, quota.resetAfterSeconds, account.dashboard.refreshedAt, now);
        const color = quota.remainingPercent <= 20 ? 'error' : quota.remainingPercent <= 45 ? 'warning' : 'success';
        return <Stack key={quota.id} spacing={0.75}>
          <Stack direction="row" sx={{ justifyContent: 'space-between', alignItems: 'baseline' }}>
            <Typography variant="body2" sx={{ fontWeight: 600 }}>{label}</Typography>
            <Typography variant="subtitle2" color={`${color}.main`}>{quota.remainingPercent}%</Typography>
          </Stack>
          <LinearProgress variant="determinate" value={quota.remainingPercent} color={color} aria-label={`${label}: ${quota.remainingPercent}%`} />
          <Typography variant="caption" color="text.secondary">{countdown !== null ? `${minuteDuration(countdown, locale.language)} · ` : ''}{t('quota.resetAt', { date: formatDateTime(quota.resetAt, locale.language) })}</Typography>
        </Stack>;
      }) : <Typography variant="body2" color="text.secondary">{t('feature.accountUi.noQuotas')}</Typography>}
      {Number.isFinite(subscriptionEnd) && <Alert severity={subscriptionPending ? 'warning' : 'success'} variant="outlined" icon={<EventAvailableOutlinedIcon fontSize="small" />}>
        <Typography variant="body2" sx={{ fontWeight: 600 }}>{t('feature.accountUi.subscription')} · {subscriptionPending ? t('feature.accountUi.verifySubscription') : t('feature.accountUi.daysRemaining', { count: subscriptionDays })}</Typography>
        <Typography variant="body2">{recordDate(details?.subscriptionEndsAt, locale.language)}</Typography>
        <Tooltip title={details?.subscriptionCheckedAt ? t('feature.accountUi.subscriptionChecked', { date: recordDate(details.subscriptionCheckedAt, locale.language) }) : t('feature.accountUi.subscriptionUnchecked')}>
          <Typography variant="caption" color="text.secondary">{t('feature.accountUi.credentialRecord')}{details?.subscriptionCheckedAt ? ` · ${formatDateTime(details.subscriptionCheckedAt, locale.language)}` : ''}</Typography>
        </Tooltip>
      </Alert>}
      {details?.resetCredits && <ResetCreditBadge accountId={account.id} available={details.resetCredits.available} applicable={details.resetCredits.applicable} now={now} />}
      <Box>
        <Typography variant="caption" color="text.secondary" sx={{ display: 'block' }}>{account.dashboard.refreshedAt ? t('feature.accountUi.refreshed', { date: recordDate(account.dashboard.refreshedAt, locale.language) }) : t('feature.accountUi.noRefresh')}</Typography>
        {!account.enabled && <Typography variant="caption" color="text.secondary">{t('feature.paused')}</Typography>}
      </Box>
      <Button size="small" endIcon={expanded ? <ExpandLessRoundedIcon /> : <ExpandMoreRoundedIcon />} aria-expanded={expanded} aria-controls={`details-${account.id}`} onClick={() => setExpanded(!expanded)} sx={{ alignSelf: 'flex-start' }}>{t('feature.accountUi.details')}</Button>
      <Collapse in={expanded} unmountOnExit id={`details-${account.id}`}><Stack spacing={1}>
        <DetailRow label={t('feature.accountUi.name')} value={details?.name} />
        <DetailRow label={t('feature.accountUi.provider')} value={details?.loginProvider ? providerLabels[details.loginProvider] ?? details.loginProvider : null} />
        <DetailRow label={t('feature.accountUi.userId')} value={details?.userId} copy shorten />
        <DetailRow label={t('feature.accountUi.accountId')} value={details?.accountId} copy shorten />
        <DetailRow label={t('feature.accountUi.tokenExpiry')} value={account.expiresAt ? recordDate(account.expiresAt, locale.language) : null} />
        <DetailRow label={t('feature.auth')} value={accountError(account.authStatus)} />
        <Typography variant="caption" color="text.secondary">{t(account.boundToCodex ? 'feature.bound' : 'feature.managed')} · {t(account.canRefresh ? 'feature.refreshable' : 'feature.noRefresh')}</Typography>
        <DetailRow label={t('feature.accountUi.managedPath')} value={details?.managedAuthPath} copy />
        <DetailRow label={t('feature.accountUi.identitySource')} value={details?.email ? t(details.identitySource === 'usage' ? 'feature.accountUi.sourceUsage' : 'feature.accountUi.sourceCredentials') : null} />
        <DetailRow label={t('feature.accountUi.planSource')} value={details?.planType ? t(details.planSource === 'usage' ? 'feature.accountUi.sourceUsage' : 'feature.accountUi.sourceCredentials') : null} />
        <DetailRow label={t('feature.accountUi.recordTime')} value={details?.credentialObservedAt ? recordDate(details.credentialObservedAt, locale.language) : null} />
        <DetailRow label={t('feature.accountUi.usageTime')} value={details?.usageObservedAt ? recordDate(details.usageObservedAt, locale.language) : null} />
        <DetailRow label={t('feature.accountUi.subscriptionStart')} value={details?.subscriptionStartedAt ? recordDate(details.subscriptionStartedAt, locale.language) : null} />
        <DetailRow label={t('feature.accountUi.extraCredits')} value={details?.extraCredits ? details.extraCredits.unlimited ? t('feature.accountUi.unlimited') : details.extraCredits.balance ?? (details.extraCredits.hasCredits === null ? null : t(details.extraCredits.hasCredits ? 'feature.accountUi.available' : 'feature.accountUi.unavailable')) : null} />
        {!!details?.models.length && <><Divider /><Typography variant="caption" color="text.secondary">{t('feature.accountUi.availability')}</Typography>{details.models.map((model) => <DetailRow key={model.id} label={model.id} value={model.available ? t('feature.accountUi.available') : model.availableAt ? t('feature.accountUi.availableAt', { date: recordDate(model.availableAt, locale.language) }) : t('feature.accountUi.unavailable')} />)}</>}
      </Stack></Collapse>
      {feedback && <Alert severity={feedback.failed ? 'error' : 'success'} onClose={() => setFeedback(null)}>{feedback.text}</Alert>}
    </Stack></CardContent>
    <Divider sx={{ mt: 'auto' }} />
    <CardActions sx={{ justifyContent: 'space-between', flexWrap: 'wrap', gap: 0.5 }}>
      <FormControlLabel sx={{ m: 0 }} slotProps={{ typography: { variant: 'body2' } }} label={t('feature.accountUi.monitor')} control={<Switch size="small" checked={account.enabled} disabled={busy} onChange={(_, enabled) => void run(() => accountsBridge.update({ ...account, enabled }))} />} />
      <Stack direction="row" spacing={0.5}>
        <Tooltip title={t('feature.accountUi.editNote')}><span><IconButton size="small" disabled={busy} aria-label={t('feature.accountUi.editNote')} onClick={() => { setNote(accountNote(account) ?? ''); setEditing(!editing); }}><EditOutlinedIcon fontSize="small" /></IconButton></span></Tooltip>
        <Tooltip title={t('feature.apply')}><span><IconButton size="small" disabled={busy} aria-label={t('feature.apply')} onClick={() => setConfirm('apply')}><ComputerOutlinedIcon fontSize="small" /></IconButton></span></Tooltip>
        <Tooltip title={t('feature.remove')}><span><IconButton size="small" disabled={busy} aria-label={t('feature.remove')} onClick={() => setConfirm('remove')}><DeleteOutlineRoundedIcon fontSize="small" /></IconButton></span></Tooltip>
      </Stack>
    </CardActions>
    <Dialog open={confirm !== null} onClose={() => !busy && setConfirm(null)} maxWidth="xs" fullWidth>
      <DialogTitle>{t(confirm === 'apply' ? 'feature.apply' : 'feature.remove')}</DialogTitle>
      <DialogContent><Typography>{t(confirm === 'apply' ? 'feature.applyConfirm' : 'feature.removeConfirm')}</Typography>{confirm === 'apply' && <Typography variant="body2" color="text.secondary" sx={{ mt: 1 }}>{t('feature.switchNotice')}</Typography>}</DialogContent>
      <DialogActions><Button disabled={busy} onClick={() => setConfirm(null)}>{t('feature.cancel')}</Button><Button disabled={busy} onClick={() => void run(() => confirm === 'apply' ? accountsBridge.apply(account.id) : accountsBridge.remove(account.id), t(confirm === 'apply' ? 'feature.applied' : 'feature.saved'))}>{t('feature.confirm')}</Button></DialogActions>
    </Dialog>
  </Card>;
}

function LoginSource({ source, accounts }: { source?: CodexLoginSource; accounts: Account[] }) {
  const { t, i18n } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const matched = accounts.find((a) => a.id === source?.matchedAccountId);
  const labels = { matched: 'feature.accountUi.sourceMatched', unmanaged: 'feature.accountUi.sourceUnmanaged', missing: 'feature.accountUi.sourceMissing', invalid: 'feature.accountUi.sourceInvalid', unsupportedStore: 'feature.accountUi.sourceUnsupported', unavailable: 'feature.accountUi.sourceUnavailable' } as const;
  return <Paper variant="outlined" sx={{ p: 2 }}><Stack spacing={0.75}>
    <Stack direction="row" spacing={1} sx={{ alignItems: 'center', justifyContent: 'space-between' }}>
      <Typography variant="subtitle2">{t('feature.accountUi.sourceTitle')}</Typography>
      <Tooltip title={t('feature.accountUi.recheck')}><span><IconButton size="small" disabled={busy} aria-label={t('feature.accountUi.recheck')} onClick={() => {
        setBusy(true); setError(false); void refreshAccounts().catch(() => setError(true)).finally(() => setBusy(false));
      }}><RefreshRoundedIcon fontSize="small" /></IconButton></span></Tooltip>
    </Stack>
    <Stack direction="row" sx={{ flexWrap: 'wrap', gap: 1, alignItems: 'center' }}>
      <Chip size="small" variant="outlined" color={source?.status === 'matched' ? 'success' : 'default'} label={t(labels[source?.status ?? 'unavailable'], { storage: source?.storage ?? '—' })} />
      {(matched || source?.email) && <Typography variant="body2" sx={{ overflowWrap: 'anywhere' }}>{matched ? accountTitle(matched) : source?.email}</Typography>}
    </Stack>
    {source?.path && <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}><Typography variant="body2" sx={{ minWidth: 0, flex: 1, overflowWrap: 'anywhere' }}>{source.path}</Typography><CopyButton value={source.path} label={t('feature.accountUi.loginPath')} /></Stack>}
    {source?.bindingMatches === false && <Typography variant="caption" color="warning.main">{t('feature.accountUi.bindingChanged')}</Typography>}
    <Typography variant="caption" color="text.secondary">{t('feature.accountUi.sourceHint')}{source ? ` ${t('feature.accountUi.checked', { date: formatDateTime(source.checkedAt, i18n.language) })}` : ''}</Typography>
    {error && <Alert severity="error">{t('feature.failed')}</Alert>}
  </Stack></Paper>;
}

export default function AccountsPage() {
  const { t } = useTranslation();
  const { accounts, selectedId, codexLogin } = useAccounts();
  const now = useForecastClock();
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [restore, setRestore] = useState(false);
  useEffect(() => { void refreshAccounts().catch(() => setMessage(t('feature.failed'))); }, [t]);
  return <Stack spacing={2.5}>
    <Stack direction="row" sx={{ justifyContent: 'space-between', alignItems: 'center', flexWrap: 'wrap', gap: 1 }}>
      <Box><Typography variant="h5" component="h1" sx={{ fontWeight: 800 }}>{t('feature.accounts')}</Typography><Typography variant="body2" color="text.secondary">{t('feature.accountUi.summary', { count: accounts.length, enabled: accounts.filter((a) => a.enabled).length })}</Typography></Box>
      <Stack direction="row" spacing={0.5}>
        <Button variant="contained" size="small" startIcon={<FileUploadOutlinedIcon />} disabled={busy} onClick={() => {
          setBusy(true); void accountsBridge.import().then((r) => setMessage(`${t('feature.imported', { count: r.imported })}${r.errors.length ? ` · ${r.errors.map(accountError).join(' · ')}` : ''}`)).catch((e) => setMessage(accountError(e))).finally(() => setBusy(false));
        }}>{t('feature.import')}</Button>
        <Tooltip title={t('feature.restore')}><span><IconButton disabled={busy} aria-label={t('feature.restore')} onClick={() => setRestore(true)}><RestoreRoundedIcon /></IconButton></span></Tooltip>
      </Stack>
    </Stack>
    <LoginSource source={codexLogin} accounts={accounts} />
    {message && <Alert severity="info" onClose={() => setMessage(null)}>{message}</Alert>}
    {!accounts.length && <Alert severity="info">{t('feature.empty')}</Alert>}
    <Grid container spacing={2}>{accounts.map((account) => <Grid key={account.id} size={{ xs: 12, lg: 6 }} sx={{ minWidth: 0 }}>
      <AccountCard account={account} selected={account.id === selectedId} codexMatched={account.id === codexLogin?.matchedAccountId} now={now} />
    </Grid>)}</Grid>
    <Dialog open={restore} onClose={() => !busy && setRestore(false)}><DialogTitle>{t('feature.restore')}</DialogTitle><DialogContent>{t('feature.switchNotice')}</DialogContent><DialogActions><Button disabled={busy} onClick={() => setRestore(false)}>{t('feature.cancel')}</Button><Button disabled={busy} onClick={() => {
      setBusy(true); void accountsBridge.restore().then(() => setMessage(t('feature.restored'))).catch((e) => setMessage(accountError(e))).finally(() => { setBusy(false); setRestore(false); });
    }}>{t('feature.confirm')}</Button></DialogActions></Dialog>
  </Stack>;
}
