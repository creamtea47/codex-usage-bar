import InfoOutlinedIcon from '@mui/icons-material/InfoOutlined';
import LaunchRoundedIcon from '@mui/icons-material/LaunchRounded';
import PaletteOutlinedIcon from '@mui/icons-material/PaletteOutlined';
import RefreshRoundedIcon from '@mui/icons-material/RefreshRounded';
import RocketLaunchOutlinedIcon from '@mui/icons-material/RocketLaunchOutlined';
import SettingsSuggestOutlinedIcon from '@mui/icons-material/SettingsSuggestOutlined';
import SystemUpdateAltRoundedIcon from '@mui/icons-material/SystemUpdateAltRounded';
import {
  Alert,
  Box,
  Button,
  CircularProgress,
  CssBaseline,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  Drawer,
  FormControl,
  FormControlLabel,
  InputLabel,
  List,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  MenuItem,
  Paper,
  Select,
  Snackbar,
  Stack,
  Switch,
  ThemeProvider,
  Toolbar,
  Typography,
  useMediaQuery,
  type AlertColor,
} from '@mui/material';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useEffect, useMemo, useState } from 'react';

import { usageBridge } from './bridge';
import { formatDateTime } from './format';
import { createUsageTheme } from './theme';
import { defaultSettings, type Settings, type Theme, type UpdateInfo } from './types';

type SettingsSection = 'display' | 'data' | 'startup' | 'about';

interface Feedback {
  message: string;
  severity: AlertColor;
}

const drawerWidth = 208;

const refreshOptions = [
  { seconds: 60, label: '1 分钟' },
  { seconds: 180, label: '3 分钟' },
  { seconds: 300, label: '5 分钟' },
  { seconds: 600, label: '10 分钟' },
  { seconds: 1800, label: '30 分钟' },
];

const navigation: Array<{ id: SettingsSection; label: string; icon: React.ReactElement }> = [
  { id: 'display', label: '显示', icon: <PaletteOutlinedIcon fontSize="small" /> },
  { id: 'data', label: '数据与刷新', icon: <RefreshRoundedIcon fontSize="small" /> },
  { id: 'startup', label: '启动', icon: <RocketLaunchOutlinedIcon fontSize="small" /> },
  { id: 'about', label: '关于与更新', icon: <InfoOutlinedIcon fontSize="small" /> },
];

function resolveTheme(theme: Theme, prefersDark: boolean): Exclude<Theme, 'system'> {
  return theme === 'system' ? (prefersDark ? 'dark' : 'light') : theme;
}

/**
 * 设置页运行在独立、非透明的 Tauri 窗口中。左侧导航现在已是实际设置分类，后续新增项可以继续归类，
 * 而不再挤入悬浮卡的 Popover。所有持久化仍由 Rust command 完成。
 */
