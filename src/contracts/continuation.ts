export interface ResponseDetails {
  startedAt: string | null;
  responseModel: string | null;
  httpStatus: number | null;
  durationMs: number;
  text: string;
  truncated: boolean;
  errorMessage: string | null;
  deliveryUncertain: boolean;
}
export type QuotaAutoContinuePhase =
  | 'disabled'
  | 'waitingForWeeklyWindow'
  | 'scheduled'
  | 'running'
  | 'waitingForRetry'
  | 'succeeded'
  | 'sentAwaitingConfirmation'
  | 'authenticationRequired'
  | 'missed'
  | 'failed';

export type QuotaAutoContinueErrorCode =
  | 'authMissing'
  | 'authInvalid'
  | 'network'
  | 'rateLimited'
  | 'serviceUnavailable'
  | 'invalidResponse'
  | 'noTextModel'
  | 'accountChanged'
  | 'persistence'
  | 'busy';

/** 自动排期的触发原因由 Rust 判定；前端只展示固定枚举，不根据百分比自行推断。 */
export type QuotaAutoContinueTriggerReason = 'quotaRecovered' | 'deadlineReached';

/**
 * 最近一次自动执行或手动测试的脱敏摘要。
 * slotIndex 使用后端的 0-based 尝试槽；手动测试没有槽位，因此为 null。
 */
export interface QuotaAutoContinueResult {
  details?: ResponseDetails | null;
  attemptedAt: string | null;
  successAt: string | null;
  errorCode: QuotaAutoContinueErrorCode | null;
  model: string | null;
  slotIndex: number | null;
}

/** 设置窗口可获取该账号的排期及最近回复；认证材料始终留在 Rust 边界内。 */
export interface QuotaAutoContinueStatus {
  accountId?: string | null;
  enabled: boolean;
  phase: QuotaAutoContinuePhase;
  targetResetAt: string | null;
  nextAttemptAt: string | null;
  attemptedCount: number;
  lastAttemptAt: string | null;
  lastSuccessAt: string | null;
  lastErrorCode: QuotaAutoContinueErrorCode | null;
  selectedModel: string | null;
  /** v0.4.1 起分开保存自动执行和手动测试，避免把“立即测试”误认成自动接续。 */
  lastAutomaticResult?: QuotaAutoContinueResult | null;
  lastManualResult?: QuotaAutoContinueResult | null;
  lastTriggerReason?: QuotaAutoContinueTriggerReason | null;
  lastResetDetectedAt?: string | null;
}
