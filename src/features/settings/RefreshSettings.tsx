import {
  Alert,
  Box,
  FormControl,
  InputLabel,
  MenuItem,
  Paper,
  Select,
  Stack,
  Typography,
} from '@mui/material';
import { useTranslation } from 'react-i18next';

import { type Settings } from '@/contracts/settings';

import type { usePreferences } from './usePreferences';
type Preferences = ReturnType<typeof usePreferences>;
const refreshOptions = [60, 180, 300, 600, 1800] as const;

interface Props {
  settings: Settings;
  settingsControlsDisabled: boolean;
  updateSettings: Preferences['updateSettings'];
}
export function RefreshSettings({ settings, settingsControlsDisabled, updateSettings }: Props) {
  const { t } = useTranslation();
  return (
    <Stack spacing={2.5}>
      <Box>
        <Typography component="h1" variant="h5" sx={{ fontWeight: 800 }}>
          {t('settings.data.title')}
        </Typography>
        <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
          {t('settings.data.subtitle')}
        </Typography>
      </Box>
      <Paper variant="outlined" sx={{ p: 2 }}>
        <Stack spacing={1.25}>
          <FormControl fullWidth size="small" disabled={settingsControlsDisabled}>
            <InputLabel id="refresh-interval-label">{t('settings.data.refresh')}</InputLabel>
            <Select
              label={t('settings.data.refresh')}
              labelId="refresh-interval-label"
              value={settings.refreshIntervalSeconds}
              onChange={(event) =>
                void updateSettings({ refreshIntervalSeconds: Number(event.target.value) })
              }
            >
              {refreshOptions.map((seconds) => (
                <MenuItem key={seconds} value={seconds}>
                  {t('settings.data.refreshMinutes', { minutes: seconds / 60 })}
                </MenuItem>
              ))}
            </Select>
          </FormControl>
          <Alert severity="info" variant="outlined">
            {t('settings.data.privacyNotice')}
          </Alert>
        </Stack>
      </Paper>
    </Stack>
  );
}
