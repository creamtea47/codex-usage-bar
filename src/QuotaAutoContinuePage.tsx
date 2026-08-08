import AutorenewRoundedIcon from '@mui/icons-material/AutorenewRounded';
import {
  Alert,
  Box,
  Button,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  FormControlLabel,
  Paper,
  Stack,
  Switch,
  Typography,
} from '@mui/material';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { formatDateTime } from './format';
import type { Language, QuotaAutoContinueStatus } from './types';

interface QuotaAutoContinuePageProps {
  enabled: boolean;
  status: QuotaAutoContinueStatus | null;
  language: Language;
  disabled: boolean;
  isChanging: boolean;
  isTesting: boolean;
  onSetEnabled: (enabled: boolean) => Promise<void>;
  onTest: () => Promise<void>;
}

/**
 * 真实请求操作始终先在本页完成二次确认；父组件只负责调用设置窗口限定的 Rust IPC。
 * 页面仅展示脱敏排期与结果类别，不接收账号、Token 或模型回复。
 */
export default function QuotaAutoContinuePage({
  enabled,
  status,
  language,
  disabled,
  isChanging,
  isTesting,
  onSetEnabled,
  onTest,
}: QuotaAutoContinuePageProps) {
  const { t } = useTranslation();
  const [confirmAction, setConfirmAction] = useState<'enable' | 'test' | null>(null);
  const busy = disabled || isChanging || isTesting;
  const phase = status?.phase ?? (enabled ? 'waitingForWeeklyWindow' : 'disabled');
  const latestResult = status?.lastErrorCode
    ? t(`settings.quotaAutoContinue.errors.${status.lastErrorCode}`)
    : status?.phase === 'sentAwaitingConfirmation'
      ? t('settings.quotaAutoContinue.sentAwaitingConfirmation')
      : status?.lastSuccessAt
        ? t('settings.quotaAutoContinue.successAt', {
            date: formatDateTime(status.lastSuccessAt, language),
          })
        : t('common.unavailable');

  const confirm = async () => {
    const action = confirmAction;
    setConfirmAction(null);
    if (action === 'enable') await onSetEnabled(true);
    if (action === 'test') await onTest();
  };

  return (
    <>
      <Stack spacing={2.5}>
        <Box>
          <Typography component="h1" variant="h5" sx={{ fontWeight: 800 }}>
            {t('settings.quotaAutoContinue.title')}
          </Typography>
          <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
            {t('settings.quotaAutoContinue.subtitle')}
          </Typography>
        </Box>

        <Paper variant="outlined" sx={{ p: 2 }}>
          <Stack spacing={1.25}>
            <FormControlLabel
              control={
                <Switch
                  checked={status?.enabled ?? enabled}
                  disabled={busy}
                  onChange={(event) => {
                    if (event.target.checked) setConfirmAction('enable');
                    else void onSetEnabled(false);
                  }}
                />
              }
              label={t('settings.quotaAutoContinue.enabled')}
            />
            <Typography variant="caption" color="text.secondary" sx={{ pl: 4 }}>
              {t('settings.quotaAutoContinue.enabledDescription')}
            </Typography>
            <Alert severity="warning" variant="outlined">
              {t('settings.quotaAutoContinue.realRequestWarning')}
            </Alert>
          </Stack>
        </Paper>

        <Paper variant="outlined" sx={{ p: 2 }}>
          <Stack spacing={1.25} divider={<Divider flexItem />}>
            <StatusRow
              label={t('settings.quotaAutoContinue.currentStatus')}
              value={t(`settings.quotaAutoContinue.phases.${phase}`)}
            />
            <StatusRow
              label={t('settings.quotaAutoContinue.targetResetAt')}
              value={formatDateTime(status?.targetResetAt ?? null, language)}
            />
            <StatusRow
              label={t('settings.quotaAutoContinue.nextAttemptAt')}
              value={formatDateTime(status?.nextAttemptAt ?? null, language)}
            />
            <StatusRow
              label={t('settings.quotaAutoContinue.attemptedCount')}
              value={t('settings.quotaAutoContinue.attemptedValue', { count: status?.attemptedCount ?? 0 })}
            />
            <StatusRow label={t('settings.quotaAutoContinue.latestResult')} value={latestResult} />
            {status?.selectedModel && (
              <StatusRow label={t('settings.quotaAutoContinue.selectedModel')} value={status.selectedModel} />
            )}
          </Stack>
        </Paper>

        <Paper variant="outlined" sx={{ p: 2 }}>
          <Stack spacing={1.25}>
            <Button
              startIcon={isTesting ? <CircularProgress color="inherit" size={16} /> : <AutorenewRoundedIcon />}
              variant="contained"
              disabled={busy}
              onClick={() => setConfirmAction('test')}
            >
              {isTesting ? t('settings.quotaAutoContinue.testing') : t('settings.quotaAutoContinue.test')}
            </Button>
            <Typography variant="caption" color="text.secondary">
              {t('settings.quotaAutoContinue.testDescription')}
            </Typography>
          </Stack>
        </Paper>

        <Alert severity="info" variant="outlined">
          {t('settings.quotaAutoContinue.runtimeNotice')}
        </Alert>
      </Stack>

      <Dialog open={confirmAction !== null} onClose={() => !busy && setConfirmAction(null)} maxWidth="xs" fullWidth>
        <DialogTitle>
          {confirmAction === 'enable'
            ? t('settings.quotaAutoContinue.enableConfirmTitle')
            : t('settings.quotaAutoContinue.testConfirmTitle')}
        </DialogTitle>
        <DialogContent dividers>
          <Typography variant="body2">
            {confirmAction === 'enable'
              ? t('settings.quotaAutoContinue.enableConfirmBody')
              : t('settings.quotaAutoContinue.testConfirmBody')}
          </Typography>
        </DialogContent>
        <DialogActions>
          <Button disabled={busy} onClick={() => setConfirmAction(null)}>
            {t('common.cancel')}
          </Button>
          <Button disabled={busy} color="warning" variant="contained" onClick={() => void confirm()}>
            {t('settings.quotaAutoContinue.confirm')}
          </Button>
        </DialogActions>
      </Dialog>
    </>
  );
}

function StatusRow({ label, value }: { label: string; value: string }) {
  return (
    <Stack direction={{ xs: 'column', sm: 'row' }} spacing={0.5} sx={{ justifyContent: 'space-between' }}>
      <Typography variant="body2" color="text.secondary">
        {label}
      </Typography>
      <Typography variant="body2" sx={{ fontWeight: 650, overflowWrap: 'anywhere' }}>
        {value}
      </Typography>
    </Stack>
  );
}
