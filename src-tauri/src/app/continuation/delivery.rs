use super::*;

impl QuotaAutoContinueRuntime {
    pub fn claim_due_attempt(
        &self,
        enabled: bool,
        now: DateTime<Utc>,
    ) -> Result<Option<ClaimedAttempt>, QuotaAutoContinueErrorCode> {
        if !enabled {
            return Ok(None);
        }
        let mut persisted = self.persisted();
        let Some(mut cycle) = persisted.active_cycle.take() else {
            return Ok(None);
        };
        if cycle.request_completed {
            persisted.active_cycle = Some(cycle);
            return Ok(None);
        }
        let Some(account) = persisted.account_fingerprint.clone() else {
            persisted.active_cycle = Some(cycle);
            return Ok(None);
        };
        let Some(window) = persisted.window_fingerprint.clone() else {
            persisted.active_cycle = Some(cycle);
            return Ok(None);
        };
        let mut reset_audit = None;
        let deadline_advanced = persisted
            .pending_reset_at
            .or_else(|| {
                persisted
                    .last_observation
                    .as_ref()
                    .and_then(|value| value.reset_at)
            })
            .filter(|value| *value > cycle.expected_reset_at);
        if cycle.confirmed_event.is_none()
            && now >= cycle.expected_reset_at
            && deadline_advanced.is_some()
        {
            persisted.active_cycle = Some(cycle);
            let remaining = persisted
                .last_observation
                .as_ref()
                .map(|value| value.remaining_percent)
                .unwrap_or(100);
            let pending_reset_at = persisted.pending_reset_at;
            if let Some(event) = confirm_event_in_state(
                &mut persisted,
                QuotaResetReason::DeadlineReached,
                now,
                remaining,
                remaining,
                pending_reset_at,
            ) {
                reset_audit = Some(reset_confirmed_audit(&event, now));
            }
            cycle = persisted
                .active_cycle
                .take()
                .expect("active cycle retained");
        }
        if cycle.confirmed_event.is_none() {
            persisted.active_cycle = Some(cycle);
            return Ok(None);
        }
        let target = attempt_anchor(&cycle);
        let elapsed = (now - target).num_seconds();
        if elapsed > ATTEMPT_OFFSETS_SECONDS[3] {
            cycle.phase = QuotaAutoContinuePhase::Missed;
            cycle.request_completed = true;
            let missed_audit = cycle.confirmed_event.as_ref().map(|event| {
                QuotaAuditRecord::warning(now, QuotaAuditAction::CycleMissed).with_event(
                    event.event_id.clone(),
                    event.generation_id.clone(),
                    event.reason,
                )
            });
            persisted.active_cycle = Some(cycle);
            save_runtime_state(&self.path, &persisted)
                .map_err(|_| QuotaAutoContinueErrorCode::Persistence)?;
            drop(persisted);
            if let Some(record) = reset_audit {
                self.audit(record);
            }
            if let Some(record) = missed_audit {
                self.audit(record);
            }
            self.notify_schedule_changed();
            return Ok(None);
        }
        let selected = cycle
            .consumed_slots
            .iter()
            .enumerate()
            .filter(|(index, consumed)| !**consumed && ATTEMPT_OFFSETS_SECONDS[*index] <= elapsed)
            .map(|(index, _)| index)
            .max();
        let Some(slot_index) = selected else {
            return Ok(None);
        };
        // 睡眠恢复只执行最近一个槽；更早且未执行的槽同时标记为已跳过，避免连续补跑。
        for consumed in cycle.consumed_slots.iter_mut().take(slot_index + 1) {
            *consumed = true;
        }
        cycle.phase = QuotaAutoContinuePhase::Running;
        let event = cycle
            .confirmed_event
            .as_ref()
            .expect("due attempt must have a confirmed reset event")
            .clone();
        *self
            .client
            .details
            .lock()
            .unwrap_or_else(|v| v.into_inner()) = None;
        persisted.last_automatic_result = Some(QuotaAutoContinueResult {
            details: None,
            attempted_at: Some(now),
            success_at: None,
            error_code: None,
            model: None,
            slot_index: Some(slot_index as u8),
        });
        let generation_id = cycle.generation_id.clone();
        let expected_reset_at = cycle.expected_reset_at;
        persisted.active_cycle = Some(cycle);
        save_runtime_state(&self.path, &persisted)
            .map_err(|_| QuotaAutoContinueErrorCode::Persistence)?;
        drop(persisted);
        if let Some(record) = reset_audit {
            self.audit(record);
        }
        let mut claimed = QuotaAuditRecord::info(
            now,
            if slot_index == 0 {
                QuotaAuditAction::Slot0Claimed
            } else {
                QuotaAuditAction::RetrySlotClaimed
            },
        )
        .with_event(
            event.event_id.clone(),
            event.generation_id.clone(),
            event.reason,
        );
        claimed.slot_index = Some(slot_index as u8);
        self.audit(claimed);
        self.notify_schedule_changed();
        Ok(Some(ClaimedAttempt {
            target_reset_at: expected_reset_at,
            account_fingerprint: account,
            window_fingerprint: window,
            slot_index,
            generation_id,
        }))
    }

