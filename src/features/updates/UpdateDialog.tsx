import LaunchRoundedIcon from '@mui/icons-material/LaunchRounded';
import SystemUpdateAltRoundedIcon from '@mui/icons-material/SystemUpdateAltRounded';
import {
  Alert,
  Button,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  LinearProgress,
  Stack,
  Typography,
} from '@mui/material';
import { useTranslation } from 'react-i18next';

import { formatDateTime } from '@/shared/format';

import { useUpdates } from './useUpdates';
import type { FeedbackKey } from '@/features/settings/feedback';
const releasePageUrl = 'https://github.com/creamtea47/codex-usage-bar/releases';
type UpdateControls = ReturnType<typeof useUpdates>;
export function UpdateDialog({
  language,
  controls,
  openExternal,
}: {
  language: string;
  controls: UpdateControls;
  openExternal: (url: string, failureKey: FeedbackKey) => Promise<void>;
}) {
  const { t } = useTranslation();
  const {
    isUpdateDialogOpen,
    setIsUpdateDialogOpen,
    isInstallingUpdate,
    updateInfo,
    updateProgress,
    installUpdate,
  } = controls;
  const progressLabel = !updateProgress
    ? t('settings.updateDialog.preparing')
    : updateProgress.stage === 'verifying'
      ? t('settings.updateDialog.verifying')
      : updateProgress.stage === 'installing'
        ? t('settings.updateDialog.handingOff')
        : updateProgress.totalBytes && updateProgress.totalBytes > 0
          ? t('settings.updateDialog.downloadingPercent', {
              percent: Math.min(
                100,
                Math.round((updateProgress.downloadedBytes / updateProgress.totalBytes) * 100),
              ),
            })
          : t('settings.updateDialog.downloading');
  return (
    <Dialog
      fullWidth
      maxWidth="xs"
      open={isUpdateDialogOpen}
      onClose={() => !isInstallingUpdate && setIsUpdateDialogOpen(false)}
    >
      <DialogTitle>{t('settings.updateDialog.title')}</DialogTitle>
      <DialogContent dividers>
        <Stack spacing={1.25}>
          <Typography variant="h6" sx={{ fontWeight: 800 }}>
            {updateInfo?.updateAvailable
              ? t('settings.updateDialog.available', { version: updateInfo.latestVersion })
              : t('settings.updateDialog.current')}
          </Typography>
          <Typography variant="body2" color="text.secondary">
            {t('settings.updateDialog.versions', {
              current: updateInfo?.currentVersion ?? t('common.unavailable'),
              latest: updateInfo?.latestVersion ?? t('common.unavailable'),
            })}
          </Typography>
          {updateInfo?.checkedAt && (
            <Typography variant="body2" color="text.secondary">
              {t('settings.updateDialog.checkedAt', { date: formatDateTime(updateInfo.checkedAt, language) })}
            </Typography>
          )}
          {updateInfo?.updateAvailable && (
            <Alert severity="info" variant="outlined">
              {t('settings.updateDialog.installDescription')}
            </Alert>
          )}
          {isInstallingUpdate && (
            <Stack spacing={0.75}>
              <LinearProgress
                aria-label={progressLabel}
                variant={updateProgress?.totalBytes ? 'determinate' : 'indeterminate'}
                value={
                  updateProgress?.totalBytes
                    ? Math.min(100, (updateProgress.downloadedBytes / updateProgress.totalBytes) * 100)
                    : undefined
                }
              />
              <Typography variant="caption" color="text.secondary">
                {progressLabel}
              </Typography>
            </Stack>
          )}
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button disabled={isInstallingUpdate} onClick={() => setIsUpdateDialogOpen(false)}>
          {t('settings.updateDialog.close')}
        </Button>
        <Button
          disabled={isInstallingUpdate}
          endIcon={<LaunchRoundedIcon />}
          onClick={() => void openExternal(releasePageUrl, 'settings.feedback.releaseOpenFailed')}
        >
          {t('settings.updateDialog.releases')}
        </Button>
        {updateInfo?.updateAvailable && (
          <Button
            disabled={isInstallingUpdate}
            startIcon={
              isInstallingUpdate ? (
                <CircularProgress color="inherit" size={16} />
              ) : (
                <SystemUpdateAltRoundedIcon />
              )
            }
            variant="contained"
            onClick={() => void installUpdate()}
          >
            {isInstallingUpdate ? t('settings.updateDialog.installing') : t('settings.updateDialog.install')}
          </Button>
        )}
      </DialogActions>
    </Dialog>
  );
}
