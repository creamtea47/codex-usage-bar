import CloseRoundedIcon from '@mui/icons-material/CloseRounded';
import RefreshRoundedIcon from '@mui/icons-material/RefreshRounded';
import SettingsOutlinedIcon from '@mui/icons-material/SettingsOutlined';
import {
  Alert,
  AppBar,
  Box,
  CircularProgress,
  CssBaseline,
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
} from '@mui/material';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useMemo, useState, type MouseEvent } from 'react';

import { usageBridge } from './bridge';
import { formatClock, statusLabel } from './format';
import { QuotaWindowCard } from './QuotaWindowCard';
import { createUsageTheme } from './theme';
import {
  defaultSettings,
  loadingSnapshot,
  type DashboardSnapshot,
  type Settings,
  type Theme,
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

function formatCountdown(nextRefreshAt: string | null, now: number): string {
  if (!nextRefreshAt) return '—';
  const next = new Date(nextRefreshAt).getTime();
  if (Number.isNaN(next)) return '—';

  const totalSeconds = Math.max(0, Math.ceil((next - now) / 1_000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`;
}

function accountSummary(snapshot: DashboardSnapshot): string {
  return [snapshot.accountEmailMasked, snapshot.planLabel].filter((part): part is string => Boolean(part)).join(' · ') || 'Codex 额度';
}

/**
 * MUI 仅负责桌面悬浮卡呈现。认证文件、网络请求、窗口尺寸策略和本地设置仍全部留在 Rust 后端。
 * 前端只消费 Rust 传回的已脱敏数据，避免认证边界被跨窗口 UI 打破。
 */
export default function App() {
  const [snapshot, setSnapshot] = useState<DashboardSnapshot>(loadingSnapshot);
  const [settings, setSettings] = useState<Settings>(defaultSettings);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const prefersDark = useMediaQuery('(prefers-color-scheme: dark)', { noSsr: true });

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    let disposed = false;
    let removeDashboardListener: (() => void) | undefined;
    let removeSettingsListener: (() => void) | undefined;

    void (async () => {
      try {
        const [dashboard, persistedSettings, removeDashboard, removeSettings] = await Promise.all([
          usageBridge.getDashboard(),
          usageBridge.getSettings(),
          usageBridge.listenForDashboard((next) => {
            if (!disposed) setSnapshot(next);
          }),
          usageBridge.listenForSettings((next) => {
            if (!disposed) setSettings(next);
          }),
        ]);
        if (disposed) {
          removeDashboard();
          removeSettings();
          return;
        }
        setSnapshot(dashboard);
        setSettings(persistedSettings);
        removeDashboardListener = removeDashboard;
        removeSettingsListener = removeSettings;
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
      removeDashboardListener?.();
      removeSettingsListener?.();
    };
  }, []);

  const activeTheme = useMemo(() => resolveTheme(settings.theme, prefersDark), [settings.theme, prefersDark]);
  const muiTheme = useMemo(() => createUsageTheme(activeTheme), [activeTheme]);
  const isReady = snapshot.status === 'ready';

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

  const openSettings = async () => {
    try {
      await usageBridge.openSettingsWindow();
    } catch {
      setFeedback({ message: '无法打开设置窗口，请稍后重试。', severity: 'error' });
    }
  };

  const beginDrag = (event: MouseEvent<HTMLDivElement>) => {
    if (settings.lockPosition || isInteractive(event.target)) return;
    void getCurrentWindow().startDragging();
  };

  const beginResize = (direction: ResizeDirection) => (event: MouseEvent<HTMLDivElement>) => {
    event.preventDefault();
    if (!settings.lockPosition) {
      // 仅由用户真实开始边缘拖拽触发手动模式；失败不应阻断原生缩放手势。
      void usageBridge.markMainSizeManual().catch(() => undefined);
      void getCurrentWindow().startResizeDragging(direction);
    }
  };

  const serviceAlertSeverity: AlertColor = snapshot.status === 'error' ? 'error' : 'warning';

  return (
    <ThemeProvider theme={muiTheme}>
      <CssBaseline />
      <GlobalStyles styles={{ '*': { boxSizing: 'border-box' } }} />
      <Box
        component="main"
        data-theme-mode={activeTheme}
        // 透明主窗口只有这一层圆角面板，避免透明区域与不透明底色叠出白色外框。
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
            <Toolbar variant="dense" sx={{ minHeight: 48, px: 1.5, gap: 0.25 }}>
              <Stack direction="row" spacing={1} sx={{ alignItems: 'center', flex: 1, minWidth: 0 }}>
                <Box
                  aria-label={isReady ? '用量服务在线' : '用量服务状态异常'}
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
              <Tooltip title={isRefreshing ? '正在刷新用量' : '刷新用量'}>
                <span>
                  <IconButton
                    aria-label="刷新用量"
                    color="inherit"
                    disabled={isRefreshing}
                    size="small"
                    onClick={() => void refresh()}
                  >
                    {isRefreshing ? <CircularProgress color="inherit" size={17} /> : <RefreshRoundedIcon fontSize="small" />}
                  </IconButton>
                </span>
              </Tooltip>
              <Tooltip title="设置">
                <IconButton aria-label="打开设置" color="inherit" size="small" onClick={() => void openSettings()}>
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

          <Box sx={{ px: 1.75, pb: 1.1 }}>
            {isReady ? (
              <Stack spacing={0.25}>
                <Typography variant="overline" color="text.secondary" sx={{ lineHeight: 1.15 }}>
                  Codex 额度
                </Typography>
                <Typography component="h1" variant="h6" noWrap sx={{ fontWeight: 800, lineHeight: 1.25 }}>
                  {accountSummary(snapshot)}
                </Typography>
                <Typography variant="caption" color="text.secondary" noWrap>
                  下次自动刷新 {formatCountdown(snapshot.nextRefreshAt, now)} · 更新于 {formatClock(snapshot.refreshedAt)}
                </Typography>
              </Stack>
            ) : (
              <Stack spacing={0.25}>
                <Typography variant="overline" color="text.secondary" sx={{ lineHeight: 1.15 }}>
                  {snapshot.planLabel ?? 'Codex 额度'}
                </Typography>
                <Typography component="h1" variant="h6" sx={{ fontWeight: 800, lineHeight: 1.25 }}>
                  {statusLabel(snapshot.status)}
                </Typography>
                {snapshot.refreshedAt && (
                  <Typography variant="caption" color="text.secondary">
                    上次更新 {formatClock(snapshot.refreshedAt)}
                  </Typography>
                )}
              </Stack>
            )}
            {snapshot.message && (
              <Alert severity={serviceAlertSeverity} variant="outlined" sx={{ mt: 1, py: 0.1 }}>
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
              <Stack
                spacing={1}
                sx={{ minHeight: 116, color: 'text.secondary', textAlign: 'center', alignItems: 'center', justifyContent: 'center' }}
              >
                {snapshot.status === 'loading' && <CircularProgress size={22} />}
                <Typography variant="body2">
                  {snapshot.status === 'loading' ? '正在从 Codex 读取额度…' : '暂无可显示的额度窗口'}
                </Typography>
              </Stack>
            )}
          </Box>
        </Paper>

        {!settings.lockPosition && (
          <>
            <Box aria-hidden data-resize-direction="East" onMouseDown={beginResize('East')} sx={{ position: 'absolute', zIndex: 2, top: 20, right: 0, bottom: 20, width: 7, cursor: 'ew-resize' }} />
            <Box aria-hidden data-resize-direction="South" onMouseDown={beginResize('South')} sx={{ position: 'absolute', zIndex: 2, right: 20, bottom: 0, left: 20, height: 7, cursor: 'ns-resize' }} />
            <Box aria-hidden data-resize-direction="SouthEast" onMouseDown={beginResize('SouthEast')} sx={{ position: 'absolute', zIndex: 3, right: 0, bottom: 0, width: 16, height: 16, cursor: 'nwse-resize' }} />
          </>
        )}
      </Box>

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
