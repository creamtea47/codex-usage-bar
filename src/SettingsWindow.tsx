import BugReportOutlinedIcon from '@mui/icons-material/BugReportOutlined';
import ContentCopyRoundedIcon from '@mui/icons-material/ContentCopyRounded';
import InfoOutlinedIcon from '@mui/icons-material/InfoOutlined';
import LaunchRoundedIcon from '@mui/icons-material/LaunchRounded';
import NotificationsOutlinedIcon from '@mui/icons-material/NotificationsOutlined';
import PaletteOutlinedIcon from '@mui/icons-material/PaletteOutlined';
import QueryStatsRoundedIcon from '@mui/icons-material/QueryStatsRounded';
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
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { openUrl } from '@tauri-apps/plugin-opener';
import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { usageBridge } from './bridge';
import { formatDateTime } from './format';
import { applyLanguagePreference, resolveSupportedLanguage } from './i18n';
import NotificationsPage from './NotificationsPage';
import { createUsageTheme } from './theme';
import TrendsErrorBoundary from './TrendsErrorBoundary';
import {
  defaultSettings,
  type AppUpdateInfo,
  type AppUpdateProgress,
  type Language,
  type NotificationSettings,
  type Settings,
  type Theme,
} from './types';

const TrendsPage = lazy(() => import('./TrendsPage'));

type SettingsSection = 'display' | 'data' | 'notifications' | 'trends' | 'startup' | 'about';
type SettingsLoadState = 'loading' | 'ready' | 'error';
type SettingsUpdater = Partial<Settings> | ((current: Settings) => Settings);
type FeedbackKey =
  | 'settings.notifications.permissionDenied'
  | 'settings.notifications.permissionUnavailable'
  | 'settings.feedback.loadFailed'
  | 'settings.feedback.nativeThemeFailed'
  | 'settings.feedback.saved'
  | 'settings.feedback.saveFailed'
  | 'settings.feedback.autostartSaved'
  | 'settings.feedback.autostartFailed'
  | 'settings.feedback.updateCheckFailed'
  | 'settings.feedback.updateInstallStarted'
  | 'settings.feedback.updateInstallFailed'
  | 'settings.feedback.releaseOpenFailed'
  | 'settings.feedback.signatureFailed'
  | 'settings.feedback.noCompatibleUpdate'
  | 'settings.feedback.updateNetworkFailed'
  | 'settings.feedback.updateConfigFailed'
  | 'settings.feedback.updateIncomplete'
  | 'settings.feedback.updateBusy'
  | 'settings.feedback.noPendingUpdate'
  | 'settings.notifications.feedback.enabled'
  | 'settings.notifications.feedback.disabled'
  | 'settings.notifications.feedback.testSent'
  | 'settings.notifications.feedback.testFailed'
  | 'settings.feedback.diagnosticsCopied'
  | 'settings.feedback.diagnosticsCopyFailed'
  | 'settings.feedback.issueOpenFailed'
  | 'settings.feedback.releaseNotesOpenFailed'
  | 'settings.feedback.repositoryOpenFailed';

interface Feedback {
  key: FeedbackKey;
  severity: AlertColor;
}

const drawerWidth = 208;
const repositoryPageUrl = 'https://github.com/creamtea47/codex-usage-bar';
const releasePageUrl = 'https://github.com/creamtea47/codex-usage-bar/releases';
const issuePageUrl = 'https://github.com/creamtea47/codex-usage-bar/issues/new/choose';
const releaseVersionPattern = /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/;

const refreshOptions = [60, 180, 300, 600, 1800] as const;

