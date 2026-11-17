use crate::app::continuation::PreflightDecision;
use crate::app::continuation::QuotaAutoContinueErrorCode;
use crate::app::refresh::refresh_and_emit;
use crate::app::state::AppState;
use crate::domain::usage::DashboardErrorCode;
use crate::ipc::events::emit_quota_auto_continue_status;
use chrono::DateTime;
use chrono::Utc;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;

pub(crate) const QUOTA_AUTO_CONTINUE_IDLE_WAIT: Duration = Duration::from_secs(365 * 24 * 60 * 60);

/// 将自动接续的绝对执行时间转换为 Tokio 等待时长。
///
/// `None` 才表示当前没有任务；已到期或因取时差产生轻微负值的任务必须返回零，
/// 否则会误用空闲等待并跳过本应立即占用的尝试槽。
pub(crate) fn quota_auto_continue_wait_duration(
    next_attempt_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Duration {
    match next_attempt_at {
        Some(deadline) => (deadline - now).to_std().unwrap_or(Duration::ZERO),
        None => QUOTA_AUTO_CONTINUE_IDLE_WAIT,
    }
}

pub(crate) fn quota_auto_continue_error_key(error: QuotaAutoContinueErrorCode) -> String {
    match error {
        QuotaAutoContinueErrorCode::AuthMissing => "authMissing",
        QuotaAutoContinueErrorCode::AuthInvalid => "authInvalid",
        QuotaAutoContinueErrorCode::Network => "network",
        QuotaAutoContinueErrorCode::RateLimited => "rateLimited",
        QuotaAutoContinueErrorCode::ServiceUnavailable => "serviceUnavailable",
        QuotaAutoContinueErrorCode::InvalidResponse => "invalidResponse",
        QuotaAutoContinueErrorCode::NoTextModel => "noTextModel",
        QuotaAutoContinueErrorCode::AccountChanged => "accountChanged",
        QuotaAutoContinueErrorCode::Persistence => "persistence",
        QuotaAutoContinueErrorCode::Busy => "busy",
    }
    .to_owned()
}

pub(crate) fn start_quota_auto_continue_loop(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let mut receiver = state.quota_auto_continue.subscribe();
        loop {
            if state.retired.load(Ordering::Acquire)
                || state.account.as_ref().is_some_and(|a| a.config().is_none())
            {
                break;
            }
            let enabled = state.current_settings().quota_auto_continue_enabled;
            let now = Utc::now();
            let status = state.quota_auto_continue.status(enabled, now);
            // 没有任务时由 watch 变更提前唤醒；已到期任务则以零等待立即执行。
            let wait = quota_auto_continue_wait_duration(status.next_attempt_at, now);
            tokio::select! {
                changed = receiver.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
                _ = tokio::time::sleep(wait) => {
                    run_scheduled_quota_auto_continue(&app, &state).await;
                }
            }
        }
    });
}

pub(crate) async fn run_scheduled_quota_auto_continue(app: &AppHandle, state: &AppState) {
    let enabled = state.current_settings().quota_auto_continue_enabled;
    let Ok(_guard) = state.quota_auto_continue.execution_guard() else {
        // 手动测试正在运行时不占用自动尝试槽；短暂延后后由当前排期再次判断。
        tokio::time::sleep(Duration::from_secs(1)).await;
        state
            .quota_auto_continue
            .activate_cached_observation(enabled, Utc::now());
        return;
    };
    let attempt = match state
        .quota_auto_continue
        .claim_due_attempt(enabled, Utc::now())
    {
        Ok(Some(attempt)) => attempt,
        Ok(None) => {
            emit_quota_auto_continue_status(app, state);
            return;
        }
        Err(error) => {
            log::warn!("额度自动接续无法占用尝试槽：类别={error:?}。");
            emit_quota_auto_continue_status(app, state);
            return;
        }
    };
    log::info!(
        "额度自动接续开始尝试：序号={}、目标重置时间={}。",
        attempt.slot_index + 1,
        attempt.target_reset_at.to_rfc3339()
    );
    emit_quota_auto_continue_status(app, state);

    if !refresh_and_emit(app, state).await {
        let snapshot = state.current_snapshot().await;
        let error = snapshot
            .message
            .map(quota_error_from_dashboard)
            .unwrap_or(QuotaAutoContinueErrorCode::Network);
        state
            .quota_auto_continue
            .finish_failure(&attempt, error, None);
        log::warn!(
            "额度自动接续只读预检失败：类别={error:?}、序号={}。",
            attempt.slot_index + 1
        );
        emit_quota_auto_continue_status(app, state);
        return;
    }

    match state.quota_auto_continue.preflight_decision(&attempt) {
        PreflightDecision::AlreadyAdvanced => {
            state
                .quota_auto_continue
                .mark_already_advanced(&attempt, Utc::now());
            log::info!("额度自动接续无需发送：当前事件已由其他执行路径结案。");
            emit_quota_auto_continue_status(app, state);
            return;
        }
        PreflightDecision::AccountChanged => {
            state.quota_auto_continue.finish_failure(
                &attempt,
                QuotaAutoContinueErrorCode::AccountChanged,
                None,
            );
            log::info!("额度自动接续已取消旧任务：当前账号已变化。");
            emit_quota_auto_continue_status(app, state);
            return;
        }
        PreflightDecision::Proceed => {}
    }

    // 只读预检期间用户可能关闭开关；真实 POST 前必须重新读取已落盘设置。
    if !state.current_settings().quota_auto_continue_enabled {
        state
            .quota_auto_continue
            .activate_cached_observation(false, Utc::now());
        log::info!("额度自动接续已在真实请求前取消：用户关闭了功能。");
        emit_quota_auto_continue_status(app, state);
        return;
    }

    match state.quota_auto_continue.send_for_attempt(&attempt).await {
        Ok(model) => {
            state
                .quota_auto_continue
                .finish_sent(&attempt, model.clone(), Utc::now());
            log::info!(
                "额度自动接续最小请求成功：序号={}、模型={}。",
                attempt.slot_index + 1,
                model
            );
            emit_quota_auto_continue_status(app, state);
            // 发送成功即停止本周期重试；后续刷新仅用于确认新 reset_at。
            let _ = refresh_and_emit(app, state).await;
            emit_quota_auto_continue_status(app, state);
        }
        Err(failure) => {
            log::warn!(
                "额度自动接续最小请求失败：序号={}、类别={:?}、模型={}。",
                attempt.slot_index + 1,
                failure.code,
                failure.model.as_deref().unwrap_or("未选择")
            );
            state
                .quota_auto_continue
                .finish_failure(&attempt, failure.code, failure.model);
            emit_quota_auto_continue_status(app, state);
        }
    }
}

pub(crate) fn quota_error_from_dashboard(error: DashboardErrorCode) -> QuotaAutoContinueErrorCode {
    match error {
        DashboardErrorCode::AuthMissing => QuotaAutoContinueErrorCode::AuthMissing,
        DashboardErrorCode::AuthInvalid => QuotaAutoContinueErrorCode::AuthInvalid,
        DashboardErrorCode::Network | DashboardErrorCode::LocalBridge => {
            QuotaAutoContinueErrorCode::Network
        }
        DashboardErrorCode::RateLimited => QuotaAutoContinueErrorCode::RateLimited,
        DashboardErrorCode::ServiceUnavailable => QuotaAutoContinueErrorCode::ServiceUnavailable,
        DashboardErrorCode::InvalidResponse => QuotaAutoContinueErrorCode::InvalidResponse,
    }
}

#[cfg(test)]
mod tests;
