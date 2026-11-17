import { Alert, Button, FormControlLabel, Paper, Stack, Switch, TextField, Typography } from '@mui/material';
import { lightweightApi } from './lightweightApi';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { Settings } from '@/contracts/settings';

interface Props {
  settings: Settings;
  disabled: boolean;
  update: (patch: Partial<Settings>) => Promise<boolean>;
}

export function LightweightSettings({ settings, disabled, update }: Props) {
  const { t, i18n } = useTranslation();
  const [measurement, setMeasurement] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let active = true;
    void lightweightApi
      .measurement()
      .then((value) => {
        if (active) setMeasurement(value);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [i18n.resolvedLanguage]);
  return (
    <Paper variant="outlined" sx={{ p: 2 }}>
      <Stack spacing={1.5} sx={{ alignItems: 'flex-start' }}>
        <Typography variant="subtitle1" sx={{ fontWeight: 700 }}>
          {t('feature.liteMode')}
        </Typography>
        <Typography variant="body2" color="text.secondary">
          {t('feature.liteHelp')}
        </Typography>
        <FormControlLabel
          control={
            <Switch
              disabled={disabled}
              checked={settings.autoLightweight}
              onChange={(event) => void update({ autoLightweight: event.target.checked })}
            />
          }
          label={t('feature.autoLite')}
        />
        <TextField
          key={settings.lightweightMinutes}
          type="number"
          size="small"
          sx={{ width: 180, maxWidth: '100%' }}
          label={t('feature.liteDelay')}
          defaultValue={settings.lightweightMinutes}
          disabled={disabled || !settings.autoLightweight}
          slotProps={{ htmlInput: { min: 1, max: 1440, step: 1 } }}
          onBlur={(event) => {
            const minutes = Math.round(Number(event.target.value));
            if (minutes >= 1 && minutes <= 1440) void update({ lightweightMinutes: minutes });
            else event.target.value = String(settings.lightweightMinutes);
          }}
        />
        <Button
          variant="outlined"
          disabled={disabled}
          onClick={() => {
            setFailed(false);
            void lightweightApi.enter().catch(() => setFailed(true));
          }}
        >
          {t('feature.enterLite')}
        </Button>
        {measurement && (
          <Typography variant="caption">
            {t('feature.liteMemory')} · {measurement}
          </Typography>
        )}
        {failed && <Alert severity="error">{t('feature.failed')}</Alert>}
      </Stack>
    </Paper>
  );
}
