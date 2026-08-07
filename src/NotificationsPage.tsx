import NotificationsOutlinedIcon from '@mui/icons-material/NotificationsOutlined';
import {
  Box,
  Button,
  CircularProgress,
  Divider,
  FormControlLabel,
  Paper,
  Stack,
  Switch,
  TextField,
  Typography,
} from '@mui/material';
import { useTranslation } from 'react-i18next';

import type { NotificationSettings } from './types';

export interface NotificationsPageProps {
  notifications: NotificationSettings;
  disabled: boolean;
  isSaving: boolean;
  isSendingTestNotification: boolean;
  onSetEnabled: (enabled: boolean) => Promise<void>;
  onUpdate: (partial: Partial<NotificationSettings>) => Promise<boolean>;
  onSendTest: () => Promise<void>;
}

function clampPercent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(100, Math.max(0, Math.round(value)));
}

/**
 * 独立通知设置页。这里只编辑现有 NotificationSettings 字段，权限申请与持久化仍由设置窗口统一调度。
 */
export default function NotificationsPage({
  notifications,
  disabled,
  isSaving,
  isSendingTestNotification,
  onSetEnabled,
  onUpdate,
  onSendTest,
}: NotificationsPageProps) {
  const { t } = useTranslation();

  return (
    <Stack component="section" spacing={2.5} aria-labelledby="notifications-page-title">
      <Box>
        <Typography id="notifications-page-title" component="h1" variant="h5" sx={{ fontWeight: 800 }}>
          {t('settings.notifications.title')}
        </Typography>
        <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
          {t('settings.notifications.subtitle')}
        </Typography>
      </Box>

      <Paper variant="outlined" sx={{ p: 2 }}>
        <Stack spacing={1.5}>
          <FormControlLabel
            control={
              <Switch
                checked={notifications.enabled}
                disabled={disabled}
                onChange={(event) => void onSetEnabled(event.target.checked)}
              />
            }
            label={t('settings.notifications.enabled')}
          />
          <Typography variant="caption" color="text.secondary" sx={{ pl: 4 }}>
            {t('settings.notifications.enabledDescription')}
          </Typography>
          <Divider />

          <FormControlLabel
            control={
              <Switch
                checked={notifications.lowQuotaEnabled}
                disabled={disabled}
                onChange={(event) => void onUpdate({ lowQuotaEnabled: event.target.checked })}
              />
            }
            label={t('settings.notifications.lowQuota')}
          />
          <Typography variant="caption" color="text.secondary" sx={{ pl: 4 }}>
            {t('settings.notifications.lowQuotaDescription')}
          </Typography>
          <TextField
            key={`low-quota-${notifications.lowQuotaThresholdPercent}`}
            label={t('settings.notifications.lowQuotaThreshold')}
            size="small"
            type="number"
            defaultValue={notifications.lowQuotaThresholdPercent}
            disabled={disabled}
            onBlur={(event) => {
              const value = clampPercent(Number(event.target.value));
              event.target.value = String(value);
              void onUpdate({ lowQuotaThresholdPercent: value });
            }}
            slotProps={{ htmlInput: { min: 0, max: 100, step: 1 } }}
          />

          <FormControlLabel
            control={
              <Switch
                checked={notifications.paceEnabled}
                disabled={disabled}
                onChange={(event) => void onUpdate({ paceEnabled: event.target.checked })}
              />
            }
            label={t('settings.notifications.pace')}
          />
          <Typography variant="caption" color="text.secondary" sx={{ pl: 4 }}>
            {t('settings.notifications.paceDescription')}
          </Typography>
          <TextField
            key={`pace-${notifications.paceDeficitThresholdPercent}`}
            label={t('settings.notifications.paceThreshold')}
            size="small"
            type="number"
            defaultValue={notifications.paceDeficitThresholdPercent}
            disabled={disabled}
            onBlur={(event) => {
              const value = clampPercent(Number(event.target.value));
              event.target.value = String(value);
              void onUpdate({ paceDeficitThresholdPercent: value });
            }}
            slotProps={{ htmlInput: { min: 0, max: 100, step: 1 } }}
          />

          <FormControlLabel
            control={
              <Switch
                checked={notifications.resetEnabled}
                disabled={disabled}
                onChange={(event) => void onUpdate({ resetEnabled: event.target.checked })}
              />
            }
            label={t('settings.notifications.reset')}
          />
          <Typography variant="caption" color="text.secondary" sx={{ pl: 4 }}>
            {t('settings.notifications.resetDescription')}
          </Typography>

          <Divider />
          <FormControlLabel
            control={
              <Switch
                checked={notifications.quietHoursEnabled}
                disabled={disabled}
                onChange={(event) => void onUpdate({ quietHoursEnabled: event.target.checked })}
              />
            }
            label={t('settings.notifications.quietHours')}
          />
          <Typography variant="caption" color="text.secondary" sx={{ pl: 4 }}>
            {t('settings.notifications.quietHoursDescription')}
          </Typography>
          <Stack direction={{ xs: 'column', sm: 'row' }} spacing={1.5}>
            <TextField
              key={`quiet-start-${notifications.quietHoursStart}`}
              fullWidth
              label={t('settings.notifications.quietStart')}
              size="small"
              type="time"
              defaultValue={notifications.quietHoursStart}
              disabled={disabled || !notifications.quietHoursEnabled}
              onBlur={(event) => {
                const value = event.target.value;
                if (/^\d{2}:\d{2}$/.test(value)) void onUpdate({ quietHoursStart: value });
                else event.target.value = notifications.quietHoursStart;
              }}
              slotProps={{ inputLabel: { shrink: true } }}
            />
            <TextField
              key={`quiet-end-${notifications.quietHoursEnd}`}
              fullWidth
              label={t('settings.notifications.quietEnd')}
              size="small"
              type="time"
              defaultValue={notifications.quietHoursEnd}
              disabled={disabled || !notifications.quietHoursEnabled}
              onBlur={(event) => {
                const value = event.target.value;
                if (/^\d{2}:\d{2}$/.test(value)) void onUpdate({ quietHoursEnd: value });
                else event.target.value = notifications.quietHoursEnd;
              }}
              slotProps={{ inputLabel: { shrink: true } }}
            />
          </Stack>
          <Button
            disabled={disabled || !notifications.enabled || isSendingTestNotification || isSaving}
            startIcon={
              isSendingTestNotification ? <CircularProgress color="inherit" size={16} /> : <NotificationsOutlinedIcon />
            }
            sx={{ alignSelf: 'flex-start' }}
            variant="outlined"
            onClick={() => void onSendTest()}
          >
            {t('settings.notifications.test')}
          </Button>
        </Stack>
      </Paper>
    </Stack>
  );
}
