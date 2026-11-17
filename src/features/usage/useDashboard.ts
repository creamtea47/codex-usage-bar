import { updateApi } from '@/features/updates/updateApi';
import { settingsApi } from '@/features/settings/settingsApi';
import { usageApi } from './usageApi';
import { type AlertColor } from '@mui/material';
import { useEffect, useState } from 'react';

import { defaultSettings } from '@/features/settings/defaultSettings';
import { loadingSnapshot } from './loadingSnapshot';
import { type AppUpdateInfo } from '@/contracts/updates';
import { type DashboardSnapshot } from '@/contracts/usage';
import { type Settings } from '@/contracts/settings';

interface Feedback {
  code: 'refreshFailed' | 'openSettingsFailed';
  severity: AlertColor;
}

export function useDashboard(accountId: string | null) {
  const [snapshot, setSnapshot] = useState<DashboardSnapshot>(loadingSnapshot);
  const [settings, setSettings] = useState<Settings>(defaultSettings);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const [updateNotice, setUpdateNotice] = useState<AppUpdateInfo | null>(null);
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
          usageApi.listenForDashboard((next) => {
            dashboardEventReceived = true;
            if (!disposed) setSnapshot(next);
          }),
          settingsApi.listenForSettings((next) => {
            settingsEventReceived = true;
            if (!disposed) setSettings(next);
          }),
          updateApi.listenForAppUpdate((next) => {
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
          usageApi.getDashboard(accountId),
          settingsApi.getSettings(),
          updateApi.getAppUpdateInfo(),
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
            status: current.quotaWindows.length || current.extraCredits ? 'stale' : 'error',
            message: 'localBridge',
          }));
        }
      }
    })();

    return () => {
      disposed = true;
      removeRegisteredListeners();
    };
  }, [accountId]);
  const refresh = async () => {
    if (isRefreshing) return;
    setIsRefreshing(true);
    try {
      setSnapshot(await usageApi.refreshDashboard(accountId));
    } catch {
      setFeedback({ code: 'refreshFailed', severity: 'error' });
    } finally {
      setIsRefreshing(false);
    }
  };
  return { snapshot, settings, feedback, setFeedback, updateNotice, setUpdateNotice, isRefreshing, refresh };
}