    pub fn preflight_decision(&self, attempt: &ClaimedAttempt) -> PreflightDecision {
        let observation = self.observation().clone();
        let Some(observation) = observation else {
            return PreflightDecision::AccountChanged;
        };
        if observation.account_fingerprint != attempt.account_fingerprint
            || observation.window_fingerprint.as_deref() != Some(&attempt.window_fingerprint)
        {
            return PreflightDecision::AccountChanged;
        }
        let same = {
            let persisted = self.persisted();
            same_cycle(&persisted, attempt)
        };
        if !same {
            return PreflightDecision::AlreadyAdvanced;
        }
        // reset_at 提前推进只进入 pending；不能再据此抑制已确认事件的发送。
        PreflightDecision::Proceed
    }

    pub fn finish_failure(
        &self,
        attempt: &ClaimedAttempt,
        error: QuotaAutoContinueErrorCode,
        model: Option<String>,
    ) {
        let mut persisted = self.persisted();
        if !same_cycle(&persisted, attempt) {
            return;
        }
        if let Some(result) = persisted.last_automatic_result.as_mut() {
            result.error_code = Some(error);
            result.details = self.client.last_details();
            result.model = model.clone();
        }
        let cycle = persisted.active_cycle.as_mut().expect("same cycle exists");
        let exhausted = cycle.consumed_slots[3]
            || self
                .client
                .last_details()
                .is_some_and(|d| d.delivery_uncertain);
        if exhausted {
            // +30 分钟末槽失败即为最终结案；否则 scheduler 会再次唤醒并误改成 Missed。
            cycle.request_completed = true;
        }
        cycle.phase = if matches!(
            error,
            QuotaAutoContinueErrorCode::AuthMissing | QuotaAutoContinueErrorCode::AuthInvalid
        ) {
            QuotaAutoContinuePhase::AuthenticationRequired
        } else if exhausted {
            QuotaAutoContinuePhase::Failed
        } else {
            QuotaAutoContinuePhase::WaitingForRetry
        };
        let audit = cycle.confirmed_event.as_ref().map(|event| {
            let mut record =
                QuotaAuditRecord::warning(Utc::now(), QuotaAuditAction::AutomaticFailed)
                    .with_event(
                        event.event_id.clone(),
                        event.generation_id.clone(),
                        event.reason,
                    );
            record.slot_index = Some(attempt.slot_index as u8);
            record.error_code = Some(error_code_name(error).to_owned());
            record
        });
        if save_runtime_state(&self.path, &persisted).is_err() {
            log::warn!("额度自动接续失败状态未能持久化：类别=storage。");
        }
        drop(persisted);
        *self.selected_model() = model;
        if let Some(record) = audit {
            self.audit(record);
        }
        self.notify_schedule_changed();
    }

    pub fn finish_sent(&self, attempt: &ClaimedAttempt, model: String, now: DateTime<Utc>) {
        let mut persisted = self.persisted();
        if !same_cycle(&persisted, attempt) {
            return;
        }
        if let Some(result) = persisted.last_automatic_result.as_mut() {
            result.details = self.client.last_details();
            result.success_at = Some(now);
            result.error_code = None;
            result.model = Some(model.clone());
        }
        let event = {
            let cycle = persisted.active_cycle.as_mut().expect("same cycle exists");
            cycle.request_completed = true;
            cycle.phase = QuotaAutoContinuePhase::SentAwaitingConfirmation;
            cycle.confirmed_event.clone()
        };
        let audit = event.as_ref().map(|event| {
            let mut record = QuotaAuditRecord::info(now, QuotaAuditAction::ResponseCompleted)
                .with_event(
                    event.event_id.clone(),
                    event.generation_id.clone(),
                    event.reason,
                );
            record.slot_index = Some(attempt.slot_index as u8);
            record
        });
        if save_runtime_state(&self.path, &persisted).is_err() {
            log::warn!("额度自动接续成功状态未能持久化：类别=storage。");
        }
        drop(persisted);
        *self.selected_model() = Some(model);
        if let Some(record) = audit {
            self.audit(record);
        }
        self.notify_schedule_changed();
    }

