import { notificationApi } from '@/features/notifications/notificationApi';
import { historyApi } from '@/features/history/historyApi';
import { settingsApi } from './settingsApi';
import { useCallback, useEffect, useRef, useState } from 'react';

import { applyLanguagePreference } from '@/shared/i18n/i18n';
import { applySettingsViewSnapshot, type SettingsViewState } from '@/features/history/trendsOperationState';
import { defaultSettings } from './defaultSettings';
import { type Language, type NotificationSettings, type Settings } from '@/contracts/settings';

import type { Feedback } from './feedback';
type SettingsUpdater = Partial<Settings> | ((current: Settings) => Settings);
interface SettingsMutationResult<T> {
  confirmedHistoryEnabled?: boolean;
  settings: Settings;
  value: T;
}

interface PendingHistoryOperation {
  enabled: boolean;
}

export function usePreferences(setFeedback: (message: Feedback) => void) {
  const [settingsView, setSettingsView] = useState<SettingsViewState>({
    settings: defaultSettings,
    trendsOperationRevision: 0,
  });
  const { settings, trendsOperationRevision } = settingsView;
  const settingsRef = useRef<Settings>(defaultSettings);
  const settingsLoadedRef = useRef(false);
  const settingsMutationTail = useRef<Promise<void>>(Promise.resolve());
  const pendingSettingsMutations = useRef(0);
  const pendingHistoryOperationRef = useRef<PendingHistoryOperation | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const applySettingsSnapshot = useCallback((next: Settings, confirmedHistoryEnabled?: boolean) => {
    settingsRef.current = next;
    setSettingsView((current) => applySettingsViewSnapshot(current, next, confirmedHistoryEnabled));
  }, []);

  const enqueueSettingsMutation = useCallback(
    <T>(mutation: (current: Settings) => Promise<SettingsMutationResult<T>>): Promise<T> => {
      pendingSettingsMutations.current += 1;
      setIsSaving(true);

      const result = settingsMutationTail.current.then(async () => {
        if (!settingsLoadedRef.current) throw new Error('settings-not-loaded');
        const completed = await mutation(settingsRef.current);
        applySettingsSnapshot(completed.settings, completed.confirmedHistoryEnabled);
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

  const updateSettings = async (updater: SettingsUpdater): Promise<boolean> => {
    try {
      await enqueueSettingsMutation(async (current) => {
        const next = typeof updater === 'function' ? updater(current) : { ...current, ...updater };
        return { settings: await settingsApi.saveSettings(next), value: undefined };
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
        const next = await notificationApi.setNotificationsEnabled(enabled);
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
          key: enabled
            ? 'settings.notifications.feedback.enabled'
            : 'settings.notifications.feedback.disabled',
          severity: 'success',
        });
      }
    } catch {
      setFeedback({ key: 'settings.notifications.permissionUnavailable', severity: 'error' });
    }
  };

  const updateHistoryEnabled = useCallback(
    async (enabled: boolean) => {
      const operation: PendingHistoryOperation = { enabled };
      pendingHistoryOperationRef.current = operation;
      try {
        const matched = await enqueueSettingsMutation(async () => {
          let next: Settings;
          try {
            next = await historyApi.setHistoryEnabled(enabled);
          } catch (operationError) {
            try {
              next = await settingsApi.getSettings();
            } catch {
              // Neither the command nor an explicit readback produced an authoritative
              // snapshot, so retain the last reliable UI state.
              throw operationError;
            }
          }
          const matchesRequest = next.historyEnabled === enabled;
          return {
            confirmedHistoryEnabled: matchesRequest ? enabled : undefined,
            settings: next,
            value: matchesRequest,
          };
        });
        if (!matched) throw new Error('history-setting-mismatch');
      } finally {
        if (pendingHistoryOperationRef.current === operation) {
          pendingHistoryOperationRef.current = null;
        }
      }
    },
    [enqueueSettingsMutation],
  );

  return {
    enqueueSettingsMutation,
    settings,
    trendsOperationRevision,
    isSaving,
    applySettingsSnapshot,
    settingsLoadedRef,
    pendingHistoryOperationRef,
    updateSettings,
    updateLanguage,
    updateNotificationSettings,
    updateNotificationsEnabled,
    updateHistoryEnabled,
  };
}
