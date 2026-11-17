import { invoke, listen } from '@/platform/ipc';
import type { AppUpdateInfo, AppUpdateProgress } from '@/contracts/updates';
export const updateApi = {
  getAppUpdateInfo: () => invoke<AppUpdateInfo>('get_app_update_info'),
  checkAppUpdate: () => invoke<AppUpdateInfo>('check_app_update'),
  installAppUpdate: () => invoke<void>('install_app_update'),
  listenForAppUpdate: (handler: (info: AppUpdateInfo) => void) =>
    listen<AppUpdateInfo>('app-update-updated', (event) => handler(event.payload)),
  listenForAppUpdateProgress: (handler: (progress: AppUpdateProgress) => void) =>
    listen<AppUpdateProgress>('app-update-progress', (event) => handler(event.payload)),
};
