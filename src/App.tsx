import CloseRoundedIcon from '@mui/icons-material/CloseRounded';
import RefreshRoundedIcon from '@mui/icons-material/RefreshRounded';
import SettingsOutlinedIcon from '@mui/icons-material/SettingsOutlined';
import {
  Alert,
  AppBar,
  Box,
  Button,
  Chip,
  CircularProgress,
  CssBaseline,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  GlobalStyles,
  IconButton,
  Paper,
  Snackbar,
  Stack,
  ThemeProvider,
  Toolbar,
  Tooltip,
  Typography,
  useMediaQuery,
  type AlertColor,
  type ChipProps,
} from '@mui/material';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useEffect, useMemo, useState, type MouseEvent } from 'react';

import { usageBridge } from './bridge';
import { formatClock, formatDateTime, statusLabel } from './format';
import { QuotaWindowCard } from './QuotaWindowCard';
import { SettingsPopover } from './SettingsPopover';
import { createUsageTheme } from './theme';
import {
  defaultSettings,
  loadingSnapshot,
  type DashboardSnapshot,
  type DashboardStatus,
  type Settings,
  type Theme,
  type UpdateInfo,
} from './types';

type ResizeDirection = 'East' | 'South' | 'SouthEast';

interface Feedback {
  message: string;
  severity: AlertColor;
}

function resolveTheme(theme: Theme, prefersDark: boolean): Exclude<Theme, 'system'> {
  return theme === 'system' ? (prefersDark ? 'dark' : 'light') : theme;
}

function isInteractive(target: EventTarget | null): boolean {
  return (
    target instanceof Element &&
    Boolean(target.closest('button, input, select, label, [role="button"], [role="combobox"], [role="checkbox"], [data-no-drag]'))
  );
}

function chipColor(status: DashboardStatus): ChipProps['color'] {
  switch (status) {
    case 'ready':
      return 'success';
    case 'stale':
      return 'warning';
    case 'error':
      return 'error';
    default:
      return 'default';
  }
}

function chipLabel(status: DashboardStatus): string {
  switch (status) {
    case 'ready':
      return '在线';
    case 'stale':
      return '已过期';
    case 'error':
      return '需处理';
    default:
      return '读取中';
  }
}

/**
 * MUI 仅负责桌面卡片呈现。认证文件、网络请求和本地设置仍全部留在 Rust 后端。
 */
