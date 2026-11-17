import {
  Alert,
  Box,
  Button,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Grid,
  IconButton,
  Paper,
  Stack,
  Tooltip,
  Typography,
} from '@mui/material';
import FileUploadOutlinedIcon from '@mui/icons-material/FileUploadOutlined';
import RestoreRoundedIcon from '@mui/icons-material/RestoreRounded';
import RefreshRoundedIcon from '@mui/icons-material/RefreshRounded';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { accountActions } from './accountActions';
import { useAccounts, refreshAccounts } from './accountStore';
import { accountError } from './accountError';
import type { Account, CodexLoginSource } from '@/contracts/accounts';
import { formatDateTime } from '@/shared/format';
import { useForecastClock } from '@/features/usage/forecastPresentation';

import { accountTitle } from './accountPresentation';
import { AccountCard } from './AccountCard';
import { CopyButton } from './CopyButton';
function LoginSource({ source, accounts }: { source?: CodexLoginSource; accounts: Account[] }) {
  const { t, i18n } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const matched = accounts.find((a) => a.id === source?.matchedAccountId);
  const labels = {
    matched: 'feature.accountUi.sourceMatched',
    unmanaged: 'feature.accountUi.sourceUnmanaged',
    missing: 'feature.accountUi.sourceMissing',
    invalid: 'feature.accountUi.sourceInvalid',
    unsupportedStore: 'feature.accountUi.sourceUnsupported',
    unavailable: 'feature.accountUi.sourceUnavailable',
  } as const;
  return (
    <Paper variant="outlined" sx={{ p: 2 }}>
      <Stack spacing={0.75}>
        <Stack direction="row" spacing={1} sx={{ alignItems: 'center', justifyContent: 'space-between' }}>
          <Typography variant="subtitle2">{t('feature.accountUi.sourceTitle')}</Typography>
          <Tooltip title={t('feature.accountUi.recheck')}>
            <span>
              <IconButton
                size="small"
                disabled={busy}
                aria-label={t('feature.accountUi.recheck')}
                onClick={() => {
                  setBusy(true);
                  setError(false);
                  void refreshAccounts()
                    .catch(() => setError(true))
                    .finally(() => setBusy(false));
                }}
              >
                <RefreshRoundedIcon fontSize="small" />
              </IconButton>
            </span>
          </Tooltip>
        </Stack>
        <Stack direction="row" sx={{ flexWrap: 'wrap', gap: 1, alignItems: 'center' }}>
          <Chip
            size="small"
            variant="outlined"
            color={source?.status === 'matched' ? 'success' : 'default'}
            label={t(labels[source?.status ?? 'unavailable'], { storage: source?.storage ?? '—' })}
          />
          {(matched || source?.email) && (
            <Typography variant="body2" sx={{ overflowWrap: 'anywhere' }}>
              {matched ? accountTitle(matched) : source?.email}
            </Typography>
          )}
        </Stack>
        {source?.path && (
          <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
            <Typography variant="body2" sx={{ minWidth: 0, flex: 1, overflowWrap: 'anywhere' }}>
              {source.path}
            </Typography>
            <CopyButton value={source.path} label={t('feature.accountUi.loginPath')} />
          </Stack>
        )}
        {source?.bindingMatches === false && (
          <Typography variant="caption" color="warning.main">
            {t('feature.accountUi.bindingChanged')}
          </Typography>
        )}
        <Typography variant="caption" color="text.secondary">
          {t('feature.accountUi.sourceHint')}
          {source
            ? ` ${t('feature.accountUi.checked', { date: formatDateTime(source.checkedAt, i18n.language) })}`
            : ''}
        </Typography>
        {error && <Alert severity="error">{t('feature.failed')}</Alert>}
      </Stack>
    </Paper>
  );
}

