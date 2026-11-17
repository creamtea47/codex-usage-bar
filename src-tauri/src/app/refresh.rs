use crate::app::account_hub;
use crate::app::notifications::alerts_allowed;
use crate::app::notifications::send_usage_notifications;
use crate::app::state::resolved_language;
use crate::app::state::AppState;
use crate::app::state::HistoryRefreshEffect;
use crate::app::state::RefreshResult;
use crate::app::state::ResetCreditObservation;
use crate::domain::refresh_scheduler::failure_retry_seconds;
use crate::domain::settings::Language;
use crate::domain::usage::DashboardErrorCode;
use crate::domain::usage::DashboardSnapshot;
use crate::domain::usage::DashboardStatus;
use crate::infra::credit_baseline as reset_credit_notification;
use crate::infra::network;
use crate::infra::usage;
use crate::ipc::events::emit_dashboard;
use crate::ipc::events::emit_quota_auto_continue_status;
use crate::ipc::events::emit_usage_history_updated;
use chrono::DateTime;
use chrono::Duration as ChronoDuration;
use chrono::Utc;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_notification::NotificationExt;

impl AppState {
    pub(crate) async fn fetch_account_dashboard(
        &self,
    ) -> Result<usage::FetchedDashboard, usage::UsageError> {
        if let Some(account) = &self.account {
            let credentials = account.credentials(false).await?;
            match self.usage_client.fetch_with_credentials(credentials).await {
                Err(usage::UsageError::Unauthorized) => {
                    self.usage_client
                        .fetch_with_credentials(account.credentials(true).await?)
                        .await
                }
                result => result,
            }
        } else {
            self.usage_client.fetch_dashboard().await
        }
    }

    pub(crate) fn notify_schedule_changed(&self) {
        self.schedule_sender.send_modify(|revision| {
            *revision = revision.wrapping_add(1);
        });
    }

    pub(crate) fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures.load(Ordering::Relaxed)
    }

    pub(crate) fn reset_notification_baseline(&self) {
        self.notification_tracker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .reset_baseline();
    }

    pub(crate) fn retry_delay_seconds(&self) -> u64 {
        let failures = self.consecutive_failures();
        if failures == 0 {
            self.current_settings().refresh_interval_seconds
        } else {
            failure_retry_seconds(failures)
        }
    }

    pub(crate) async fn reschedule_from_now_locked(&self) -> DashboardSnapshot {
        let delay = self.retry_delay_seconds();
        let mut snapshot = self.snapshot.lock().await;
        snapshot.next_refresh_at = Some(Utc::now() + ChronoDuration::seconds(delay as i64));
        let result = snapshot.clone();
        drop(snapshot);
        self.notify_schedule_changed();
        result
    }
}