export default function App() {
  const [snapshot, setSnapshot] = useState<DashboardSnapshot>(loadingSnapshot);
  const [settings, setSettings] = useState<Settings>(defaultSettings);
  const [autostart, setAutostart] = useState(false);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [isCheckingUpdate, setIsCheckingUpdate] = useState(false);
  const [settingsAnchor, setSettingsAnchor] = useState<HTMLButtonElement | null>(null);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const prefersDark = useMediaQuery('(prefers-color-scheme: dark)', { noSsr: true });

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void (async () => {
      try {
        const [dashboard, persistedSettings, autostartEnabled, removeListener] = await Promise.all([
          usageBridge.getDashboard(),
          usageBridge.getSettings(),
          usageBridge.getAutostart(),
          usageBridge.listenForDashboard((next) => {
            if (!disposed) setSnapshot(next);
          }),
        ]);
        if (disposed) {
          removeListener();
          return;
        }
        setSnapshot(dashboard);
        setSettings(persistedSettings);
        setAutostart(autostartEnabled);
        unlisten = removeListener;
      } catch {
        if (!disposed) {
          setSnapshot((current) => ({
            ...current,
            status: current.quotaWindows.length ? 'stale' : 'error',
            message: '无法连接到本地用量服务。请从 CodexUsageBar 桌面程序中启动。',
          }));
        }
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const activeTheme = useMemo(() => resolveTheme(settings.theme, prefersDark), [settings.theme, prefersDark]);
  const muiTheme = useMemo(() => createUsageTheme(activeTheme), [activeTheme]);

  const refresh = async () => {
    if (isRefreshing) return;
    setIsRefreshing(true);
    try {
      setSnapshot(await usageBridge.refreshDashboard());
    } catch {
      setFeedback({ message: '刷新请求未完成，请稍后重试。', severity: 'error' });
    } finally {
      setIsRefreshing(false);
    }
  };

  const updateSettings = async (next: Settings) => {
    if (isSaving) return;
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
      const nextUpdate = await usageBridge.checkUpdate();
      setSettingsAnchor(null);
      setUpdateInfo(nextUpdate);
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

  const beginDrag = (event: MouseEvent<HTMLDivElement>) => {
    if (settings.lockPosition || isInteractive(event.target)) return;
    void getCurrentWindow().startDragging();
  };

  const beginResize = (direction: ResizeDirection) => (event: MouseEvent<HTMLDivElement>) => {
    event.preventDefault();
    if (!settings.lockPosition) void getCurrentWindow().startResizeDragging(direction);
  };

  const serviceAlertSeverity: AlertColor = snapshot.status === 'error' ? 'error' : 'warning';

  return (
    <ThemeProvider theme={muiTheme}>
      <CssBaseline />
      <GlobalStyles styles={{ '*': { boxSizing: 'border-box' } }} />
      <Box
        component="main"
        data-theme-mode={activeTheme}
        // 透明窗口只保留这一层圆角面板；不能再留出透明内边距，否则未开启透明时会暴露成白色外框。
        sx={{ position: 'relative', width: '100%', height: '100%', bgcolor: 'transparent', overflow: 'hidden', userSelect: 'none' }}
      >
        <Paper
          elevation={0}
          sx={{
            height: '100%',
            display: 'flex',
            flexDirection: 'column',
            overflow: 'hidden',
            border: 1,
            borderColor: 'divider',
            // MUI 的数值圆角会乘以主题的 16px 基准值；这里使用明确的单层 16px 圆角。
            borderRadius: '16px',
            boxShadow: 'none',
          }}
        >
          <AppBar
            color="transparent"
            elevation={0}
            position="static"
            onMouseDown={beginDrag}
            sx={{ bgcolor: 'transparent', color: 'text.primary', cursor: settings.lockPosition ? 'default' : 'grab' }}
          >
            <Toolbar variant="dense" sx={{ minHeight: 52, px: 1.5, gap: 1 }}>
              <Stack direction="row" spacing={1} sx={{ alignItems: 'center', flex: 1, minWidth: 0 }}>
                <Box
                  sx={{
                    width: 10,
                    height: 10,
                    borderRadius: '50%',
                    bgcolor:
                      snapshot.status === 'ready'
                        ? 'success.main'
                        : snapshot.status === 'stale'
                          ? 'warning.main'
                          : snapshot.status === 'error'
                            ? 'error.main'
                            : 'action.active',
                    boxShadow: (theme) => `0 0 0 5px ${theme.palette.action.selected}`,
                  }}
                />
                <Typography variant="subtitle2" noWrap sx={{ fontWeight: 800 }}>
                  Codex 用量
                </Typography>
              </Stack>
              <Tooltip title="设置">
                <IconButton
                  aria-label="打开设置"
                  color="inherit"
                  size="small"
                  onClick={(event) => setSettingsAnchor(event.currentTarget)}
                >
                  <SettingsOutlinedIcon fontSize="small" />
                </IconButton>
              </Tooltip>
              <Tooltip title="退出应用">
                <IconButton aria-label="退出应用" color="inherit" size="small" onClick={() => void getCurrentWindow().close()}>
                  <CloseRoundedIcon fontSize="small" />
                </IconButton>
              </Tooltip>
            </Toolbar>
          </AppBar>

          <Box sx={{ px: 1.75, pb: 1.25 }}>
            <Stack direction="row" spacing={1.5} sx={{ justifyContent: 'space-between', alignItems: 'flex-start' }}>
              <Box sx={{ minWidth: 0 }}>
                <Typography variant="overline" color="text.secondary" sx={{ lineHeight: 1.3 }}>
                  {snapshot.planLabel ?? 'Codex 额度'}
                </Typography>
                <Typography component="h1" variant="h6" sx={{ fontWeight: 800, lineHeight: 1.25 }}>
                  {statusLabel(snapshot.status)}
                </Typography>
              </Box>
              <Chip color={chipColor(snapshot.status)} label={chipLabel(snapshot.status)} size="small" />
            </Stack>
            {snapshot.message && (
              <Alert severity={serviceAlertSeverity} variant="outlined" sx={{ mt: 1.25, py: 0.1 }}>
                {snapshot.message}
              </Alert>
            )}
          </Box>

          <Box component="section" aria-label="额度窗口" sx={{ flex: 1, minHeight: 0, overflowY: 'auto', px: 1.75, pb: 1.5 }}>
            {snapshot.quotaWindows.length > 0 ? (
              <Stack spacing={1}>
                {snapshot.quotaWindows.map((quotaWindow) => (
                  <QuotaWindowCard key={quotaWindow.id} now={now} quotaWindow={quotaWindow} />
                ))}
              </Stack>
            ) : (
              <Stack spacing={1} sx={{ minHeight: 130, color: 'text.secondary', textAlign: 'center', alignItems: 'center', justifyContent: 'center' }}>
                {snapshot.status === 'loading' && <CircularProgress size={24} />}
                <Typography variant="body2">
                  {snapshot.status === 'loading' ? '正在从 Codex 读取额度…' : '暂无可显示的额度窗口'}
                </Typography>
              </Stack>
            )}
          </Box>

          <Stack
            direction="row"
            spacing={1}
            sx={{ alignItems: 'center', justifyContent: 'space-between', px: 1.75, py: 1.25, borderTop: 1, borderColor: 'divider' }}
          >
            <Typography variant="caption" color="text.secondary" noWrap>
              上次更新 {formatClock(snapshot.refreshedAt)}
            </Typography>
            <Button
              disabled={isRefreshing}
              size="small"
              startIcon={isRefreshing ? <CircularProgress color="inherit" size={14} /> : <RefreshRoundedIcon />}
              variant="contained"
              onClick={() => void refresh()}
            >
              {isRefreshing ? '刷新中…' : '立即刷新'}
            </Button>
          </Stack>
        </Paper>

        <SettingsPopover
          anchorEl={settingsAnchor}
          autostart={autostart}
          disabled={isSaving}
          isCheckingUpdate={isCheckingUpdate}
          open={Boolean(settingsAnchor)}
          settings={settings}
          onAutostartChange={(enabled) => void updateAutostart(enabled)}
          onCheckUpdate={() => void checkUpdate()}
          onClose={() => setSettingsAnchor(null)}
          onSettingsChange={(next) => void updateSettings(next)}
        />

        {!settings.lockPosition && (
          <>
            <Box aria-hidden onMouseDown={beginResize('East')} sx={{ position: 'absolute', zIndex: 2, top: 24, right: 0, bottom: 24, width: 7, cursor: 'ew-resize' }} />
            <Box aria-hidden onMouseDown={beginResize('South')} sx={{ position: 'absolute', zIndex: 2, right: 24, bottom: 0, left: 24, height: 7, cursor: 'ns-resize' }} />
            <Box aria-hidden onMouseDown={beginResize('SouthEast')} sx={{ position: 'absolute', zIndex: 3, right: 0, bottom: 0, width: 16, height: 16, cursor: 'nwse-resize' }} />
          </>
        )}
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
            <Button variant="contained" onClick={() => void openReleasePage()}>
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
