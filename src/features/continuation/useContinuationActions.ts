import { continuationApi } from './continuationApi';
import { currentAccountId } from '@/features/accounts/accountStore';
import { useState } from 'react';

import { type QuotaAutoContinueStatus } from '@/contracts/continuation';

import type { Feedback } from '@/features/settings/feedback';
import { usePreferences } from '@/features/settings/usePreferences';
export function useContinuationActions(
  selectedId: string | null,
  enqueueSettingsMutation: ReturnType<typeof usePreferences>['enqueueSettingsMutation'],
  setQuotaAutoContinueStatus: (s: QuotaAutoContinueStatus) => void,
  setFeedback: (value: Feedback) => void,
) {
  const [isChangingQuotaAutoContinue, setIsChangingQuotaAutoContinue] = useState(false);
  const [isTestingQuotaAutoContinue, setIsTestingQuotaAutoContinue] = useState(false);
  const updateQuotaAutoContinue = async (enabled: boolean) => {
    if (isChangingQuotaAutoContinue) return;
    setIsChangingQuotaAutoContinue(true);
    const expectedAccount = selectedId;
    try {
      const status = await enqueueSettingsMutation(async (current) => {
        const nextStatus = await continuationApi.setQuotaAutoContinueEnabled(enabled, expectedAccount);
        if (currentAccountId() !== expectedAccount) throw new Error('accountChanged');
        return {
          settings: { ...current, quotaAutoContinueEnabled: nextStatus.enabled },
          value: nextStatus,
        };
      });
      if (currentAccountId() !== expectedAccount) return;
      setQuotaAutoContinueStatus(status);
      setFeedback({
        key: enabled
          ? 'settings.quotaAutoContinue.feedback.enabled'
          : 'settings.quotaAutoContinue.feedback.disabled',
        severity: 'success',
      });
    } catch {
      if (currentAccountId() !== expectedAccount) return;
      setFeedback({ key: 'settings.quotaAutoContinue.feedback.operationFailed', severity: 'error' });
    } finally {
      setIsChangingQuotaAutoContinue(false);
    }
  };

  const testQuotaAutoContinue = async () => {
    if (isTestingQuotaAutoContinue) return;
    setIsTestingQuotaAutoContinue(true);
    const expectedAccount = selectedId;
    try {
      const result = await continuationApi.testQuotaAutoContinue(expectedAccount);
      if (currentAccountId() !== expectedAccount) return;
      setQuotaAutoContinueStatus(result);
      setFeedback({ key: 'settings.quotaAutoContinue.feedback.testSent', severity: 'success' });
    } catch {
      if (currentAccountId() !== expectedAccount) return;
      setFeedback({ key: 'settings.quotaAutoContinue.feedback.testFailed', severity: 'error' });
      try {
        const status = await continuationApi.getQuotaAutoContinueStatus(expectedAccount);
        if (currentAccountId() === expectedAccount) setQuotaAutoContinueStatus(status);
      } catch {
        // 错误反馈已覆盖主操作失败；状态重读失败时保留上一次脱敏状态。
      }
    } finally {
      setIsTestingQuotaAutoContinue(false);
    }
  };

  return {
    isChangingQuotaAutoContinue,
    isTestingQuotaAutoContinue,
    updateQuotaAutoContinue,
    testQuotaAutoContinue,
  };
}
