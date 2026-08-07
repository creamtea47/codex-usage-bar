import ArrowCircleUpRoundedIcon from '@mui/icons-material/ArrowCircleUpRounded';
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
import { useTranslation } from 'react-i18next';

import { usageBridge } from './bridge';
import { formatClock, formatDateTime, formatDuration, statusLabel } from './format';
import { applyLanguagePreference, resolveSupportedLanguage } from './i18n';
import { QuotaWindowCard } from './QuotaWindowCard';
import { hasValidPaceWindow } from './quotaWindowPresentation';
import { createUsageTheme } from './theme';
import {
  defaultSettings,
  loadingSnapshot,
  type AppUpdateInfo,
  type DashboardErrorCode,
  type DashboardSnapshot,
  type Settings,
  type Theme,
} from './types';

type ResizeDirection = 'East' | 'South' | 'SouthEast';

interface Feedback {
  code: 'refreshFailed' | 'openSettingsFailed';
  severity: AlertColor;
}

const dashboardErrorTranslationKeys = {
  authMissing: 'dashboardError.authMissing',
  authInvalid: 'dashboardError.authInvalid',
  network: 'dashboardError.network',
  rateLimited: 'dashboardError.rateLimited',
  serviceUnavailable: 'dashboardError.serviceUnavailable',
  invalidResponse: 'dashboardError.invalidResponse',
  localBridge: 'dashboardError.localBridge',
} as const satisfies Record<DashboardErrorCode, `dashboardError.${DashboardErrorCode}`>;