const navigation: Array<{
  id: SettingsSection;
  labelKey:
    | 'settings.nav.display'
    | 'settings.nav.data'
    | 'settings.nav.notifications'
    | 'settings.nav.trends'
    | 'settings.nav.startup'
    | 'settings.nav.about';
  icon: React.ReactElement;
}> = [
  { id: 'display', labelKey: 'settings.nav.display', icon: <PaletteOutlinedIcon fontSize="small" /> },
  { id: 'data', labelKey: 'settings.nav.data', icon: <RefreshRoundedIcon fontSize="small" /> },
  {
    id: 'notifications',
    labelKey: 'settings.nav.notifications',
    icon: <NotificationsOutlinedIcon fontSize="small" />,
  },
  { id: 'trends', labelKey: 'settings.nav.trends', icon: <QueryStatsRoundedIcon fontSize="small" /> },
  { id: 'startup', labelKey: 'settings.nav.startup', icon: <RocketLaunchOutlinedIcon fontSize="small" /> },
  { id: 'about', labelKey: 'settings.nav.about', icon: <InfoOutlinedIcon fontSize="small" /> },
];

const knownUpdateErrors = [
  ['更新包的签名验证失败，已取消安装。', 'settings.feedback.signatureFailed'],
  ['暂时没有适用于当前设备的有效更新。', 'settings.feedback.noCompatibleUpdate'],
  ['无法连接更新服务，请检查网络或代理后重试。', 'settings.feedback.updateNetworkFailed'],
  ['更新服务配置不可用，请稍后重试。', 'settings.feedback.updateConfigFailed'],
  ['更新安装未完成，请稍后重试。', 'settings.feedback.updateIncomplete'],
  ['更新操作正在进行，请稍后再试。', 'settings.feedback.updateBusy'],
  ['没有待安装的更新，请先重新检查更新。', 'settings.feedback.noPendingUpdate'],
] as const satisfies ReadonlyArray<readonly [string, FeedbackKey]>;

function resolveTheme(theme: Theme, prefersDark: boolean): Exclude<Theme, 'system'> {
  return theme === 'system' ? (prefersDark ? 'dark' : 'light') : theme;
}

function safeUpdateErrorKey(error: unknown, fallback: FeedbackKey): FeedbackKey {
  const message = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
  return knownUpdateErrors.find(([safeMessage]) => message.includes(safeMessage))?.[1] ?? fallback;
}

function releaseNotesUrl(version: string | null | undefined): string {
  return version && releaseVersionPattern.test(version) ? `${releasePageUrl}/tag/v${version}` : releasePageUrl;
}