    pub fn mark_already_advanced(&self, attempt: &ClaimedAttempt, _now: DateTime<Utc>) {
        let mut persisted = self.persisted();
        if same_cycle(&persisted, attempt) {
            let cycle = persisted.active_cycle.as_mut().expect("same cycle exists");
            cycle.request_completed = true;
            cycle.phase = QuotaAutoContinuePhase::Succeeded;
            if save_runtime_state(&self.path, &persisted).is_err() {
                log::warn!("额度自动接续确认状态未能持久化：类别=storage。");
            }
        }
        drop(persisted);
        self.notify_schedule_changed();
    }

    pub async fn send_for_attempt(&self, attempt: &ClaimedAttempt) -> Result<String, SendFailure> {
        let salt = self.persisted().salt.clone();
        self.client
            .send_greeting(Some((&attempt.account_fingerprint, &salt)))
            .await
    }

    pub async fn send_manual_test(&self) -> Result<String, SendFailure> {
        let guard = self.manual_account_guard();
        self.client
            .send_greeting(
                guard
                    .as_ref()
                    .map(|(fingerprint, salt)| (fingerprint.as_str(), salt.as_str())),
            )
            .await
    }

    pub(super) fn manual_account_guard(&self) -> Option<(String, String)> {
        // 两次复制之间不持有任何 guard，避免与 observe_dashboard 形成 observation/state ABBA。
        let observation = self.observation().clone();
        let salt = self.persisted().salt.clone();
        observation.map(|observation| (observation.account_fingerprint, salt))
    }

    pub fn record_manual_success(&self, model: String, now: DateTime<Utc>) {
        let mut persisted = self.persisted();
        persisted.last_manual_result = Some(QuotaAutoContinueResult {
            details: self.client.last_details(),
            attempted_at: Some(now),
            success_at: Some(now),
            error_code: None,
            model: Some(model.clone()),
            slot_index: None,
        });
        let audit = if let Some(cycle) = persisted
            .active_cycle
            .as_mut()
            .filter(|cycle| cycle.confirmed_event.is_some())
        {
            cycle.request_completed = true;
            cycle.phase = QuotaAutoContinuePhase::Succeeded;
            cycle.confirmed_event.as_ref().map(|event| {
                QuotaAuditRecord::info(now, QuotaAuditAction::ManualSatisfied).with_event(
                    event.event_id.clone(),
                    event.generation_id.clone(),
                    event.reason,
                )
            })
        } else {
            None
        };
        if let Some(record) = audit.as_ref() {
            let _ = record;
        }
        if save_runtime_state(&self.path, &persisted).is_err() {
            log::warn!("额度自动接续手动测试结果未能持久化：类别=storage。");
        }
        drop(persisted);
        *self.selected_model() = Some(model);
        if let Some(record) = audit {
            self.audit(record);
        }
        self.notify_schedule_changed();
    }

    pub fn record_manual_failure(&self, failure: &SendFailure) {
        let mut persisted = self.persisted();
        if self
            .client
            .last_details()
            .is_some_and(|d| d.delivery_uncertain)
        {
            if let Some(cycle) = persisted
                .active_cycle
                .as_mut()
                .filter(|c| c.confirmed_event.is_some())
            {
                cycle.request_completed = true;
                cycle.phase = QuotaAutoContinuePhase::Failed;
            }
        }
        persisted.last_manual_result = Some(QuotaAutoContinueResult {
            details: self.client.last_details(),
            attempted_at: Some(Utc::now()),
            success_at: None,
            error_code: Some(failure.code),
            model: failure.model.clone(),
            slot_index: None,
        });
        if save_runtime_state(&self.path, &persisted).is_err() {
            log::warn!("额度自动接续手动测试失败状态未能持久化：类别=storage。");
        }
        drop(persisted);
        *self.selected_model() = failure.model.clone();
        self.notify_schedule_changed();
    }
}
