import { invoke } from '@/platform/ipc';
import type { NotificationEnableResult } from '@/contracts/settings';
export const notificationApi = {
  setNotificationsEnabled: (enabled: boolean) =>
    invoke<NotificationEnableResult>('set_notifications_enabled', { enabled }),
  sendTestNotification: () => invoke<void>('send_test_notification'),
};