function isDashboardErrorCode(value: string): value is DashboardErrorCode {
  return Object.hasOwn(dashboardErrorTranslationKeys, value);
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

/**
 * MUI 仅负责桌面悬浮卡呈现。认证文件、网络请求、窗口尺寸策略和本地设置仍全部留在 Rust 后端。
 * 前端只消费 Rust 传回的已脱敏数据，避免认证边界被跨窗口 UI 打破。
 */
export default function App() {
  const { t, i18n } = useTranslation();
  const [snapshot, setSnapshot] = useState<DashboardSnapshot>(loadingSnapshot);
  const [settings, setSettings] = useState<Settings>(defaultSettings);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const [updateNotice, setUpdateNotice] = useState<AppUpdateInfo | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const prefersDark = useMediaQuery('(prefers-color-scheme: dark)', { noSsr: true });
  const language = resolveSupportedLanguage(i18n.resolvedLanguage ?? i18n.language);

  useEffect(() => {
    void applyLanguagePreference(settings.language);
  }, [settings.language]);

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    let disposed = false;
    let removeDashboardListener: (() => void) | undefined;
    let removeSettingsListener: (() => void) | undefined;
    let removeUpdateListener: (() => void) | undefined;
    let dashboardEventReceived = false;
    let settingsEventReceived = false;
    let updateEventReceived = false;
    const removeRegisteredListeners = () => {
      removeDashboardListener?.();
      removeSettingsListener?.();
      removeUpdateListener?.();
      removeDashboardListener = undefined;
      removeSettingsListener = undefined;
      removeUpdateListener = undefined;
    };

    void (async () => {
      try {
        // Register every listener before reading initial state so an event cannot
        // fall into the gap between an older command response and subscription.
        const listenerResults = await Promise.allSettled([
          usageBridge.listenForDashboard((next) => {
            dashboardEventReceived = true;
            if (!disposed) setSnapshot(next);
          }),
          usageBridge.listenForSettings((next) => {
            settingsEventReceived = true;
            if (!disposed) setSettings(next);
          }),
          usageBridge.listenForAppUpdate((next) => {
            updateEventReceived = true;
            if (!disposed) setUpdateNotice(next.updateAvailable ? next : null);
          }),
        ]);
        const [dashboardListenerResult, settingsListenerResult, updateListenerResult] = listenerResults;
        if (dashboardListenerResult.status === 'fulfilled') {
          removeDashboardListener = dashboardListenerResult.value;
        }
        if (settingsListenerResult.status === 'fulfilled') {
          removeSettingsListener = settingsListenerResult.value;
        }
        if (updateListenerResult.status === 'fulfilled') {
          removeUpdateListener = updateListenerResult.value;
        }
        if (disposed) {
          removeRegisteredListeners();
          return;
        }
        if (listenerResults.some((result) => result.status === 'rejected')) {
          removeRegisteredListeners();
          throw new Error('event subscription failed');
        }

        const [dashboard, persistedSettings, pendingUpdate] = await Promise.all([
          usageBridge.getDashboard(),
          usageBridge.getSettings(),
          usageBridge.getAppUpdateInfo(),
        ]);
        if (disposed) return;
        // An event can arrive while the initial command snapshot is still in flight.
        // Keep the event because it represents the newer state.
        if (!dashboardEventReceived) setSnapshot(dashboard);
        if (!settingsEventReceived) setSettings(persistedSettings);
        if (!updateEventReceived) setUpdateNotice(pendingUpdate.updateAvailable ? pendingUpdate : null);
      } catch {
        if (!disposed) {
          setSnapshot((current) => ({
            ...current,
            status: current.quotaWindows.length ? 'stale' : 'error',
            message: 'localBridge',
          }));
        }
      }
    })();

    return () => {
      disposed = true;
      removeRegisteredListeners();
    };
  }, []);

  const activeTheme = useMemo(() => resolveTheme(settings.theme, prefersDark), [settings.theme, prefersDark]);
  const muiTheme = useMemo(() => createUsageTheme(activeTheme), [activeTheme]);
  const isReady = snapshot.status === 'ready';
  const accountSummary =
    [snapshot.accountEmailMasked, snapshot.planLabel].filter((part): part is string => Boolean(part)).join(' · ') ||
    t('app.quotaTitle');
  const messageCode: string | null = snapshot.message;
  const dashboardMessage = messageCode
    ? t(isDashboardErrorCode(messageCode) ? dashboardErrorTranslationKeys[messageCode] : 'dashboardError.generic')
    : null;
  const forecastSuggestions = useMemo(
    () =>
      snapshot.quotaWindows.flatMap((quotaWindow) => {
        // Rust 用节奏标记区分长周期窗口；短周期、采集中和稳定状态不生成底部建议。
        if (!hasValidPaceWindow(quotaWindow) || !quotaWindow.forecast) return [];

        let forecastMessage: string | null = null;
        if (quotaWindow.forecast.status === 'lastsUntilReset') {
          forecastMessage = t('quota.forecast.lastsUntilReset');
        } else if (
          quotaWindow.forecast.status === 'exhaustsBeforeReset' &&
          quotaWindow.forecast.exhaustsAt &&
          Number.isFinite(new Date(quotaWindow.forecast.exhaustsAt).getTime())
        ) {
          forecastMessage = t('quota.forecast.exhaustsAt', {
            date: formatDateTime(quotaWindow.forecast.exhaustsAt, language),
          });
        }
        if (!forecastMessage) return [];

        const label =
          quotaWindow.label ??
          (quotaWindow.fallbackLabel === 'fiveHour'
            ? t('quota.fallbackLabel.fiveHour')
            : quotaWindow.fallbackLabel === 'weekly'
              ? t('quota.fallbackLabel.weekly')
              : t('quota.fallbackLabel.window', {
                  duration: formatDuration(quotaWindow.windowSeconds, language),
                }));
        const message = t('quota.forecast.suggestion', { label, forecast: forecastMessage });
        return [{ id: quotaWindow.id, message }];
      }),
    [language, snapshot.quotaWindows, t],
  );

  const refresh = async () => {
    if (isRefreshing) return;
    setIsRefreshing(true);
    try {
      setSnapshot(await usageBridge.refreshDashboard());
    } catch {
      setFeedback({ code: 'refreshFailed', severity: 'error' });
    } finally {
      setIsRefreshing(false);
    }
  };

  const openSettings = async (section?: 'about') => {
    try {
      await usageBridge.openSettingsWindow(section);
    } catch {
      setFeedback({ code: 'openSettingsFailed', severity: 'error' });
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
                  aria-label={isReady ? t('app.serviceOnline') : t('app.serviceUnavailable')}
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
                  {t('app.title')}
                </Typography>
              </Stack>
              <Box
                component="span"
                role="status"
                aria-live="polite"
                aria-atomic="true"
                sx={{
                  position: 'absolute',
                  width: 1,
                  height: 1,
                  p: 0,
                  m: -1,
                  overflow: 'hidden',
                  clipPath: 'inset(50%)',
                  whiteSpace: 'nowrap',
                  border: 0,
                }}
              >
                {updateNotice ? t('app.updateAvailableStatus', { version: updateNotice.latestVersion }) : ''}
              </Box>
              {updateNotice && (
                <Tooltip title={t('app.updateAvailableTooltip', { version: updateNotice.latestVersion })}>
                  <IconButton
                    aria-label={t('app.updateAvailableAria', { version: updateNotice.latestVersion })}
                    color="success"
                    size="small"
                    onClick={() => void openSettings('about')}
                  >
                    <ArrowCircleUpRoundedIcon fontSize="small" />
                  </IconButton>
                </Tooltip>
              )}
              <Tooltip title={isRefreshing ? t('app.refreshing') : t('app.refresh')}>
                <span>
                  <IconButton
                    aria-label={t('app.refresh')}
                    color="inherit"
                    disabled={isRefreshing}
                    size="small"
                    onClick={() => void refresh()}
                  >
                    {isRefreshing ? <CircularProgress color="inherit" size={17} /> : <RefreshRoundedIcon fontSize="small" />}
                  </IconButton>
                </span>
              </Tooltip>
              <Tooltip title={t('app.settings')}>
                <IconButton aria-label={t('app.openSettings')} color="inherit" size="small" onClick={() => void openSettings()}>
                  <SettingsOutlinedIcon fontSize="small" />
                </IconButton>
              </Tooltip>
              <Tooltip title={t('app.quit')}>
                <IconButton aria-label={t('app.quit')} color="inherit" size="small" onClick={() => void getCurrentWindow().close()}>
                  <CloseRoundedIcon fontSize="small" />
                </IconButton>
              </Tooltip>
            </Toolbar>
          </AppBar>

          <Box sx={{ px: 1.75, pb: 1.1 }}>
            {isReady ? (
              <Stack spacing={0.25}>
                <Typography variant="overline" color="text.secondary" sx={{ lineHeight: 1.15 }}>
                  {t('app.quotaTitle')}
                </Typography>
                <Typography component="h1" variant="h6" noWrap sx={{ fontWeight: 800, lineHeight: 1.25 }}>
                  {accountSummary}
                </Typography>
                <Typography variant="caption" color="text.secondary" noWrap>
                  {t('app.nextRefresh', {
                    countdown: formatCountdown(snapshot.nextRefreshAt, now),
                    time: formatClock(snapshot.refreshedAt, language),
                  })}
                </Typography>
              </Stack>
            ) : (
              <Stack spacing={0.25}>
                <Typography variant="overline" color="text.secondary" sx={{ lineHeight: 1.15 }}>
                  {snapshot.planLabel ?? t('app.quotaTitle')}
                </Typography>
                <Typography component="h1" variant="h6" sx={{ fontWeight: 800, lineHeight: 1.25 }}>
                  {statusLabel(snapshot.status)}
                </Typography>
                {snapshot.nextRefreshAt ? (
                  <Typography variant="caption" color="text.secondary">
                    {t('app.retryNext', {
                      countdown: formatCountdown(snapshot.nextRefreshAt, now),
                      time: formatClock(snapshot.refreshedAt, language),
                    })}
                  </Typography>
                ) : snapshot.refreshedAt ? (
                  <Typography variant="caption" color="text.secondary">
                    {t('app.lastUpdated', { time: formatClock(snapshot.refreshedAt, language) })}
                  </Typography>
                ) : null}
              </Stack>
            )}
            {dashboardMessage && (
              <Alert severity={serviceAlertSeverity} variant="outlined" sx={{ mt: 1, py: 0.1 }}>
                {dashboardMessage}
              </Alert>
            )}
          </Box>

          <Box component="section" aria-label={t('app.quotaWindowsAria')} sx={{ flex: 1, minHeight: 0, overflowY: 'auto', px: 1.75, pb: 1.5 }}>
            {snapshot.quotaWindows.length > 0 ? (
              <Stack spacing={1}>
                {snapshot.quotaWindows.map((quotaWindow) => (
                  <QuotaWindowCard
                    key={quotaWindow.id}
                    now={now}
                    quotaWindow={quotaWindow}
                    refreshedAt={snapshot.refreshedAt}
                  />
                ))}
              </Stack>
            ) : (
              <Stack
                spacing={1}
                sx={{ minHeight: 116, color: 'text.secondary', textAlign: 'center', alignItems: 'center', justifyContent: 'center' }}
              >
                {snapshot.status === 'loading' && <CircularProgress aria-label={t('app.loadingQuota')} size={22} />}
                <Typography variant="body2">
                  {snapshot.status === 'loading' ? t('app.loadingQuota') : t('app.noQuotaWindows')}
                </Typography>
              </Stack>
            )}
          </Box>

          {forecastSuggestions.length > 0 && (
            <Box
              component="section"
              aria-label={t('quota.forecast.suggestionsAria')}
              sx={{
                flex: '0 0 auto',
                height: 16 + Math.min(forecastSuggestions.length, 3) * 20,
                overflowX: 'hidden',
                overflowY: forecastSuggestions.length > 3 ? 'auto' : 'hidden',
                boxShadow: (theme) => `inset 0 1px 0 ${theme.palette.divider}`,
                px: 1.75,
                py: 1,
              }}
            >
              {forecastSuggestions.map(({ id, message }) => (
                <Tooltip key={id} title={message} placement="top">
                  <Typography
                    data-forecast-suggestion
                    noWrap
                    variant="caption"
                    color="text.secondary"
                    sx={{ display: 'block', height: 20, lineHeight: '20px' }}
                  >
                    {message}
                  </Typography>
                </Tooltip>
              ))}
            </Box>
          )}
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
        <Alert
          closeText={t('common.close')}
          severity={feedback?.severity ?? 'info'}
          variant="filled"
          onClose={() => setFeedback(null)}
        >
          {feedback?.code === 'refreshFailed'
            ? t('app.feedback.refreshFailed')
            : feedback?.code === 'openSettingsFailed'
              ? t('app.feedback.openSettingsFailed')
              : null}
        </Alert>
      </Snackbar>

    </ThemeProvider>
  );
}