/** Independent settings window; all sensitive operations remain in label-gated Rust commands. */
export default function SettingsWindow() {
  const { t, i18n } = useTranslation();
  const [settings, setSettings] = useState<Settings>(defaultSettings);
  const settingsRef = useRef<Settings>(defaultSettings);
  const settingsLoadedRef = useRef(false);
  const settingsMutationTail = useRef<Promise<void>>(Promise.resolve());
  const pendingSettingsMutations = useRef(0);
  const [autostart, setAutostart] = useState(false);
  const [autostartLoaded, setAutostartLoaded] = useState(false);
  const [activeSection, setActiveSection] = useState<SettingsSection>('display');
  const [settingsLoadState, setSettingsLoadState] = useState<SettingsLoadState>('loading');
  const [isSaving, setIsSaving] = useState(false);
  const [isChangingAutostart, setIsChangingAutostart] = useState(false);
  const [isSendingTestNotification, setIsSendingTestNotification] = useState(false);
  const [isCheckingUpdate, setIsCheckingUpdate] = useState(false);
  const [isInstallingUpdate, setIsInstallingUpdate] = useState(false);
  const [isCopyingDiagnostics, setIsCopyingDiagnostics] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<AppUpdateInfo | null>(null);
  const [updateProgress, setUpdateProgress] = useState<AppUpdateProgress | null>(null);
  const [isUpdateDialogOpen, setIsUpdateDialogOpen] = useState(false);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const prefersDark = useMediaQuery('(prefers-color-scheme: dark)', { noSsr: true });
  const language = resolveSupportedLanguage(i18n.resolvedLanguage ?? i18n.language);

  const applySettingsSnapshot = useCallback((next: Settings) => {
    settingsRef.current = next;
    setSettings(next);
  }, []);

  const enqueueSettingsMutation = useCallback(
    <T,>(mutation: (current: Settings) => Promise<{ settings: Settings; value: T }>): Promise<T> => {
      pendingSettingsMutations.current += 1;
      setIsSaving(true);

      const result = settingsMutationTail.current.then(async () => {
        if (!settingsLoadedRef.current) throw new Error('settings-not-loaded');
        const completed = await mutation(settingsRef.current);
        applySettingsSnapshot(completed.settings);
        return completed.value;
      });

      settingsMutationTail.current = result.then(
        () => undefined,
        () => undefined,
      );

      return result.finally(() => {
        pendingSettingsMutations.current = Math.max(0, pendingSettingsMutations.current - 1);
        setIsSaving(pendingSettingsMutations.current > 0);
      });
    },
    [applySettingsSnapshot],
  );

  useEffect(() => {
    void applyLanguagePreference(settings.language);
  }, [settings.language]);

  useEffect(() => {
    let disposed = false;
    const removeListeners: Array<() => void> = [];
    let settingsEventReceived = false;
    let updateEventReceived = false;

    void (async () => {
      const registerListener = async (listener: Promise<() => void>) => {
        try {
          const remove = await listener;
          if (disposed) remove();
          else removeListeners.push(remove);
        } catch {
          // A failed optional event stream must not prevent the authoritative reads below.
        }
      };

      await Promise.all([
        registerListener(
          usageBridge.listenForSettings((next) => {
            settingsEventReceived = true;
            if (!disposed) applySettingsSnapshot(next);
          }),
        ),
        registerListener(
          usageBridge.listenForAppUpdate((next) => {
            updateEventReceived = true;
            if (!disposed) setUpdateInfo(next);
          }),
        ),
        registerListener(
          usageBridge.listenForAppUpdateProgress((next) => {
            if (!disposed) setUpdateProgress(next);
          }),
        ),
        registerListener(
          usageBridge.listenForSettingsNavigation((section) => {
            if (!disposed) {
              setActiveSection(section);
              setIsUpdateDialogOpen(section === 'about');
            }
          }),
        ),
      ]);

      if (disposed) return;

      const [settingsResult, autostartResult, updateResult] = await Promise.allSettled([
        usageBridge.getSettings(),
        usageBridge.getAutostart(),
        usageBridge.getAppUpdateInfo(),
      ]);
      if (disposed) return;

      if (settingsResult.status === 'fulfilled') {
        if (!settingsEventReceived) applySettingsSnapshot(settingsResult.value);
        settingsLoadedRef.current = true;
        setSettingsLoadState('ready');
      } else {
        settingsLoadedRef.current = false;
        setSettingsLoadState('error');
        setFeedback({ key: 'settings.feedback.loadFailed', severity: 'error' });
      }
      if (autostartResult.status === 'fulfilled') {
        setAutostart(autostartResult.value);
        setAutostartLoaded(true);
      }
      if (updateResult.status === 'fulfilled' && !updateEventReceived) {
        setUpdateInfo(updateResult.value);
      }
    })();

    return () => {
      disposed = true;
      removeListeners.splice(0).forEach((remove) => remove());
    };
  }, [applySettingsSnapshot]);

  const activeTheme = useMemo(() => resolveTheme(settings.theme, prefersDark), [settings.theme, prefersDark]);
  const muiTheme = useMemo(() => createUsageTheme(activeTheme), [activeTheme]);
  const nativeTheme = settings.theme === 'system' ? null : settings.theme;
  const settingsControlsDisabled = settingsLoadState !== 'ready';

  useEffect(() => {
    const currentWindow = getCurrentWindow();
    void Promise.all([
      currentWindow.setTheme(nativeTheme),
      currentWindow.setBackgroundColor(muiTheme.palette.background.paper),
    ]).catch(() => setFeedback({ key: 'settings.feedback.nativeThemeFailed', severity: 'error' }));
  }, [muiTheme.palette.background.paper, nativeTheme]);

  const updateSettings = async (updater: SettingsUpdater): Promise<boolean> => {
    try {
      await enqueueSettingsMutation(async (current) => {
        const next = typeof updater === 'function' ? updater(current) : { ...current, ...updater };
        return { settings: await usageBridge.saveSettings(next), value: undefined };
      });
      setFeedback({ key: 'settings.feedback.saved', severity: 'success' });
      return true;
    } catch {
      setFeedback({ key: 'settings.feedback.saveFailed', severity: 'error' });
      return false;
    }
  };

  const updateLanguage = async (nextLanguage: Language) => {
    const previousLanguage = settingsRef.current.language;
    await applyLanguagePreference(nextLanguage);
    if (!(await updateSettings((current) => ({ ...current, language: nextLanguage })))) {
      await applyLanguagePreference(previousLanguage);
    }
  };

  const updateNotificationSettings = (partial: Partial<NotificationSettings>) =>
    updateSettings((current) => ({
      ...current,
      notifications: { ...current.notifications, ...partial },
    }));

  const updateNotificationsEnabled = async (enabled: boolean) => {
    try {
      const result = await enqueueSettingsMutation(async () => {
        const next = await usageBridge.setNotificationsEnabled(enabled);
        return { settings: next.settings, value: next };
      });
      if (enabled && !result.enabled) {
        setFeedback({
          key:
            result.permission === 'denied'
              ? 'settings.notifications.permissionDenied'
              : 'settings.notifications.permissionUnavailable',
          severity: 'warning',
        });
      } else {
        setFeedback({
          key: enabled ? 'settings.notifications.feedback.enabled' : 'settings.notifications.feedback.disabled',
          severity: 'success',
        });
      }
    } catch {
      setFeedback({ key: 'settings.notifications.permissionUnavailable', severity: 'error' });
    }
  };

  const updateHistoryEnabled = useCallback(
    async (enabled: boolean) => {
      await enqueueSettingsMutation(async () => ({
        settings: await usageBridge.setHistoryEnabled(enabled),
        value: undefined,
      }));
    },
    [enqueueSettingsMutation],
  );

  const sendTestNotification = async () => {
    if (isSendingTestNotification) return;
    setIsSendingTestNotification(true);
    try {
      await usageBridge.sendTestNotification();
      setFeedback({ key: 'settings.notifications.feedback.testSent', severity: 'success' });
    } catch {
      setFeedback({ key: 'settings.notifications.feedback.testFailed', severity: 'error' });
    } finally {
      setIsSendingTestNotification(false);
    }
  };

  const updateAutostart = async (enabled: boolean) => {
    if (isChangingAutostart) return;
    setIsChangingAutostart(true);
    try {
      setAutostart(await usageBridge.setAutostart(enabled));
      setFeedback({ key: 'settings.feedback.autostartSaved', severity: 'success' });
    } catch {
      setFeedback({ key: 'settings.feedback.autostartFailed', severity: 'error' });
    } finally {
      setIsChangingAutostart(false);
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
      setFeedback({ key: safeUpdateErrorKey(error, 'settings.feedback.updateCheckFailed'), severity: 'error' });
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
      setFeedback({ key: 'settings.feedback.updateInstallStarted', severity: 'success' });
    } catch (error) {
      setFeedback({ key: safeUpdateErrorKey(error, 'settings.feedback.updateInstallFailed'), severity: 'error' });
    } finally {
      setIsInstallingUpdate(false);
    }
  };

  const openExternal = async (url: string, failureKey: FeedbackKey) => {
    try {
      await openUrl(url);
    } catch {
      setFeedback({ key: failureKey, severity: 'error' });
    }
  };

  const copyDiagnostics = async () => {
    if (isCopyingDiagnostics) return;
    setIsCopyingDiagnostics(true);
    try {
      const diagnostics = await usageBridge.getDiagnostics();
      await writeText(diagnostics);
      setFeedback({ key: 'settings.feedback.diagnosticsCopied', severity: 'success' });
    } catch {
      setFeedback({ key: 'settings.feedback.diagnosticsCopyFailed', severity: 'error' });
    } finally {
      setIsCopyingDiagnostics(false);
    }
  };

  const progressLabel = !updateProgress
    ? t('settings.updateDialog.preparing')
    : updateProgress.stage === 'verifying'
      ? t('settings.updateDialog.verifying')
      : updateProgress.stage === 'installing'
        ? t('settings.updateDialog.handingOff')
        : updateProgress.totalBytes && updateProgress.totalBytes > 0
          ? t('settings.updateDialog.downloadingPercent', {
              percent: Math.min(100, Math.round((updateProgress.downloadedBytes / updateProgress.totalBytes) * 100)),
            })
          : t('settings.updateDialog.downloading');

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
                  {t('settings.title')}
                </Typography>
              </Box>
            </Stack>
          </Toolbar>
          <Divider />
          <List aria-label={t('settings.navigationAria')} disablePadding sx={{ px: 1, py: 1 }}>
            {navigation.map((item) => {
              const label = t(item.labelKey);
              return (
                <ListItemButton
                  key={item.id}
                  selected={activeSection === item.id}
                  onClick={() => setActiveSection(item.id)}
                  sx={{ borderRadius: 1.5, mb: 0.5 }}
                >
                  <ListItemIcon sx={{ minWidth: 34 }}>{item.icon}</ListItemIcon>
                  <ListItemText
                    primary={label}
                    slotProps={{ primary: { sx: { fontSize: 14, fontWeight: activeSection === item.id ? 700 : 500 } } }}
                  />
                </ListItemButton>
              );
            })}
          </List>
        </Drawer>

        <Box
          component="section"
          aria-label={t('settings.contentAria')}
          sx={{ flex: 1, minWidth: 0, overflow: 'auto', bgcolor: 'background.default' }}
        >
          <Box
            sx={{
              width: '100%',
              maxWidth: activeSection === 'trends' ? 980 : 680,
              mx: 'auto',
              px: { xs: 2, sm: 4 },
              py: 4,
            }}
          >
            {settingsLoadState === 'loading' && <LinearProgress sx={{ mb: 2 }} />}
            {settingsLoadState === 'error' && (
              <Alert severity="error" sx={{ mb: 2 }}>
                {t('settings.feedback.loadFailed')}
              </Alert>
            )}

            {activeSection === 'display' && (
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
              </Stack>
            )}

            {activeSection === 'data' && (
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
                        onChange={(event) => void updateSettings({ refreshIntervalSeconds: Number(event.target.value) })}
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
            )}

            {activeSection === 'notifications' && (
              <NotificationsPage
                notifications={settings.notifications}
                disabled={settingsControlsDisabled}
                isSaving={isSaving}
                isSendingTestNotification={isSendingTestNotification}
                onSetEnabled={updateNotificationsEnabled}
                onUpdate={updateNotificationSettings}
                onSendTest={sendTestNotification}
              />
            )}

            {activeSection === 'trends' && settingsLoadState === 'ready' && (
              <TrendsErrorBoundary>
                <Suspense fallback={<LinearProgress aria-label={t('trends.loading')} />}>
                  <TrendsPage
                    historyEnabled={settings.historyEnabled}
                    getUsageHistory={usageBridge.getUsageHistory}
                    onSetHistoryEnabled={updateHistoryEnabled}
                    onClearUsageHistory={usageBridge.clearUsageHistory}
                    listenForUsageHistory={usageBridge.listenForUsageHistory}
                  />
                </Suspense>
              </TrendsErrorBoundary>
            )}

            {activeSection === 'startup' && (
              <Stack spacing={2.5}>
                <Box>
                  <Typography component="h1" variant="h5" sx={{ fontWeight: 800 }}>
                    {t('settings.startup.title')}
                  </Typography>
                  <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
                    {t('settings.startup.subtitle')}
                  </Typography>
                </Box>
                <Paper variant="outlined" sx={{ p: 2 }}>
                  <FormControlLabel
                    control={
                      <Switch
                        checked={autostart}
                        disabled={!autostartLoaded || isChangingAutostart}
                        onChange={(event) => void updateAutostart(event.target.checked)}
                      />
                    }
                    label={t('settings.startup.autostart')}
                  />
                  <Typography variant="caption" color="text.secondary" sx={{ display: 'block', pl: 4 }}>
                    {t('settings.startup.description')}
                  </Typography>
                </Paper>
              </Stack>
            )}

            {activeSection === 'about' && (
              <Stack spacing={2.5}>
                <Box>
                  <Typography component="h1" variant="h5" sx={{ fontWeight: 800 }}>
                    {t('settings.about.title')}
                  </Typography>
                  <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
                    {t('settings.about.subtitle')}
                  </Typography>
                </Box>
                <Paper variant="outlined" sx={{ p: 2 }}>
                  <Stack spacing={1.5}>
                    <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                      {t('settings.about.updateTitle')}
                    </Typography>
                    <Typography variant="body2" color="text.secondary">
                      {t('settings.about.currentVersion', { version: updateInfo?.currentVersion ?? t('common.unavailable') })}
                    </Typography>
                    <FormControlLabel
                      control={
                        <Switch
                          checked={settings.autoCheckUpdates}
                          disabled={settingsControlsDisabled}
                          onChange={(event) => void updateSettings({ autoCheckUpdates: event.target.checked })}
                        />
                      }
                      label={t('settings.about.autoCheck')}
                    />
                    <Typography variant="caption" color="text.secondary" sx={{ pl: 4 }}>
                      {t('settings.about.autoCheckDescription')}
                    </Typography>
                    {updateInfo?.checkedAt && (
                      <Typography variant="caption" color="text.secondary">
                        {t('settings.about.lastChecked', { date: formatDateTime(updateInfo.checkedAt, language) })}
                      </Typography>
                    )}
                    {updateInfo?.updateAvailable && (
                      <Alert
                        severity="info"
                        variant="outlined"
                        action={
                          <Button color="inherit" size="small" onClick={() => setIsUpdateDialogOpen(true)}>
                            {t('settings.about.view')}
                          </Button>
                        }
                      >
                        {t('settings.about.updateAvailable', { version: updateInfo.latestVersion })}
                      </Alert>
                    )}
                    <Stack direction={{ xs: 'column', sm: 'row' }} spacing={1} sx={{ alignItems: 'flex-start' }}>
                      <Button
                        disabled={isCheckingUpdate}
                        startIcon={
                          isCheckingUpdate ? (
                            <CircularProgress color="inherit" size={16} />
                          ) : (
                            <SystemUpdateAltRoundedIcon />
                          )
                        }
                        variant="contained"
                        onClick={() => void checkUpdate()}
                      >
                        {isCheckingUpdate ? t('settings.about.checking') : t('settings.about.check')}
                      </Button>
                      <Button
                        startIcon={<LaunchRoundedIcon />}
                        variant="outlined"
                        onClick={() =>
                          void openExternal(repositoryPageUrl, 'settings.feedback.repositoryOpenFailed')
                        }
                      >
                        {t('settings.about.repository')}
                      </Button>
                    </Stack>
                  </Stack>
                </Paper>

                <Paper variant="outlined" sx={{ p: 2 }}>
                  <Stack spacing={1.5}>
                    <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                      {t('settings.about.diagnosticsTitle')}
                    </Typography>
                    <Typography variant="body2" color="text.secondary">
                      {t('settings.about.diagnosticsDescription')}
                    </Typography>
                    <Stack direction={{ xs: 'column', sm: 'row' }} spacing={1}>
                      <Button
                        disabled={isCopyingDiagnostics}
                        startIcon={
                          isCopyingDiagnostics ? <CircularProgress color="inherit" size={16} /> : <ContentCopyRoundedIcon />
                        }
                        variant="contained"
                        onClick={() => void copyDiagnostics()}
                      >
                        {t('settings.about.copyDiagnostics')}
                      </Button>
                      <Button
                        startIcon={<BugReportOutlinedIcon />}
                        variant="outlined"
                        onClick={() => void openExternal(issuePageUrl, 'settings.feedback.issueOpenFailed')}
                      >
                        {t('settings.about.reportIssue')}
                      </Button>
                      <Button
                        startIcon={<LaunchRoundedIcon />}
                        variant="outlined"
                        onClick={() =>
                          void openExternal(
                            releaseNotesUrl(updateInfo?.currentVersion),
                            'settings.feedback.releaseNotesOpenFailed',
                          )
                        }
                      >
                        {t('settings.about.releaseNotes')}
                      </Button>
                    </Stack>
                    <Alert severity="warning" variant="outlined">
                      {t('settings.about.issueHint')}
                    </Alert>
                  </Stack>
                </Paper>
              </Stack>
            )}
          </Box>
        </Box>
      </Box>

      <Dialog
        fullWidth
        maxWidth="xs"
        open={isUpdateDialogOpen}
        onClose={() => !isInstallingUpdate && setIsUpdateDialogOpen(false)}
      >
        <DialogTitle>{t('settings.updateDialog.title')}</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            <Typography variant="h6" sx={{ fontWeight: 800 }}>
              {updateInfo?.updateAvailable
                ? t('settings.updateDialog.available', { version: updateInfo.latestVersion })
                : t('settings.updateDialog.current')}
            </Typography>
            <Typography variant="body2" color="text.secondary">
              {t('settings.updateDialog.versions', {
                current: updateInfo?.currentVersion ?? t('common.unavailable'),
                latest: updateInfo?.latestVersion ?? t('common.unavailable'),
              })}
            </Typography>
            {updateInfo?.checkedAt && (
              <Typography variant="body2" color="text.secondary">
                {t('settings.updateDialog.checkedAt', { date: formatDateTime(updateInfo.checkedAt, language) })}
              </Typography>
            )}
            {updateInfo?.updateAvailable && (
              <Alert severity="info" variant="outlined">
                {t('settings.updateDialog.installDescription')}
              </Alert>
            )}
            {isInstallingUpdate && (
              <Stack spacing={0.75}>
                <LinearProgress
                  aria-label={progressLabel}
                  variant={updateProgress?.totalBytes ? 'determinate' : 'indeterminate'}
                  value={
                    updateProgress?.totalBytes
                      ? Math.min(100, (updateProgress.downloadedBytes / updateProgress.totalBytes) * 100)
                      : undefined
                  }
                />
                <Typography variant="caption" color="text.secondary">
                  {progressLabel}
                </Typography>
              </Stack>
            )}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button disabled={isInstallingUpdate} onClick={() => setIsUpdateDialogOpen(false)}>
            {t('settings.updateDialog.close')}
          </Button>
          <Button
            disabled={isInstallingUpdate}
            endIcon={<LaunchRoundedIcon />}
            onClick={() => void openExternal(releasePageUrl, 'settings.feedback.releaseOpenFailed')}
          >
            {t('settings.updateDialog.releases')}
          </Button>
          {updateInfo?.updateAvailable && (
            <Button
              disabled={isInstallingUpdate}
              startIcon={
                isInstallingUpdate ? (
                  <CircularProgress color="inherit" size={16} />
                ) : (
                  <SystemUpdateAltRoundedIcon />
                )
              }
              variant="contained"
              onClick={() => void installUpdate()}
            >
              {isInstallingUpdate ? t('settings.updateDialog.installing') : t('settings.updateDialog.install')}
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
        <Alert
          closeText={t('common.close')}
          severity={feedback?.severity ?? 'info'}
          variant="filled"
          onClose={() => setFeedback(null)}
        >
          {feedback ? t(feedback.key) : null}
        </Alert>
      </Snackbar>
    </ThemeProvider>
  );
}