export default function SettingsWindow() {
  const [settings, setSettings] = useState<Settings>(defaultSettings);
  const [autostart, setAutostart] = useState(false);
  const [activeSection, setActiveSection] = useState<SettingsSection>('display');
  const [isSaving, setIsSaving] = useState(false);
  const [isCheckingUpdate, setIsCheckingUpdate] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const prefersDark = useMediaQuery('(prefers-color-scheme: dark)', { noSsr: true });

  useEffect(() => {
    let disposed = false;
    let removeSettingsListener: (() => void) | undefined;

    void (async () => {
      try {
        const [persistedSettings, autostartEnabled, removeListener] = await Promise.all([
          usageBridge.getSettings(),
          usageBridge.getAutostart(),
          usageBridge.listenForSettings((next) => {
            if (!disposed) setSettings(next);
          }),
        ]);
        if (disposed) {
          removeListener();
          return;
        }
        setSettings(persistedSettings);
        setAutostart(autostartEnabled);
        removeSettingsListener = removeListener;
      } catch {
        if (!disposed) setFeedback({ message: '无法读取当前设置，请稍后重试。', severity: 'error' });
      }
    })();

    return () => {
      disposed = true;
      removeSettingsListener?.();
    };
  }, []);

  const activeTheme = useMemo(() => resolveTheme(settings.theme, prefersDark), [settings.theme, prefersDark]);
  const muiTheme = useMemo(() => createUsageTheme(activeTheme), [activeTheme]);

  const updateSettings = async (partial: Partial<Settings>) => {
    if (isSaving) return;
    const next = { ...settings, ...partial };
    setIsSaving(true);
    try {
      setSettings(await usageBridge.saveSettings(next));
      setFeedback({ message: '设置已保存。', severity: 'success' });
    } catch {
      setFeedback({ message: '无法保存设置。', severity: 'error' });
    } finally {
      setIsSaving(false);
    }
  };

  const updateAutostart = async (enabled: boolean) => {
    if (isSaving) return;
    setIsSaving(true);
    try {
      setAutostart(await usageBridge.setAutostart(enabled));
      setFeedback({ message: '开机启动设置已保存。', severity: 'success' });
    } catch {
      setFeedback({ message: '无法更新开机启动设置。', severity: 'error' });
    } finally {
      setIsSaving(false);
    }
  };

  const checkUpdate = async () => {
    if (isCheckingUpdate) return;
    setIsCheckingUpdate(true);
    try {
      setUpdateInfo(await usageBridge.checkUpdate());
    } catch {
      setFeedback({ message: '暂时无法检查更新，请稍后重试。', severity: 'error' });
    } finally {
      setIsCheckingUpdate(false);
    }
  };

  const openReleasePage = async () => {
    if (!updateInfo) return;
    try {
      await openUrl(updateInfo.downloadUrl ?? updateInfo.releaseUrl);
    } catch {
      setFeedback({ message: '无法打开发布页面，请稍后重试。', severity: 'error' });
    }
  };

  return (
    <ThemeProvider theme={muiTheme}>
      <CssBaseline />
      <Box
        component="main"
        data-theme-mode={activeTheme}
        sx={{ display: 'flex', width: '100%', minHeight: '100vh', bgcolor: 'background.paper', color: 'text.primary' }}
      >
        <Drawer
          variant="permanent"
          sx={{
            width: drawerWidth,
            flexShrink: 0,
            '& .MuiDrawer-paper': {
              width: drawerWidth,
              boxSizing: 'border-box',
              borderRight: 1,
              borderColor: 'divider',
              bgcolor: 'background.paper',
            },
          }}
        >
          <Toolbar sx={{ minHeight: 72, px: 2 }}>
            <Stack direction="row" spacing={1} sx={{ alignItems: 'center', minWidth: 0 }}>
              <SettingsSuggestOutlinedIcon color="primary" />
              <Box sx={{ minWidth: 0 }}>
                <Typography variant="subtitle2" noWrap sx={{ fontWeight: 800 }}>
                  CodexUsageBar
                </Typography>
                <Typography variant="caption" color="text.secondary" noWrap>
                  设置
                </Typography>
              </Box>
            </Stack>
          </Toolbar>
          <Divider />
          <List disablePadding sx={{ px: 1, py: 1 }}>
            {navigation.map((item) => (
              <ListItemButton key={item.id} selected={activeSection === item.id} onClick={() => setActiveSection(item.id)} sx={{ borderRadius: 1.5, mb: 0.5 }}>
                <ListItemIcon sx={{ minWidth: 34 }}>{item.icon}</ListItemIcon>
                <ListItemText
                  primary={item.label}
                  slotProps={{ primary: { sx: { fontSize: 14, fontWeight: activeSection === item.id ? 700 : 500 } } }}
                />
              </ListItemButton>
            ))}
          </List>
        </Drawer>

        <Box component="section" sx={{ flex: 1, minWidth: 0, overflow: 'auto', bgcolor: 'background.default' }}>
          <Box sx={{ width: '100%', maxWidth: 680, mx: 'auto', px: { xs: 2, sm: 4 }, py: 4 }}>
            {activeSection === 'display' && (
              <Stack spacing={2.5}>
                <Box>
                  <Typography component="h1" variant="h5" sx={{ fontWeight: 800 }}>
                    显示
                  </Typography>
                  <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
                    调整悬浮卡的外观与窗口交互方式。
                  </Typography>
                </Box>
                <Paper variant="outlined" sx={{ p: 2 }}>
                  <Stack spacing={0.5}>
                    <FormControlLabel
                      control={<Switch checked={settings.alwaysOnTop} disabled={isSaving} onChange={(event) => void updateSettings({ alwaysOnTop: event.target.checked })} />}
                      label="始终置顶"
                    />
                    <Typography variant="caption" color="text.secondary" sx={{ pl: 4 }}>
                      让主卡保持在其他窗口前方。
                    </Typography>
                    <Divider sx={{ my: 1 }} />
                    <FormControlLabel
                      control={<Switch checked={settings.lockPosition} disabled={isSaving} onChange={(event) => void updateSettings({ lockPosition: event.target.checked })} />}
                      label="锁定位置与大小"
                    />
                    <Typography variant="caption" color="text.secondary" sx={{ pl: 4 }}>
                      锁定后无法拖动或从边缘调整主卡大小。
                    </Typography>
                  </Stack>
                </Paper>
                <Paper variant="outlined" sx={{ p: 2 }}>
                  <Stack>
                    <FormControl fullWidth size="small" disabled={isSaving}>
                      <InputLabel id="theme-label">主题</InputLabel>
                      <Select
                        label="主题"
                        labelId="theme-label"
                        value={settings.theme}
                        onChange={(event) => void updateSettings({ theme: event.target.value as Theme })}
                      >
                        <MenuItem value="system">跟随系统</MenuItem>
                        <MenuItem value="light">浅色</MenuItem>
                        <MenuItem value="dark">深色</MenuItem>
                      </Select>
                    </FormControl>
                  </Stack>
                </Paper>
              </Stack>
            )}

            {activeSection === 'data' && (
              <Stack spacing={2.5}>
                <Box>
                  <Typography component="h1" variant="h5" sx={{ fontWeight: 800 }}>
                    数据与刷新
                  </Typography>
                  <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
                    控制主卡的自动刷新频率；手动刷新始终可用。
                  </Typography>
                </Box>
                <Paper variant="outlined" sx={{ p: 2 }}>
                  <Stack spacing={1.25}>
                    <FormControl fullWidth size="small" disabled={isSaving}>
                      <InputLabel id="refresh-interval-label">自动刷新</InputLabel>
                      <Select
                        label="自动刷新"
                        labelId="refresh-interval-label"
                        value={settings.refreshIntervalSeconds}
                        onChange={(event) => void updateSettings({ refreshIntervalSeconds: Number(event.target.value) })}
                      >
                        {refreshOptions.map((option) => (
                          <MenuItem key={option.seconds} value={option.seconds}>
                            {option.label}
                          </MenuItem>
                        ))}
                      </Select>
                    </FormControl>
                    <Alert severity="info" variant="outlined">
                      用量数据仅由 Rust 后端只读获取。前端不会读取认证文件，也不会续期 Token 或操作重置额度。
                    </Alert>
                  </Stack>
                </Paper>
              </Stack>
            )}

            {activeSection === 'startup' && (
              <Stack spacing={2.5}>
                <Box>
                  <Typography component="h1" variant="h5" sx={{ fontWeight: 800 }}>
                    启动
                  </Typography>
                  <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
                    配置 Windows 或 macOS 登录后是否自动启动 CodexUsageBar。
                  </Typography>
                </Box>
                <Paper variant="outlined" sx={{ p: 2 }}>
                  <FormControlLabel
                    control={<Switch checked={autostart} disabled={isSaving} onChange={(event) => void updateAutostart(event.target.checked)} />}
                    label="开机启动"
                  />
                  <Typography variant="caption" color="text.secondary" sx={{ display: 'block', pl: 4 }}>
                    此设置仅控制新版 CodexUsageBar；旧版自启项不会被自动删除。
                  </Typography>
                </Paper>
              </Stack>
            )}

            {activeSection === 'about' && (
              <Stack spacing={2.5}>
                <Box>
                  <Typography component="h1" variant="h5" sx={{ fontWeight: 800 }}>
                    关于与更新
                  </Typography>
                  <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
                    仅查询公开 GitHub Release，不会自动下载、替换或重启应用。
                  </Typography>
                </Box>
                <Paper variant="outlined" sx={{ p: 2 }}>
                  <Stack spacing={1.25}>
                    <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                      应用更新
                    </Typography>
                    <Button
                      disabled={isCheckingUpdate}
                      startIcon={isCheckingUpdate ? <CircularProgress color="inherit" size={16} /> : <SystemUpdateAltRoundedIcon />}
                      sx={{ alignSelf: 'flex-start' }}
                      variant="contained"
                      onClick={() => void checkUpdate()}
                    >
                      {isCheckingUpdate ? '正在检查…' : '检查更新'}
                    </Button>
                  </Stack>
                </Paper>
              </Stack>
            )}
          </Box>
        </Box>
      </Box>

      <Dialog fullWidth maxWidth="xs" open={Boolean(updateInfo)} onClose={() => setUpdateInfo(null)}>
        <DialogTitle>检查更新</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1}>
            <Typography variant="h6" sx={{ fontWeight: 800 }}>
              {updateInfo?.updateAvailable ? `发现新版本 v${updateInfo.latestVersion}` : '当前已是最新版本'}
            </Typography>
            <Typography variant="body2" color="text.secondary">
              当前版本 v{updateInfo?.currentVersion} · 最新版本 v{updateInfo?.latestVersion}
            </Typography>
            {updateInfo?.publishedAt && (
              <Typography variant="body2" color="text.secondary">
                发布于 {formatDateTime(updateInfo.publishedAt)}
              </Typography>
            )}
            {updateInfo?.updateAvailable && (
              <Alert severity="info" variant="outlined">
                请在发布页下载安装包。应用不会自动下载、替换或重启自身。
              </Alert>
            )}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setUpdateInfo(null)}>关闭</Button>
          {updateInfo?.updateAvailable && (
            <Button endIcon={<LaunchRoundedIcon />} variant="contained" onClick={() => void openReleasePage()}>
              打开发布页
            </Button>
          )}
        </DialogActions>
      </Dialog>

      <Snackbar
        anchorOrigin={{ vertical: 'bottom', horizontal: 'center' }}
        autoHideDuration={2_400}
        open={Boolean(feedback)}
        onClose={() => setFeedback(null)}
      >
        <Alert severity={feedback?.severity ?? 'info'} variant="filled" onClose={() => setFeedback(null)}>
          {feedback?.message}
        </Alert>
      </Snackbar>
    </ThemeProvider>
  );
}
