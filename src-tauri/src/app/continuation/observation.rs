use super::*;

impl QuotaAutoContinueRuntime {
    pub fn observe_dashboard(
        &self,
        enabled: bool,
        identity: &UsageAccountIdentity,
        snapshot: &DashboardSnapshot,
        now: DateTime<Utc>,
    ) -> bool {
        // 指纹计算只需 salt；先复制并释放 state 锁，再更新 observation，
        // 最后重新锁 state，确保任何路径都不会同时持有两把互斥锁。
        let salt = self.persisted().salt.clone();
        let identity = match identity {
            UsageAccountIdentity::AccountId(value) => AccountIdentity::AccountId(value),
            UsageAccountIdentity::Token(value) => AccountIdentity::Token(value),
        };
        let Ok(fingerprint) = account_fingerprint(&salt, identity) else {
            log::warn!("额度自动接续无法建立脱敏账号指纹。");
            return false;
        };
        let weekly = weekly_window(snapshot);
        let reset_at = weekly.and_then(|window| window.reset_at);
        let remaining_percent = weekly.map(|window| window.remaining_percent);
        let window_seconds = weekly.map(|window| window.window_seconds);
        let window_fingerprint =
            weekly.map(|window| quota_window_fingerprint(&salt, &window.id, window.window_seconds));
        *self.observation() = Some(Observation {
            account_fingerprint: fingerprint.clone(),
            window_fingerprint: window_fingerprint.clone(),
            weekly_reset_at: reset_at,
            window_seconds,
        });
        let mut persisted = self.persisted();

        let previous = serde_json::to_vec(&*persisted).ok();
        let identity_changed = persisted.account_fingerprint.as_deref() != Some(&fingerprint)
            || window_fingerprint
                .as_ref()
                .is_some_and(|current| persisted.window_fingerprint.as_ref() != Some(current));
        let mut audit_records = Vec::new();
        if identity_changed {
            // 账号/窗口切换后的首个快照只建立 baseline，禁止补发旧账号事件。
            persisted.start_cycle(fingerprint, window_fingerprint, window_seconds, reset_at);
            persisted.migration_baseline_pending = false;
            persisted.last_observation =
                remaining_percent.map(|remaining_percent| PersistedObservation {
                    reset_at,
                    remaining_percent,
                    observed_at: now,
                });
        } else if persisted.migration_baseline_pending && weekly.is_some() {
            let expired_target_advanced = persisted.active_cycle.as_ref().is_some_and(|cycle| {
                cycle.expected_reset_at <= now
                    && reset_at.is_some_and(|value| value > cycle.expected_reset_at)
            });
            if expired_target_advanced || persisted.active_cycle.is_none() {
                // 旧 target 已经过期时，以首帧当前 reset_at 建立纯 baseline；不生成 event，
                // 不占 slot，也不把 v1 的过期排期解释成一次刚发生的重置。
                persisted.start_cycle(fingerprint, window_fingerprint, window_seconds, reset_at);
            } else if window_seconds.is_some() && persisted.window_seconds != window_seconds {
                // 未来 target 与已占槽位原样保留，仅补齐 v1 缺失的真实窗口秒数。
                persisted.window_seconds = window_seconds;
            }
            persisted.pending_reset_at = None;
            persisted.migration_baseline_pending = false;
            persisted.last_observation =
                remaining_percent.map(|remaining_percent| PersistedObservation {
                    reset_at,
                    remaining_percent,
                    observed_at: now,
                });
        } else {
            promote_resolved_cycle(&mut persisted, reset_at);
            if persisted.active_cycle.is_none() {
                persisted.start_cycle(fingerprint, window_fingerprint, window_seconds, reset_at);
            }

            let previous_observation = persisted.last_observation.clone();
            let quota_recovered = previous_observation.as_ref().is_some_and(|previous| {
                previous.remaining_percent < 100 && remaining_percent == Some(100)
            });
            if quota_recovered {
                let previous_remaining = previous_observation
                    .as_ref()
                    .map(|value| value.remaining_percent)
                    .unwrap_or(100);
                if let Some(event) = confirm_event_in_state(
                    &mut persisted,
                    QuotaResetReason::QuotaRecovered,
                    now,
                    previous_remaining,
                    100,
                    reset_at,
                ) {
                    audit_records.push(reset_confirmed_audit(&event, now));
                    if !enabled {
                        if let Some(cycle) = persisted.active_cycle.as_mut() {
                            cycle.request_completed = true;
                            cycle.phase = QuotaAutoContinuePhase::Succeeded;
                        }
                    }
                }
            }

            let mut pending_candidate = None;
            if let Some(cycle) = persisted.active_cycle.as_mut() {
                if let Some(current_reset) =
                    reset_at.filter(|value| *value > cycle.expected_reset_at)
                {
                    // 旧截止前观察到的任何更晚 reset_at 都只暂存为下一周期；
                    // 当前 active.expected 绝不能被刷新覆盖，否则旧截止到达时会漏确认事件。
                    pending_candidate = Some(current_reset);
                }

                let anchor = attempt_anchor(cycle);
                if cycle.confirmed_event.is_some()
                    && !cycle.request_completed
                    && now > anchor + ChronoDuration::seconds(ATTEMPT_OFFSETS_SECONDS[3])
                {
                    cycle.phase = QuotaAutoContinuePhase::Missed;
                    cycle.request_completed = true;
                    if let Some(event) = cycle.confirmed_event.as_ref() {
                        audit_records.push(
                            QuotaAuditRecord::warning(now, QuotaAuditAction::CycleMissed)
                                .with_event(
                                    event.event_id.clone(),
                                    event.generation_id.clone(),
                                    event.reason,
                                ),
                        );
                    }
                }
            }
            if let Some(next) = pending_candidate {
                persisted.pending_reset_at = Some(
                    persisted
                        .pending_reset_at
                        .map_or(next, |known| known.max(next)),
                );
            }

            let deadline_advanced = persisted.active_cycle.as_ref().is_some_and(|cycle| {
                cycle.confirmed_event.is_none()
                    && now >= cycle.expected_reset_at
                    && persisted
                        .pending_reset_at
                        .or(reset_at)
                        .is_some_and(|value| value > cycle.expected_reset_at)
            });
            if deadline_advanced {
                let previous_remaining = previous_observation
                    .as_ref()
                    .map(|value| value.remaining_percent)
                    .or(remaining_percent)
                    .unwrap_or(100);
                let current_remaining = remaining_percent.unwrap_or(previous_remaining);
                let next_reset_at = persisted.pending_reset_at.or(reset_at);
                if let Some(event) = confirm_event_in_state(
                    &mut persisted,
                    QuotaResetReason::DeadlineReached,
                    now,
                    previous_remaining,
                    current_remaining,
                    next_reset_at,
                ) {
                    audit_records.push(reset_confirmed_audit(&event, now));
                    if !enabled {
                        // 重置事件仍供通知和趋势消费，但关闭自动接续时本周期绝不补发历史 hi。
                        if let Some(cycle) = persisted.active_cycle.as_mut() {
                            cycle.request_completed = true;
                            cycle.phase = QuotaAutoContinuePhase::Succeeded;
                        }
                    }
                }
            }
            persisted.last_observation =
                remaining_percent.map(|remaining_percent| PersistedObservation {
                    reset_at,
                    remaining_percent,
                    observed_at: now,
                });
        }
        let changed = previous.as_deref() != serde_json::to_vec(&*persisted).ok().as_deref();
        if changed {
            if save_runtime_state(&self.path, &persisted).is_err() {
                log::warn!("额度自动接续状态持久化失败：类别=storage。");
            }
            drop(persisted);
            for record in audit_records {
                self.audit(record);
            }
            self.notify_schedule_changed();
        }
        changed
    }

