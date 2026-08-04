import {
  Button,
  CircularProgress,
  Divider,
  FormControl,
  FormControlLabel,
  InputLabel,
  MenuItem,
  Popover,
  Select,
  Stack,
  Switch,
  Typography,
} from '@mui/material';
import SystemUpdateAltRoundedIcon from '@mui/icons-material/SystemUpdateAltRounded';

import type { Settings, Theme } from './types';

interface SettingsPopoverProps {
  anchorEl: HTMLButtonElement | null;
  open: boolean;
  settings: Settings;
  autostart: boolean;
  disabled: boolean;
  isCheckingUpdate: boolean;
  onClose: () => void;
  onSettingsChange: (settings: Settings) => void;
  onAutostartChange: (enabled: boolean) => void;
  onCheckUpdate: () => void;
}

const refreshOptions = [
  { seconds: 60, label: '1 分钟' },
  { seconds: 180, label: '3 分钟' },
  { seconds: 300, label: '5 分钟' },
  { seconds: 600, label: '10 分钟' },
  { seconds: 1800, label: '30 分钟' },
];

/** 小窗口使用 Popover 呈现设置，避免 Dialog 遮住用量信息。 */
export function SettingsPopover({
  anchorEl,
  open,
  settings,
  autostart,
  disabled,
  isCheckingUpdate,
  onClose,
  onSettingsChange,
  onAutostartChange,
  onCheckUpdate,
}: SettingsPopoverProps) {
  const update = (partial: Partial<Settings>) => onSettingsChange({ ...settings, ...partial });

  return (
    <Popover
      anchorEl={anchorEl}
      anchorOrigin={{ vertical: 'bottom', horizontal: 'right' }}
      transformOrigin={{ vertical: 'top', horizontal: 'right' }}
      open={open}
      onClose={onClose}
      slotProps={{
        paper: {
          sx: { width: 276, maxWidth: 'calc(100vw - 24px)', maxHeight: 'calc(100vh - 16px)', overflowY: 'auto', p: 1.5, mt: 0.5 },
        },
      }}
    >
      <Stack spacing={0.55}>
        <Typography variant="overline" color="text.secondary" sx={{ fontWeight: 700 }}>
          显示设置
        </Typography>
        <FormControlLabel
          control={
            <Switch
              checked={settings.alwaysOnTop}
              disabled={disabled}
              onChange={(event) => update({ alwaysOnTop: event.target.checked })}
            />
          }
          label="始终置顶"
        />
        <FormControlLabel
          control={
            <Switch
              checked={settings.lockPosition}
              disabled={disabled}
              onChange={(event) => update({ lockPosition: event.target.checked })}
            />
          }
          label="锁定位置与大小"
        />
        <FormControlLabel
          control={
            <Switch
              checked={autostart}
              disabled={disabled}
              onChange={(event) => onAutostartChange(event.target.checked)}
            />
          }
          label="开机启动"
        />
        <Divider sx={{ my: 0.5 }} />
        <Stack spacing={0.65}>
          <Typography variant="overline" color="text.secondary" sx={{ fontWeight: 700 }}>
            应用更新
          </Typography>
          <Button
            disabled={disabled || isCheckingUpdate}
            fullWidth
            size="small"
            startIcon={isCheckingUpdate ? <CircularProgress color="inherit" size={14} /> : <SystemUpdateAltRoundedIcon />}
            variant="outlined"
            onClick={onCheckUpdate}
          >
            {isCheckingUpdate ? '正在检查…' : '检查更新'}
          </Button>
          <Typography variant="caption" color="text.secondary">
            仅检查公开 Release，不会自动下载或安装。
          </Typography>
        </Stack>
        <Divider sx={{ my: 0.5 }} />
        <FormControl fullWidth size="small" disabled={disabled}>
          <InputLabel id="refresh-interval-label">自动刷新</InputLabel>
          <Select
            label="自动刷新"
            labelId="refresh-interval-label"
            value={settings.refreshIntervalSeconds}
            onChange={(event) => update({ refreshIntervalSeconds: Number(event.target.value) })}
          >
            {refreshOptions.map((option) => (
              <MenuItem key={option.seconds} value={option.seconds}>
                {option.label}
              </MenuItem>
            ))}
          </Select>
        </FormControl>
        <FormControl fullWidth size="small" disabled={disabled}>
          <InputLabel id="theme-label">主题</InputLabel>
          <Select
            label="主题"
            labelId="theme-label"
            value={settings.theme}
            onChange={(event) => update({ theme: event.target.value as Theme })}
          >
            <MenuItem value="system">跟随系统</MenuItem>
            <MenuItem value="light">浅色</MenuItem>
            <MenuItem value="dark">深色</MenuItem>
          </Select>
        </FormControl>
      </Stack>
    </Popover>
  );
}
