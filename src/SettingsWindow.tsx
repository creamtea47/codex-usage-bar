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
  LinearProgress,
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
import { getCurrentWindow } from '@tauri-apps/api/window';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useEffect, useMemo, useState } from 'react';

import { usageBridge } from './bridge';
import { formatDateTime } from './format';
import { createUsageTheme } from './theme';
import {
  defaultSettings,
  type AppUpdateInfo,
  type AppUpdateProgress,
  type Settings,
  type Theme,
} from './types';

type SettingsSection = 'display' | 'data' | 'startup' | 'about';

interface Feedback {
  message: string;
  severity: AlertColor;
}

const drawerWidth = 208;
const releasePageUrl = 'https://github.com/creamtea47/codex-usage-bar/releases';

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

function formatUpdateProgress(progress: AppUpdateProgress | null): string {
  if (!progress) return '正在准备更新…';
  if (progress.stage === 'verifying') return '正在验证更新包签名…';
  if (progress.stage === 'installing') return '正在交接系统安装程序…';
  if (progress.totalBytes && progress.totalBytes > 0) {
    return `正在下载更新：${Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100))}%`;
  }
  return '正在下载更新…';
}

/** Rust 只会返回这些固定更新错误；前端白名单展示，避免意外回显第三方网络异常细节。 */
function safeUpdateErrorMessage(error: unknown, fallback: string): string {
  const message = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
  const safeMessages = [
    '更新包的签名验证失败，已取消安装。',
    '暂时没有适用于当前设备的有效更新。',
    '无法连接更新服务，请检查网络或代理后重试。',
    '更新服务配置不可用，请稍后重试。',
    '更新安装未完成，请稍后重试。',
    '更新操作正在进行，请稍后再试。',
    '没有待安装的更新，请先重新检查更新。',
  ];
  return safeMessages.find((candidate) => message.includes(candidate)) ?? fallback;
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
  const [isInstallingUpdate, setIsInstallingUpdate] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<AppUpdateInfo | null>(null);
  const [updateProgress, setUpdateProgress] = useState<AppUpdateProgress | null>(null);
  const [isUpdateDialogOpen, setIsUpdateDialogOpen] = useState(false);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const prefersDark = useMediaQuery('(prefers-color-scheme: dark)', { noSsr: true });

  useEffect(() => {
    let disposed = false;
    let removeSettingsListener: (() => void) | undefined;
    let removeUpdateListener: (() => void) | undefined;
    let removeUpdateProgressListener: (() => void) | undefined;
    let removeNavigationListener: (() => void) | undefined;
    let settingsEventReceived = false;
    let updateEventReceived = false;

    void (async () => {
      try {
        const [persistedSettings, autostartEnabled, existingUpdate, removeListener, removeUpdate, removeUpdateProgress, removeNavigation] = await Promise.all([
          usageBridge.getSettings(),
          usageBridge.getAutostart(),
          usageBridge.getAppUpdateInfo(),
          usageBridge.listenForSettings((next) => {
            settingsEventReceived = true;
            if (!disposed) setSettings(next);
          }),
          usageBridge.listenForAppUpdate((next) => {
            updateEventReceived = true;
            if (!disposed) setUpdateInfo(next);
          }),
          usageBridge.listenForAppUpdateProgress((next) => {
            if (!disposed) setUpdateProgress(next);
          }),
          usageBridge.listenForSettingsNavigation((section) => {
            if (!disposed) {
              setActiveSection(section);
              setIsUpdateDialogOpen(section === 'about');
            }
          }),
        ]);
        if (disposed) {
          removeListener();
          removeUpdate();
          removeUpdateProgress();
          removeNavigation();
          return;
        }
        // Prefer events received during startup over command snapshots captured earlier.
        if (!settingsEventReceived) setSettings(persistedSettings);
        setAutostart(autostartEnabled);
        if (!updateEventReceived) setUpdateInfo(existingUpdate);
        removeSettingsListener = removeListener;
        removeUpdateListener = removeUpdate;
        removeUpdateProgressListener = removeUpdateProgress;
        removeNavigationListener = removeNavigation;
      } catch {
        if (!disposed) setFeedback({ message: '无法读取当前设置，请稍后重试。', severity: 'error' });
      }
    })();

    return () => {
      disposed = true;
      removeSettingsListener?.();
      removeUpdateListener?.();
      removeUpdateProgressListener?.();
      removeNavigationListener?.();
    };
  }, []);

  const activeTheme = useMemo(() => resolveTheme(settings.theme, prefersDark), [settings.theme, prefersDark]);
  const muiTheme = useMemo(() => createUsageTheme(activeTheme), [activeTheme]);
  const nativeTheme = settings.theme === 'system' ? null : settings.theme;

  useEffect(() => {
    // MUI only themes the webview. Keep the native macOS/Windows title bar in sync too.
    // `null` lets the native window keep tracking system appearance changes.
    const currentWindow = getCurrentWindow();
    void Promise.all([
      currentWindow.setTheme(nativeTheme),
      currentWindow.setBackgroundColor(muiTheme.palette.background.paper),
    ])
      .catch(() => setFeedback({ message: '原生窗口主题同步失败。', severity: 'error' }));
  }, [muiTheme.palette.background.paper, nativeTheme]);

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
    setUpdateProgress(null);
    try {
      setUpdateInfo(await usageBridge.checkAppUpdate());
      setIsUpdateDialogOpen(true);
    } catch (error) {
      setFeedback({ message: safeUpdateErrorMessage(error, '暂时无法检查更新，请稍后重试。'), severity: 'error' });
    } finally {
      setIsCheckingUpdate(false);
    }
  };

  const installUpdate = async () => {
    if (!updateInfo?.updateAvailable || isInstallingUpdate) return;
    setIsInstallingUpdate(true);
    setUpdateProgress({ stage: 'downloading', downloadedBytes: 0, totalBytes: null });
    try {
      await usageBridge.installAppUpdate();
      setFeedback({ message: '更新已交接给系统安装程序。', severity: 'success' });
    } catch (error) {
      setFeedback({ message: safeUpdateErrorMessage(error, '更新未完成，请检查网络后重试。'), severity: 'error' });
    } finally {
      setIsInstallingUpdate(false);
    }
  };

  const openReleasePage = async () => {
    try {
      await openUrl(releasePageUrl);
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
                    通过公开 HTTPS 更新清单检查；下载后验证更新包签名，不会读取或发送 Codex 认证信息。
                  </Typography>
                </Box>
                <Paper variant="outlined" sx={{ p: 2 }}>
                  <Stack spacing={1.5}>
                    <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                      应用更新
                    </Typography>
                    <Typography variant="body2" color="text.secondary">
                      当前版本 v{updateInfo?.currentVersion ?? '—'}
                    </Typography>
                    <FormControlLabel
                      control={<Switch checked={settings.autoCheckUpdates} disabled={isSaving} onChange={(event) => void updateSettings({ autoCheckUpdates: event.target.checked })} />}
                      label="自动检查更新"
                    />
                    <Typography variant="caption" color="text.secondary" sx={{ pl: 4 }}>
                      启动后会检查一次，之后最多每 6 小时检查一次；仅检测，不会自动下载、安装或重启。
                    </Typography>
                    {updateInfo?.checkedAt && (
                      <Typography variant="caption" color="text.secondary">
                        最近检查：{formatDateTime(updateInfo.checkedAt)}
                      </Typography>
                    )}
                    {updateInfo?.updateAvailable && (
                      <Alert
                        severity="info"
                        variant="outlined"
                        action={
                          <Button color="inherit" size="small" onClick={() => setIsUpdateDialogOpen(true)}>
                            查看
                          </Button>
                        }
                      >
                        发现新版本 v{updateInfo.latestVersion}
                      </Alert>
                    )}
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

      <Dialog fullWidth maxWidth="xs" open={isUpdateDialogOpen} onClose={() => !isInstallingUpdate && setIsUpdateDialogOpen(false)}>
        <DialogTitle>检查更新</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            <Typography variant="h6" sx={{ fontWeight: 800 }}>
              {updateInfo?.updateAvailable ? `发现新版本 v${updateInfo.latestVersion}` : '当前已是最新版本'}
            </Typography>
            <Typography variant="body2" color="text.secondary">
              当前版本 v{updateInfo?.currentVersion} · 最新版本 v{updateInfo?.latestVersion}
            </Typography>
            {updateInfo?.checkedAt && (
              <Typography variant="body2" color="text.secondary">
                检查时间 {formatDateTime(updateInfo.checkedAt)}
              </Typography>
            )}
            {updateInfo?.updateAvailable && (
              <Alert severity="info" variant="outlined">
                点击“下载并安装”后会下载并验证签名。Windows 将在开始安装时关闭应用；macOS 完成替换后会重新启动。
              </Alert>
            )}
            {isInstallingUpdate && (
              <Stack spacing={0.75}>
                <LinearProgress
                  variant={updateProgress?.totalBytes ? 'determinate' : 'indeterminate'}
                  value={
                    updateProgress?.totalBytes
                      ? Math.min(100, (updateProgress.downloadedBytes / updateProgress.totalBytes) * 100)
                      : undefined
                  }
                />
                <Typography variant="caption" color="text.secondary">
                  {formatUpdateProgress(updateProgress)}
                </Typography>
              </Stack>
            )}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button disabled={isInstallingUpdate} onClick={() => setIsUpdateDialogOpen(false)}>
            关闭
          </Button>
          <Button disabled={isInstallingUpdate} endIcon={<LaunchRoundedIcon />} onClick={() => void openReleasePage()}>
            发布页
          </Button>
          {updateInfo?.updateAvailable && (
            <Button
              disabled={isInstallingUpdate}
              startIcon={isInstallingUpdate ? <CircularProgress color="inherit" size={16} /> : <SystemUpdateAltRoundedIcon />}
              variant="contained"
              onClick={() => void installUpdate()}
            >
              {isInstallingUpdate ? '正在更新…' : '下载并安装'}
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
