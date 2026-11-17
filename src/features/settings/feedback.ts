import type { AlertColor } from '@mui/material';
export type FeedbackKey =
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
  | 'settings.quotaAutoContinue.feedback.enabled'
  | 'settings.quotaAutoContinue.feedback.disabled'
  | 'settings.quotaAutoContinue.feedback.operationFailed'
  | 'settings.quotaAutoContinue.feedback.testSent'
  | 'settings.quotaAutoContinue.feedback.testFailed'
  | 'settings.feedback.diagnosticsCopied'
  | 'settings.feedback.diagnosticsCopyFailed'
  | 'settings.feedback.issueOpenFailed'
  | 'settings.feedback.releaseNotesOpenFailed'
  | 'settings.feedback.repositoryOpenFailed';

export interface Feedback {
  key: FeedbackKey;
  severity: AlertColor;
}
