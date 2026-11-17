import { updateApi } from './updateApi';
import { useState } from 'react';

import { type AppUpdateInfo, type AppUpdateProgress } from '@/contracts/updates';

import type { Feedback, FeedbackKey } from '@/features/settings/feedback';
const knownUpdateErrors = [
  ['更新包的签名验证失败，已取消安装。', 'settings.feedback.signatureFailed'],
  ['暂时没有适用于当前设备的有效更新。', 'settings.feedback.noCompatibleUpdate'],
  ['无法连接更新服务，请检查网络或代理后重试。', 'settings.feedback.updateNetworkFailed'],
  ['更新服务配置不可用，请稍后重试。', 'settings.feedback.updateConfigFailed'],
  ['更新安装未完成，请稍后重试。', 'settings.feedback.updateIncomplete'],
  ['更新操作正在进行，请稍后再试。', 'settings.feedback.updateBusy'],
  ['没有待安装的更新，请先重新检查更新。', 'settings.feedback.noPendingUpdate'],
] as const satisfies ReadonlyArray<readonly [string, FeedbackKey]>;

function safeUpdateErrorKey(error: unknown, fallback: FeedbackKey): FeedbackKey {
  const message = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
  return knownUpdateErrors.find(([safeMessage]) => message.includes(safeMessage))?.[1] ?? fallback;
}

export function useUpdates(setFeedback: (message: Feedback) => void) {
  const [isCheckingUpdate, setIsCheckingUpdate] = useState(false);
  const [isInstallingUpdate, setIsInstallingUpdate] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<AppUpdateInfo | null>(null);
  const [updateProgress, setUpdateProgress] = useState<AppUpdateProgress | null>(null);
  const [isUpdateDialogOpen, setIsUpdateDialogOpen] = useState(false);
  const checkUpdate = async () => {
    if (isCheckingUpdate) return;
    setIsCheckingUpdate(true);
    setUpdateProgress(null);
    try {
      setUpdateInfo(await updateApi.checkAppUpdate());
      setIsUpdateDialogOpen(true);
    } catch (error) {
      setFeedback({
        key: safeUpdateErrorKey(error, 'settings.feedback.updateCheckFailed'),
        severity: 'error',
      });
    } finally {
      setIsCheckingUpdate(false);
    }
  };

  const installUpdate = async () => {
    if (!updateInfo?.updateAvailable || isInstallingUpdate) return;
    setIsInstallingUpdate(true);
    setUpdateProgress({ stage: 'downloading', downloadedBytes: 0, totalBytes: null });
    try {
      await updateApi.installAppUpdate();
      setFeedback({ key: 'settings.feedback.updateInstallStarted', severity: 'success' });
    } catch (error) {
      setFeedback({
        key: safeUpdateErrorKey(error, 'settings.feedback.updateInstallFailed'),
        severity: 'error',
      });
    } finally {
      setIsInstallingUpdate(false);
    }
  };

  return {
    isCheckingUpdate,
    isInstallingUpdate,
    updateInfo,
    setUpdateInfo,
    updateProgress,
    setUpdateProgress,
    isUpdateDialogOpen,
    setIsUpdateDialogOpen,
    checkUpdate,
    installUpdate,
  };
}