export default function AccountsPage() {
  const { t } = useTranslation();
  const { accounts, selectedId, codexLogin } = useAccounts();
  const now = useForecastClock();
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [signingIn, setSigningIn] = useState(false);
  useEffect(() => {
    let mounted = true;
    const stop = accountActions.onLoginFinished(() => {
      if (mounted) setSigningIn(false);
    });
    void stop
      .then(() => accountActions.loginPending())
      .then((active) => {
        if (mounted && active) setSigningIn(true);
      })
      .catch(() => {});
    return () => {
      mounted = false;
      void stop.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);
  const signIn = async () => {
    setSigningIn(true);
    try {
      await accountActions.login();
      setMessage(t('feature.loginDone'));
    } catch (error) {
      setMessage(accountError(error));
    } finally {
      setSigningIn(false);
    }
  };
  const [restore, setRestore] = useState(false);
  useEffect(() => {
    void refreshAccounts().catch(() => setMessage(t('feature.failed')));
  }, [t]);
  return (
    <Stack spacing={2.5}>
      <Stack
        direction="row"
        sx={{ justifyContent: 'space-between', alignItems: 'center', flexWrap: 'wrap', gap: 1 }}
      >
        <Box>
          <Typography variant="h5" component="h1" sx={{ fontWeight: 800 }}>
            {t('feature.accounts')}
          </Typography>
          <Typography variant="body2" color="text.secondary">
            {t('feature.accountUi.summary', {
              count: accounts.length,
              enabled: accounts.filter((a) => a.enabled).length,
            })}
          </Typography>
        </Box>
        <Stack direction="row" spacing={0.5}>
          {signingIn ? (
            <Button
              size="small"
              onClick={() => void accountActions.cancelLogin().catch((e) => setMessage(accountError(e)))}
            >
              {t('feature.cancelLogin')}
            </Button>
          ) : (
            <Button variant="contained" size="small" disabled={busy} onClick={() => void signIn()}>
              {t('feature.loginImport')}
            </Button>
          )}
          <Button
            variant="outlined"
            size="small"
            startIcon={<FileUploadOutlinedIcon />}
            disabled={busy || signingIn}
            onClick={() => {
              setBusy(true);
              void accountActions
                .import()
                .then((r) =>
                  setMessage(
                    `${t('feature.imported', { count: r.imported })}${r.errors.length ? ` · ${r.errors.map(accountError).join(' · ')}` : ''}`,
                  ),
                )
                .catch((e) => setMessage(accountError(e)))
                .finally(() => setBusy(false));
            }}
          >
            {t('feature.import')}
          </Button>
          <Tooltip title={t('feature.restore')}>
            <span>
              <IconButton disabled={busy} aria-label={t('feature.restore')} onClick={() => setRestore(true)}>
                <RestoreRoundedIcon />
              </IconButton>
            </span>
          </Tooltip>
        </Stack>
      </Stack>
      <LoginSource source={codexLogin} accounts={accounts} />
      {message && (
        <Alert severity="info" onClose={() => setMessage(null)}>
          {message}
        </Alert>
      )}
      {!accounts.length && <Alert severity="info">{t('feature.empty')}</Alert>}
      <Grid container spacing={2}>
        {accounts.map((account) => (
          <Grid key={account.id} size={{ xs: 12, lg: 6 }} sx={{ minWidth: 0 }}>
            <AccountCard
              account={account}
              selected={account.id === selectedId}
              codexMatched={account.id === codexLogin?.matchedAccountId}
              now={now}
            />
          </Grid>
        ))}
      </Grid>
      <Dialog open={restore} onClose={() => !busy && setRestore(false)}>
        <DialogTitle>{t('feature.restore')}</DialogTitle>
        <DialogContent>{t('feature.switchNotice')}</DialogContent>
        <DialogActions>
          <Button disabled={busy} onClick={() => setRestore(false)}>
            {t('feature.cancel')}
          </Button>
          <Button
            disabled={busy}
            onClick={() => {
              setBusy(true);
              void accountActions
                .restore()
                .then(() => setMessage(t('feature.restored')))
                .catch((e) => setMessage(accountError(e)))
                .finally(() => {
                  setBusy(false);
                  setRestore(false);
                });
            }}
          >
            {t('feature.confirm')}
          </Button>
        </DialogActions>
      </Dialog>
    </Stack>
  );
}
