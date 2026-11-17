import { updateApi } from '@/features/updates/updateApi';
import { continuationApi } from '@/features/continuation/continuationApi';
import { settingsApi } from '@/features/settings/settingsApi';
import { useEffect, useState } from 'react';

import { type QuotaAutoContinueStatus } from '@/contracts/continuation';

import { useUpdates } from '@/features/updates/useUpdates';
import type { Feedback } from '@/features/settings/feedback';
import { usePreferences } from '@/features/settings/usePreferences';
type SettingsLoadState = 'loading' | 'ready' | 'error';

export function useSettingsWindowData(
  selectedId: string | null,
  onNavigate: (section: 'about') => void,
  setFeedback: (value: Feedback) => void,
) {
  const [autostart, setAutostart] = useState(false);
  const [autostartLoaded, setAutostartLoaded] = useState(false);
  const [settingsLoadState, setSettingsLoadState] = useState<SettingsLoadState>('loading');
  const [quotaAutoContinueStatus, setQuotaAutoContinueStatus] = useState<QuotaAutoContinueStatus | null>(
    null,
  );
  const {
    settings,
    trendsOperationRevision,
    isSaving,
    enqueueSettingsMutation,
    applySettingsSnapshot,
    settingsLoadedRef,
    pendingHistoryOperationRef,
    updateSettings,
    updateLanguage,
    updateNotificationSettings,
    updateNotificationsEnabled,
    updateHistoryEnabled,
  } = usePreferences(setFeedback);
  const updates = useUpdates(setFeedback);
  const { setUpdateInfo, setUpdateProgress, setIsUpdateDialogOpen } = updates;
  useEffect(() => {
    let disposed = false;
    const removeListeners: Array<() => void> = [];
    let settingsEventReceived = false;
    let updateEventReceived = false;
    let quotaAutoContinueEventReceived = false;

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
          settingsApi.listenForSettings((next) => {
            settingsEventReceived = true;
            const pendingHistoryOperation = pendingHistoryOperationRef.current;
            if (disposed) return;
            if (pendingHistoryOperation && next.historyEnabled === pendingHistoryOperation.enabled) {
              // Rust emits the authoritative snapshot before the invoke response resolves.
              // Commit only the command response (or an explicit readback) so settings and
              // the recovery revision change atomically and unrelated same-value events
              // cannot confirm this operation.
              return;
            }
            applySettingsSnapshot(next);
          }),
        ),
        registerListener(
          updateApi.listenForAppUpdate((next) => {
            updateEventReceived = true;
            if (!disposed) setUpdateInfo(next);
          }),
        ),
        registerListener(
          updateApi.listenForAppUpdateProgress((next) => {
            if (!disposed) setUpdateProgress(next);
          }),
        ),
        registerListener(
          settingsApi.listenForSettingsNavigation((section) => {
            if (!disposed) {
              onNavigate(section);
              setIsUpdateDialogOpen(section === 'about');
            }
          }),
        ),
        registerListener(
          continuationApi.listenForQuotaAutoContinue((next) => {
            quotaAutoContinueEventReceived = true;
            if (!disposed) setQuotaAutoContinueStatus(next);
          }, selectedId),
        ),
      ]);

      if (disposed) return;

      const [settingsResult, autostartResult, updateResult, quotaAutoContinueResult] =
        await Promise.allSettled([
          settingsApi.getSettings(),
          settingsApi.getAutostart(),
          updateApi.getAppUpdateInfo(),
          continuationApi.getQuotaAutoContinueStatus(selectedId),
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
      if (quotaAutoContinueResult.status === 'fulfilled' && !quotaAutoContinueEventReceived) {
        setQuotaAutoContinueStatus(quotaAutoContinueResult.value);
      }
    })();

    return () => {
      disposed = true;
      removeListeners.splice(0).forEach((remove) => remove());
    };
  }, [
    applySettingsSnapshot,
    selectedId,
    pendingHistoryOperationRef,
    setIsUpdateDialogOpen,
    setUpdateInfo,
    setUpdateProgress,
    settingsLoadedRef,
    onNavigate,
    setFeedback,
  ]);
  return {
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
  };
}
