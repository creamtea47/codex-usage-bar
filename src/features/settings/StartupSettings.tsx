import { Box, Divider, FormControlLabel, Paper, Stack, Switch, Typography } from '@mui/material';
import { useTranslation } from 'react-i18next';

import { type Settings } from '@/contracts/settings';

import type { usePreferences } from './usePreferences';
type Preferences = ReturnType<typeof usePreferences>;

interface Props {
  settings: Settings;
  settingsControlsDisabled: boolean;
  updateSettings: Preferences['updateSettings'];
  autostart: boolean;
  autostartLoaded: boolean;
  isChangingAutostart: boolean;
  updateAutostart: (enabled: boolean) => Promise<void>;
}
export function StartupSettings({
  settings,
  settingsControlsDisabled,
  updateSettings,
  autostart,
  autostartLoaded,
  isChangingAutostart,
  updateAutostart,
}: Props) {
  const { t } = useTranslation();
  return (
    <Stack spacing={2.5}>
      <Box>
        <Typography component="h1" variant="h5" sx={{ fontWeight: 800 }}>
          {t('settings.startup.title')}
        </Typography>
        <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
          {t('settings.startup.subtitle')}
        </Typography>
      </Box>
      <Paper variant="outlined" sx={{ p: 2 }}>
        <Stack spacing={0.5}>
          <FormControlLabel
            control={
              <Switch
                checked={autostart}
                disabled={!autostartLoaded || isChangingAutostart}
                onChange={(event) => void updateAutostart(event.target.checked)}
              />
            }
            label={t('settings.startup.autostart')}
          />
          <Typography variant="caption" color="text.secondary" sx={{ display: 'block', pl: 4 }}>
            {t('settings.startup.description')}
          </Typography>
          <Divider sx={{ my: 1 }} />
          <FormControlLabel
            control={
              <Switch
                checked={settings.minimizeToTrayOnClose}
                disabled={settingsControlsDisabled}
                onChange={(event) => void updateSettings({ minimizeToTrayOnClose: event.target.checked })}
              />
            }
            label={t('settings.startup.minimizeToTrayOnClose')}
          />
          <Typography variant="caption" color="text.secondary" sx={{ display: 'block', pl: 4 }}>
            {t('settings.startup.minimizeToTrayOnCloseDescription')}
          </Typography>
        </Stack>
      </Paper>
    </Stack>
  );
}
