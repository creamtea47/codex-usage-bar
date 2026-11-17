import { LightweightSettings } from './LightweightSettings';
import {
  Box,
  Divider,
  FormControl,
  FormControlLabel,
  InputLabel,
  MenuItem,
  Paper,
  Select,
  Stack,
  Switch,
  Typography,
} from '@mui/material';
import { useTranslation } from 'react-i18next';

import { type Language, type Settings, type Theme } from '@/contracts/settings';

import type { usePreferences } from './usePreferences';
type Preferences = ReturnType<typeof usePreferences>;

interface Props {
  settings: Settings;
  settingsControlsDisabled: boolean;
  updateSettings: Preferences['updateSettings'];
  updateLanguage: Preferences['updateLanguage'];
}
export function DisplaySettings({
  settings,
  settingsControlsDisabled,
  updateSettings,
  updateLanguage,
}: Props) {
  const { t } = useTranslation();
  return (
    <Stack spacing={2.5}>
      <Box>
        <Typography component="h1" variant="h5" sx={{ fontWeight: 800 }}>
          {t('settings.display.title')}
        </Typography>
        <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
          {t('settings.display.subtitle')}
        </Typography>
      </Box>
      <Paper variant="outlined" sx={{ p: 2 }}>
        <Stack spacing={0.5}>
          <FormControlLabel
            control={
              <Switch
                checked={settings.alwaysOnTop}
                disabled={settingsControlsDisabled}
                onChange={(event) => void updateSettings({ alwaysOnTop: event.target.checked })}
              />
            }
            label={t('settings.display.alwaysOnTop')}
          />
          <Typography variant="caption" color="text.secondary" sx={{ pl: 4 }}>
            {t('settings.display.alwaysOnTopDescription')}
          </Typography>
          <Divider sx={{ my: 1 }} />
          <FormControlLabel
            control={
              <Switch
                checked={settings.lockPosition}
                disabled={settingsControlsDisabled}
                onChange={(event) => void updateSettings({ lockPosition: event.target.checked })}
              />
            }
            label={t('settings.display.lockPosition')}
          />
          <Typography variant="caption" color="text.secondary" sx={{ pl: 4 }}>
            {t('settings.display.lockPositionDescription')}
          </Typography>
        </Stack>
      </Paper>
      <Paper variant="outlined" sx={{ p: 2 }}>
        <Stack spacing={2}>
          <FormControl fullWidth size="small" disabled={settingsControlsDisabled}>
            <InputLabel id="theme-label">{t('settings.display.theme')}</InputLabel>
            <Select
              label={t('settings.display.theme')}
              labelId="theme-label"
              value={settings.theme}
              onChange={(event) => void updateSettings({ theme: event.target.value as Theme })}
            >
              <MenuItem value="system">{t('settings.display.themeSystem')}</MenuItem>
              <MenuItem value="light">{t('settings.display.themeLight')}</MenuItem>
              <MenuItem value="dark">{t('settings.display.themeDark')}</MenuItem>
            </Select>
          </FormControl>
          <FormControl fullWidth size="small" disabled={settingsControlsDisabled}>
            <InputLabel id="language-label">{t('settings.display.language')}</InputLabel>
            <Select
              label={t('settings.display.language')}
              labelId="language-label"
              value={settings.language}
              onChange={(event) => void updateLanguage(event.target.value as Language)}
            >
              <MenuItem value="system">{t('settings.display.languageSystem')}</MenuItem>
              <MenuItem value="zh-CN">{t('settings.display.languageChinese')}</MenuItem>
              <MenuItem value="en">{t('settings.display.languageEnglish')}</MenuItem>
            </Select>
          </FormControl>
        </Stack>
      </Paper>
      <LightweightSettings settings={settings} update={updateSettings} disabled={settingsControlsDisabled} />
    </Stack>
  );
}