impl AppState {
    pub(crate) async fn refresh_dashboard(&self) -> RefreshResult {
        // 定时刷新与手动刷新同时到达时，复用现有快照，避免重复携带凭据发出请求。
        let observed_generation = self.refresh_generation.load(Ordering::Acquire);
        let _guard = match self.refresh_guard.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                let completed = self.refresh_guard.lock().await;
                if refresh_completed_since(
                    observed_generation,
                    self.refresh_generation.load(Ordering::Acquire),
                    self.completed_network_revision.load(Ordering::Acquire),
                    self.network_revision.load(Ordering::Acquire),
                ) {
                    // 单飞竞争者等待当前刷新结束后复用最终快照，不发第二个请求或重复广播。
                    let snapshot = self.current_snapshot().await;
                    drop(completed);
                    return RefreshResult {
                        started: std::time::Instant::now(),
                        snapshot,
                        was_successful: false,
                        should_emit: false,
                        history_effect: HistoryRefreshEffect::default(),
                        quota_auto_continue_changed: false,
                        reset_credit_observation: None,
                    };
                }
                // 锁若只由历史开关/清除操作占用，不得吞掉用户或定时刷新。
                completed
            }
        };

        let (network_revision, started, fetched) = loop {
            let generation = self.network_revision.load(Ordering::Acquire);
            let started = std::time::Instant::now();
            let result = self.fetch_account_dashboard().await;
            if generation == self.network_revision.load(Ordering::Acquire) {
                break (generation, started, result);
            }
        };
        let result = match fetched {
            Ok(fetched) => {
                self.reset_credit_cache
                    .lock()
                    .unwrap_or_else(|v| v.into_inner())
                    .observe_count(fetched.reset_credit_availability.map(|v| v.available_count));
                *self
                    .account_profile
                    .lock()
                    .unwrap_or_else(|v| v.into_inner()) = Some(fetched.profile.clone());
                let mut fresh = fetched.snapshot;
                fresh.account_id = self.account.as_ref().map(|a| a.id.clone());
                let quota_auto_continue_changed = self.quota_auto_continue.observe_dashboard(
                    self.current_settings().quota_auto_continue_enabled,
                    &fetched.account_identity,
                    &fresh,
                    Utc::now(),
                );
                // 统一识别器必须先换代，趋势才能把当前成功快照写入正确 generation。
                let history_effect = self
                    .apply_history_to_snapshot(&mut fresh, &fetched.account_identity)
                    .await;
                self.consecutive_failures.store(0, Ordering::Relaxed);
                let _schedule_guard = self.schedule_guard.lock().await;
                // 必须在请求完成后、持有排期锁时重读，避免旧请求覆盖新设置。
                let interval = self.current_settings().refresh_interval_seconds;
                fresh.next_refresh_at = Some(Utc::now() + ChronoDuration::seconds(interval as i64));
                log::info!(
                    "用量刷新成功：窗口数={}，下次刷新间隔={}秒",
                    fresh.quota_windows.len(),
                    interval
                );
                let mut snapshot = self.snapshot.lock().await;
                *snapshot = fresh.clone();
                drop(snapshot);
                self.notify_schedule_changed();
                RefreshResult {
                    started,
                    snapshot: fresh,
                    was_successful: true,
                    should_emit: true,
                    history_effect,
                    quota_auto_continue_changed,
                    reset_credit_observation: Some(ResetCreditObservation {
                        account_identity: fetched.account_identity,
                        availability: fetched.reset_credit_availability,
                    }),
                }
            }
            Err(error) => {
                let failures = self
                    .consecutive_failures
                    .fetch_add(1, Ordering::Relaxed)
                    .saturating_add(1);
                let retry_seconds = failure_retry_seconds(failures);
                let _schedule_guard = self.schedule_guard.lock().await;
                let mut snapshot = self.snapshot.lock().await;
                snapshot.account_id = self.account.as_ref().map(|a| a.id.clone());
                snapshot.status =
                    if snapshot.quota_windows.is_empty() && snapshot.extra_credits.is_none() {
                        DashboardStatus::Error
                    } else {
                        DashboardStatus::Stale
                    };
                snapshot.message = Some(error.code());
                snapshot.next_refresh_at =
                    Some(Utc::now() + ChronoDuration::seconds(retry_seconds as i64));
                let result = snapshot.clone();
                drop(snapshot);
                self.notify_schedule_changed();
                log::warn!(
                    "用量刷新失败：类别={:?}、连续失败={}、下次重试={}秒",
                    error.code(),
                    failures,
                    retry_seconds
                );
                RefreshResult {
                    started,
                    snapshot: result,
                    was_successful: false,
                    should_emit: true,
                    history_effect: HistoryRefreshEffect::default(),
                    quota_auto_continue_changed: false,
                    reset_credit_observation: None,
                }
            }
        };
        self.completed_network_revision
            .store(network_revision, Ordering::Release);
        self.refresh_generation.fetch_add(1, Ordering::Release);
        result
    }
}

pub(crate) fn refresh_completed_since(
    observed_generation: u64,
    current_generation: u64,
    completed_network: u64,
    current_network: u64,
) -> bool {
    observed_generation != current_generation && completed_network == current_network
}

pub(crate) fn scheduled_deadline_is_current(
    expected_revision: u64,
    current_revision: u64,
    expected_deadline: Option<DateTime<Utc>>,
    current_deadline: Option<DateTime<Utc>>,
) -> bool {
    expected_revision == current_revision && expected_deadline == current_deadline
}

pub(crate) fn start_refresh_loop(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let mut schedule_receiver = state.schedule_sender.subscribe();
        if state
            .account
            .as_ref()
            .is_none_or(|a| a.config().is_some_and(|c| c.enabled))
        {
            refresh_and_emit(&app, &state).await;
        }
        loop {
            if state.retired.load(Ordering::Acquire) {
                break;
            }
            if let Some(account) = &state.account {
                let Some(config) = account.config() else {
                    break;
                };
                if !config.enabled {
                    if schedule_receiver.changed().await.is_err() {
                        break;
                    }
                    continue;
                }
            }
            let revision = *schedule_receiver.borrow_and_update();
            let deadline = state.current_snapshot().await.next_refresh_at;
            let wait = deadline
                .as_ref()
                .and_then(|deadline| (*deadline - Utc::now()).to_std().ok())
                .unwrap_or(Duration::ZERO);
            tokio::select! {
                biased;
                changed = schedule_receiver.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
                _ = tokio::time::sleep(wait) => {
                    // 与 deadline 写入使用同一把锁。设置变更若先完成，这里会看到
                    // 新修订号并丢弃旧 sleep；若这里先验证，则本次刷新已正式开始。
                    let schedule_guard = state.schedule_guard.lock().await;
                    let current_revision = *schedule_receiver.borrow();
                    let current_deadline = state.current_snapshot().await.next_refresh_at;
                    let should_refresh = scheduled_deadline_is_current(
                        revision,
                        current_revision,
                        deadline,
                        current_deadline,
                    );
                    drop(schedule_guard);
                    if should_refresh {
                        refresh_and_emit(&app, &state).await;
                    }
                }
            }
        }
    });
}

