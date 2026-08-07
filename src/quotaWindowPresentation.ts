import type { QuotaWindow } from './types';

/** 仅有效的长周期时间窗才能展示节奏黑线或对应的底部预测建议。 */
export function hasValidPaceWindow(
  quotaWindow: QuotaWindow,
): quotaWindow is QuotaWindow & { startAt: string; resetAt: string } {
  if (!quotaWindow.showPaceMarker || !quotaWindow.startAt || !quotaWindow.resetAt) return false;
  const start = new Date(quotaWindow.startAt).getTime();
  const reset = new Date(quotaWindow.resetAt).getTime();
  return Number.isFinite(start) && Number.isFinite(reset) && reset > start;
}
