import type { Settings } from '@/contracts/settings';
import type { usePreferences } from '@/features/settings/usePreferences';
import BugReportOutlinedIcon from '@mui/icons-material/BugReportOutlined';
import ContentCopyRoundedIcon from '@mui/icons-material/ContentCopyRounded';
import LaunchRoundedIcon from '@mui/icons-material/LaunchRounded';
import SystemUpdateAltRoundedIcon from '@mui/icons-material/SystemUpdateAltRounded';
import {
  Alert,
  Box,
  Button,
  CircularProgress,
  FormControlLabel,
  Paper,
  Stack,
  Switch,
  Typography,
} from '@mui/material';
import { useTranslation } from 'react-i18next';

import { formatDateTime } from '@/shared/format';

import type { FeedbackKey } from '@/features/settings/feedback';
import { useUpdates } from './useUpdates';
type UpdateControls = ReturnType<typeof useUpdates>;
const repositoryPageUrl = 'https://github.com/creamtea47/codex-usage-bar';
const releasePageUrl = 'https://github.com/creamtea47/codex-usage-bar/releases';
const issuePageUrl = 'https://github.com/creamtea47/codex-usage-bar/issues/new/choose';
const releaseVersionPattern = /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/;
function releaseNotesUrl(version: string | null | undefined): string {
  return version && releaseVersionPattern.test(version)
    ? `${releasePageUrl}/tag/v${version}`
    : releasePageUrl;
}

interface Props {
  settings: Settings;
  settingsControlsDisabled: boolean;
  updateSettings: ReturnType<typeof usePreferences>['updateSettings'];
  setIsUpdateDialogOpen: (open: boolean) => void;
  updateInfo: UpdateControls['updateInfo'];
  isCheckingUpdate: boolean;
  checkUpdate: UpdateControls['checkUpdate'];
  isCopyingDiagnostics: boolean;
  copyDiagnostics: () => Promise<void>;
  openExternal: (url: string, failureKey: FeedbackKey) => Promise<void>;
  language: 'en' | 'zh-CN';
}
export function AboutPage({
  settings,
  settingsControlsDisabled,
  updateSettings,
  setIsUpdateDialogOpen,
  updateInfo,
  isCheckingUpdate,
  checkUpdate,
  isCopyingDiagnostics,
  copyDiagnostics,
  openExternal,
  language,
}: Props) {
  const { t } = useTranslation();
  return (
    <Stack spacing={2.5}>
      <Box>
        <Typography component="h1" variant="h5" sx={{ fontWeight: 800 }}>
          {t('settings.about.title')}
        </Typography>
        <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
          {t('settings.about.subtitle')}
        </Typography>
      </Box>
      <Paper variant="outlined" sx={{ p: 2 }}>
        <Stack spacing={1.5}>
          <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
            {t('settings.about.updateTitle')}
          </Typography>
          <Typography variant="body2" color="text.secondary">
            {t('settings.about.currentVersion', {
              version: updateInfo?.currentVersion ?? t('common.unavailable'),
            })}
          </Typography>
          <FormControlLabel
            control={
              <Switch
                checked={settings.autoCheckUpdates}
                disabled={settingsControlsDisabled}
                onChange={(event) => void updateSettings({ autoCheckUpdates: event.target.checked })}
              />
            }
            label={t('settings.about.autoCheck')}
          />
          <Typography variant="caption" color="text.secondary" sx={{ pl: 4 }}>
            {t('settings.about.autoCheckDescription')}
          </Typography>
          {updateInfo?.checkedAt && (
            <Typography variant="caption" color="text.secondary">
              {t('settings.about.lastChecked', { date: formatDateTime(updateInfo.checkedAt, language) })}
            </Typography>
          )}
          {updateInfo?.updateAvailable && (
            <Alert
              severity="info"
              variant="outlined"
              action={
                <Button color="inherit" size="small" onClick={() => setIsUpdateDialogOpen(true)}>
                  {t('settings.about.view')}
                </Button>
              }
            >
              {t('settings.about.updateAvailable', { version: updateInfo.latestVersion })}
            </Alert>
          )}
          <Stack direction={{ xs: 'column', sm: 'row' }} spacing={1} sx={{ alignItems: 'flex-start' }}>
            <Button
              disabled={isCheckingUpdate}
              startIcon={
                isCheckingUpdate ? (
                  <CircularProgress color="inherit" size={16} />
                ) : (
                  <SystemUpdateAltRoundedIcon />
                )
              }
              variant="contained"
              onClick={() => void checkUpdate()}
            >
              {isCheckingUpdate ? t('settings.about.checking') : t('settings.about.check')}
            </Button>
            <Button
              startIcon={<LaunchRoundedIcon />}
              variant="outlined"
              onClick={() => void openExternal(repositoryPageUrl, 'settings.feedback.repositoryOpenFailed')}
            >
              {t('settings.about.repository')}
            </Button>
          </Stack>
        </Stack>
      </Paper>

      <Paper variant="outlined" sx={{ p: 2 }}>
        <Stack spacing={1.5}>
          <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
            {t('settings.about.diagnosticsTitle')}
          </Typography>
          <Typography variant="body2" color="text.secondary">
            {t('settings.about.diagnosticsDescription')}
          </Typography>
          <Stack direction={{ xs: 'column', sm: 'row' }} spacing={1}>
            <Button
              disabled={isCopyingDiagnostics}
              startIcon={
                isCopyingDiagnostics ? (
                  <CircularProgress color="inherit" size={16} />
                ) : (
                  <ContentCopyRoundedIcon />
                )
              }
              variant="contained"
              onClick={() => void copyDiagnostics()}
            >
              {t('settings.about.copyDiagnostics')}
            </Button>
            <Button
              startIcon={<BugReportOutlinedIcon />}
              variant="outlined"
              onClick={() => void openExternal(issuePageUrl, 'settings.feedback.issueOpenFailed')}
            >
              {t('settings.about.reportIssue')}
            </Button>
            <Button
              startIcon={<LaunchRoundedIcon />}
              variant="outlined"
              onClick={() =>
                void openExternal(
                  releaseNotesUrl(updateInfo?.currentVersion),
                  'settings.feedback.releaseNotesOpenFailed',
                )
              }
            >
              {t('settings.about.releaseNotes')}
            </Button>
          </Stack>
          <Alert severity="warning" variant="outlined">
            {t('settings.about.issueHint')}
          </Alert>
        </Stack>
      </Paper>
    </Stack>
  );
}