pub(crate) async fn refresh_and_emit(app: &AppHandle, state: &AppState) -> bool {
    let result = state.refresh_dashboard().await;
    let was_successful = result.was_successful;
    let _ = finalize_refresh_result(app, state, result).await;
    was_successful
}

pub(crate) fn refresh_result_is_current(
    result: &RefreshResult,
    current: &DashboardSnapshot,
) -> bool {
    result.should_emit && result.snapshot == *current
}

/**
 * 把刷新结果的广播纳入 refresh_guard 提交序。
 * 历史开关或清除若已先更新当前快照，旧刷新只能返回最新快照，不能重新发布旧预测与旧高度。
 */
pub(crate) async fn finalize_refresh_result(
    app: &AppHandle,
    state: &AppState,
    result: RefreshResult,
) -> DashboardSnapshot {
    let _publish_barrier = state.refresh_guard.lock().await;
    let current = state.current_snapshot().await;
    if state.completed_network_revision.load(Ordering::Acquire)
        != state.network_revision.load(Ordering::Acquire)
        || !refresh_result_is_current(&result, &current)
    {
        return current;
    }
    if result.should_emit {
        let offline = result.snapshot.message == Some(DashboardErrorCode::Network);
        if (offline || result.was_successful)
            && app
                .state::<network::Connectivity>()
                .observe(result.started, offline)
        {
            let settings = state.current_settings();
            if settings.notifications.connection_enabled && alerts_allowed(&settings) {
                let body = match (resolved_language(settings.language), offline) {
                    (Language::ZhCn, true) => "无法连接 Codex 用量服务，将自动重试。",
                    (Language::ZhCn, false) => "连接已恢复，已读取最新 Codex 用量。",
                    (_, true) => "Cannot connect to Codex usage. Retrying automatically.",
                    (_, false) => "Connection restored. Codex usage is up to date.",
                };
                let _ = app
                    .notification()
                    .builder()
                    .title("CodexUsageBar")
                    .body(body)
                    .show();
            }
        }
    }
    if result.should_emit && result.was_successful {
        let reset_credit_increase =
            result
                .reset_credit_observation
                .as_ref()
                .and_then(|observation| {
                    state
                        .reset_credit_notifications
                        .observe(&observation.account_identity, observation.availability)
                });
        if let Some(observation) = result.reset_credit_observation.as_ref() {
            if let Some(increase) = state.reset_credit_notifications.observe_extra(
                &observation.account_identity,
                result.snapshot.extra_credits.as_ref(),
            ) {
                let settings = state.current_settings();
                if settings.notifications.extra_credit_enabled && alerts_allowed(&settings) {
                    use reset_credit_notification::ExtraIncrease::*;
                    let body = match (resolved_language(settings.language), increase) {
                        (Language::ZhCn, Balance(delta, total)) => {
                            format!("Codex 额度到账 +{delta}，当前余额 {total}")
                        }
                        (Language::ZhCn, Available) => "Codex 额外额度已可用".into(),
                        (_, Balance(delta, total)) => {
                            format!("Codex credits increased by {delta}. Balance: {total}")
                        }
                        (_, Available) => "Codex extra credits are now available".into(),
                    };
                    let label = state
                        .account
                        .as_ref()
                        .and_then(|a| a.config())
                        .map(|a| a.label)
                        .unwrap_or_else(|| "CodexUsageBar".into());
                    let _ = app.notification().builder().title(label).body(body).show();
                }
            }
        }
        send_usage_notifications(
            app,
            state,
            &result.snapshot,
            result.history_effect.account_changed,
            reset_credit_increase,
        );
    }
    if result.should_emit && result.history_effect.changed {
        emit_usage_history_updated(app, state.account.as_ref().map(|a| a.id.clone()));
    }
    if result.should_emit && result.quota_auto_continue_changed {
        emit_quota_auto_continue_status(app, state);
    }
    let snapshot = result.snapshot;
    account_hub::emit_accounts_changed(app);
    if result.should_emit && account_hub::is_selected(app, state) {
        state.apply_auto_main_height(app, &snapshot);
        emit_dashboard(app, snapshot.clone());
    }
    snapshot
}

#[cfg(test)]
mod tests;