    pub fn activate_cached_observation(&self, enabled: bool, now: DateTime<Utc>) {
        if !enabled {
            let mut persisted = self.persisted();
            let mut changed = false;
            if let Some(cycle) = persisted
                .active_cycle
                .as_mut()
                .filter(|cycle| cycle.confirmed_event.is_some() && !cycle.request_completed)
            {
                // 用户关闭开关即永久结案当前事件的自动消费者；通知 Pending 保持不变，
                // 之后重新开启也不能补发关闭期间的历史 hi。
                cycle.request_completed = true;
                cycle.phase = QuotaAutoContinuePhase::Succeeded;
                changed = true;
            }
            if changed && save_runtime_state(&self.path, &persisted).is_err() {
                log::warn!("额度自动接续禁用结案未能持久化：类别=storage。");
            }
            drop(persisted);
            self.notify_schedule_changed();
            return;
        }
        let observation = self.observation().clone();
        let Some(observation) = observation else {
            self.notify_schedule_changed();
            return;
        };
        let mut persisted = self.persisted();
        if persisted.account_fingerprint.as_deref() != Some(&observation.account_fingerprint)
            || observation
                .window_fingerprint
                .as_ref()
                .is_some_and(|current| persisted.window_fingerprint.as_ref() != Some(current))
            || persisted.active_cycle.is_none()
        {
            persisted.start_cycle(
                observation.account_fingerprint,
                observation.window_fingerprint,
                observation.window_seconds,
                observation.weekly_reset_at,
            );
        }
        if let Some(cycle) = persisted.active_cycle.as_mut() {
            if cycle.confirmed_event.is_some()
                && now > attempt_anchor(cycle) + ChronoDuration::seconds(ATTEMPT_OFFSETS_SECONDS[3])
            {
                cycle.phase = QuotaAutoContinuePhase::Missed;
                cycle.request_completed = true;
            }
        }
        if save_runtime_state(&self.path, &persisted).is_err() {
            log::warn!("额度自动接续状态持久化失败：类别=storage。");
        }
        drop(persisted);
        self.notify_schedule_changed();
    }
}
