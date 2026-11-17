import { useSettingsWindowData } from './useSettingsWindowData';
import { useContinuationActions } from '@/features/continuation/useContinuationActions';
import { notificationApi } from '@/features/notifications/notificationApi';
import { historyApi } from '@/features/history/historyApi';
import { settingsApi } from '@/features/settings/settingsApi';
import type { UsageHistoryRequest } from '@/contracts/history';
import AccountsPage from '@/features/accounts/AccountsPage';
import { AccountSelector } from '@/features/accounts/AccountSelector';
import { useAccounts } from '@/features/accounts/accountStore';
import InfoOutlinedIcon from '@mui/icons-material/InfoOutlined';
import LoopRoundedIcon from '@mui/icons-material/LoopRounded';
import NotificationsOutlinedIcon from '@mui/icons-material/NotificationsOutlined';
import PaletteOutlinedIcon from '@mui/icons-material/PaletteOutlined';
import QueryStatsRoundedIcon from '@mui/icons-material/QueryStatsRounded';
import RefreshRoundedIcon from '@mui/icons-material/RefreshRounded';
import RocketLaunchOutlinedIcon from '@mui/icons-material/RocketLaunchOutlined';
import SettingsSuggestOutlinedIcon from '@mui/icons-material/SettingsSuggestOutlined';
import {
  Alert,
  Box,
  CssBaseline,
  Divider,
  Drawer,
  LinearProgress,
  List,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  Snackbar,
  Stack,
  ThemeProvider,
  Toolbar,
  Typography,
  useMediaQuery,
} from '@mui/material';
import { getCurrentWindow } from '@/platform/desktop';
import { writeText } from '@/platform/desktop';
import { openUrl } from '@/platform/desktop';
import { lazy, Suspense, useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { resolveSupportedLanguage } from '@/shared/i18n/i18n';
import NotificationsPage from '@/features/notifications/NotificationsPage';
import QuotaAutoContinuePage from '@/features/continuation/QuotaAutoContinuePage';
import { createUsageTheme } from '@/shared/theme';
import TrendsErrorBoundary from '@/features/history/TrendsErrorBoundary';

import { DisplaySettings } from '@/features/settings/DisplaySettings';
import { RefreshSettings } from '@/features/settings/RefreshSettings';
import { StartupSettings } from '@/features/settings/StartupSettings';
import { AboutPage } from '@/features/updates/AboutPage';
import type { Feedback, FeedbackKey } from '@/features/settings/feedback';
import { UpdateDialog } from '@/features/updates/UpdateDialog';
import { resolveTheme } from '@/shared/theme';
const TrendsPage = lazy(() => import('@/features/history/TrendsPage'));

type SettingsSection =
  | 'accounts'
  | 'display'
  | 'data'
  | 'notifications'
  | 'trends'
  | 'quota-auto-continue'
  | 'startup'
  | 'about';
const drawerWidth = 208;
const navigation: Array<{
  id: SettingsSection;
  labelKey:
    | 'feature.accounts'
    | 'settings.nav.display'
    | 'settings.nav.data'
    | 'settings.nav.notifications'
    | 'settings.nav.trends'
    | 'settings.nav.quotaAutoContinue'
    | 'settings.nav.startup'
    | 'settings.nav.about';
  icon: React.ReactElement;
}> = [
  { id: 'accounts', labelKey: 'feature.accounts', icon: <InfoOutlinedIcon fontSize="small" /> },
  { id: 'display', labelKey: 'settings.nav.display', icon: <PaletteOutlinedIcon fontSize="small" /> },
  { id: 'data', labelKey: 'settings.nav.data', icon: <RefreshRoundedIcon fontSize="small" /> },
  {
    id: 'notifications',
    labelKey: 'settings.nav.notifications',
    icon: <NotificationsOutlinedIcon fontSize="small" />,
  },
  { id: 'trends', labelKey: 'settings.nav.trends', icon: <QueryStatsRoundedIcon fontSize="small" /> },
  {
    id: 'quota-auto-continue',
    labelKey: 'settings.nav.quotaAutoContinue',
    icon: <LoopRoundedIcon fontSize="small" />,
  },
  { id: 'startup', labelKey: 'settings.nav.startup', icon: <RocketLaunchOutlinedIcon fontSize="small" /> },
  { id: 'about', labelKey: 'settings.nav.about', icon: <InfoOutlinedIcon fontSize="small" /> },
];

/** Independent settings window; all sensitive operations remain in label-gated Rust commands. */
export default function SettingsWindow() {
  const { selectedId } = useAccounts();
  const getHistory = useCallback(
    (request: UsageHistoryRequest) => historyApi.getUsageHistory(request, selectedId),
    [selectedId],
  );
  const subscribeHistory = useCallback(
    (handler: () => void) => historyApi.listenForUsageHistory(handler, selectedId),
    [selectedId],
  );
  const { t, i18n } = useTranslation();
  const [activeSection, setActiveSection] = useState<SettingsSection>(() =>
    window.location.hash === '#about' ? 'about' : 'display',
  );
  const [isChangingAutostart, setIsChangingAutostart] = useState(false);
  const [isSendingTestNotification, setIsSendingTestNotification] = useState(false);
  const [isCopyingDiagnostics, setIsCopyingDiagnostics] = useState(false);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const prefersDark = useMediaQuery('(prefers-color-scheme: dark)', { noSsr: true });
  const language = resolveSupportedLanguage(i18n.resolvedLanguage ?? i18n.language);

  const {
    settings,
    trendsOperationRevision,
    isSaving,
    enqueueSettingsMutation,
    updateSettings,
    updateLanguage,
    updateNotificationSettings,
    updateNotificationsEnabled,
    updateHistoryEnabled,
    updates,
    settingsLoadState,
    autostart,
    autostartLoaded,
    setAutostart,
    quotaAutoContinueStatus,
    setQuotaAutoContinueStatus,
  } = useSettingsWindowData(selectedId, setActiveSection, setFeedback);
  const {
    isChangingQuotaAutoContinue,
    isTestingQuotaAutoContinue,
    updateQuotaAutoContinue,
    testQuotaAutoContinue,
  } = useContinuationActions(selectedId, enqueueSettingsMutation, setQuotaAutoContinueStatus, setFeedback);
  const { updateInfo, isCheckingUpdate, setIsUpdateDialogOpen, checkUpdate } = updates;
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

  const sendTestNotification = async () => {
    if (isSendingTestNotification) return;
    setIsSendingTestNotification(true);
    try {
      await notificationApi.sendTestNotification();
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
      setAutostart(await settingsApi.setAutostart(enabled));
      setFeedback({ key: 'settings.feedback.autostartSaved', severity: 'success' });
    } catch {
      setFeedback({ key: 'settings.feedback.autostartFailed', severity: 'error' });
    } finally {
      setIsChangingAutostart(false);
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
      const diagnostics = await settingsApi.getDiagnostics();
      await writeText(diagnostics);
      setFeedback({ key: 'settings.feedback.diagnosticsCopied', severity: 'success' });
    } catch {
      setFeedback({ key: 'settings.feedback.diagnosticsCopyFailed', severity: 'error' });
    } finally {
      setIsCopyingDiagnostics(false);
    }
  };

  return (
    <ThemeProvider theme={muiTheme}>
      <CssBaseline />
      <Box
        component="main"
        data-theme-mode={activeTheme}
        sx={{
          display: 'flex',
          width: '100%',
          height: '100dvh',
          overflow: 'hidden',
          bgcolor: 'background.paper',
          color: 'text.primary',
        }}
      >
        <Drawer
          variant="permanent"
          sx={{
            width: drawerWidth,
            flexShrink: 0,
            '& .MuiDrawer-paper': {
              width: drawerWidth,
              display: 'flex',
              flexDirection: 'column',
              boxSizing: 'border-box',
              borderRight: 1,
              borderColor: 'divider',
              bgcolor: 'background.paper',
            },
          }}
        >
          <Toolbar sx={{ minHeight: 72, px: 2, flexShrink: 0 }}>
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
          <List
            aria-label={t('settings.navigationAria')}
            disablePadding
            sx={{ px: 1, py: 1, flex: 1, minHeight: 0, overflowY: 'auto' }}
          >
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
                    slotProps={{
                      primary: { sx: { fontSize: 14, fontWeight: activeSection === item.id ? 700 : 500 } },
                    }}
                  />
                </ListItemButton>
              );
            })}
          </List>
          <Divider />
          <Box component="section" aria-label={t('feature.selected')} sx={{ p: 1.5, flexShrink: 0 }}>
            <AccountSelector sidebar />
          </Box>
        </Drawer>

        <Box
          component="section"
          aria-label={t('settings.contentAria')}
          sx={{ flex: 1, minWidth: 0, overflow: 'auto', bgcolor: 'background.default' }}
        >
          <Box
            sx={{
              width: '100%',
              maxWidth: activeSection === 'trends' ? 980 : activeSection === 'accounts' ? 1000 : 680,
              mx: 'auto',
              px: { xs: 2, sm: activeSection === 'accounts' ? 3 : 4 },
              py: 4,
            }}
          >
            {settingsLoadState === 'loading' && <LinearProgress sx={{ mb: 2 }} />}
            {settingsLoadState === 'error' && (
              <Alert severity="error" sx={{ mb: 2 }}>
                {t('settings.feedback.loadFailed')}
              </Alert>
            )}

            {activeSection === 'accounts' && <AccountsPage />}
            {activeSection === 'display' && (
              <DisplaySettings
                settings={settings}
                settingsControlsDisabled={settingsControlsDisabled}
                updateSettings={updateSettings}
                updateLanguage={updateLanguage}
              />
            )}

            {activeSection === 'data' && (
              <RefreshSettings
                settings={settings}
                settingsControlsDisabled={settingsControlsDisabled}
                updateSettings={updateSettings}
              />
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

            {settingsLoadState === 'ready' && (
              <TrendsErrorBoundary
                active={activeSection === 'trends'}
                recoveryRevision={trendsOperationRevision}
              >
                <Suspense fallback={<LinearProgress aria-label={t('trends.loading')} />}>
                  <TrendsPage
                    accountId={selectedId}
                    key={selectedId ?? 'legacy'}
                    historyEnabled={settings.historyEnabled}
                    getUsageHistory={getHistory}
                    onSetHistoryEnabled={updateHistoryEnabled}
                    onClearUsageHistory={() => historyApi.clearUsageHistory(selectedId)}
                    listenForUsageHistory={subscribeHistory}
                  />
                </Suspense>
              </TrendsErrorBoundary>
            )}

            {activeSection === 'quota-auto-continue' && (
              <QuotaAutoContinuePage
                key={selectedId ?? 'legacy'}
                enabled={settings.quotaAutoContinueEnabled}
                status={
                  quotaAutoContinueStatus?.accountId && quotaAutoContinueStatus.accountId !== selectedId
                    ? null
                    : quotaAutoContinueStatus
                }
                language={settings.language}
                disabled={settingsControlsDisabled}
                isChanging={isChangingQuotaAutoContinue}
                isTesting={isTestingQuotaAutoContinue}
                onSetEnabled={updateQuotaAutoContinue}
                onTest={testQuotaAutoContinue}
              />
            )}

            {activeSection === 'startup' && (
              <StartupSettings
                settings={settings}
                settingsControlsDisabled={settingsControlsDisabled}
                updateSettings={updateSettings}
                autostart={autostart}
                autostartLoaded={autostartLoaded}
                isChangingAutostart={isChangingAutostart}
                updateAutostart={updateAutostart}
              />
            )}

            {activeSection === 'about' && (
              <AboutPage
                settings={settings}
                settingsControlsDisabled={settingsControlsDisabled}
                updateSettings={updateSettings}
                setIsUpdateDialogOpen={setIsUpdateDialogOpen}
                updateInfo={updateInfo}
                isCheckingUpdate={isCheckingUpdate}
                checkUpdate={checkUpdate}
                isCopyingDiagnostics={isCopyingDiagnostics}
                copyDiagnostics={copyDiagnostics}
                openExternal={openExternal}
                language={language}
              />
            )}
          </Box>
        </Box>
      </Box>

      <UpdateDialog language={language} controls={updates} openExternal={openExternal} />

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
