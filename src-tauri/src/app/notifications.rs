use crate::app::state::resolved_language;
use crate::app::state::AppState;
use crate::domain::notifications::MatchedQuotaResetEvent;
use crate::domain::notifications::NotificationBatch;
use crate::domain::notifications::NotificationPolicy;
use crate::domain::notifications::NotificationReason;
use crate::domain::notifications::NotificationSnapshot;
use crate::domain::notifications::NotificationWindow;
use crate::domain::notifications::QuietHours;
use crate::domain::quota_reset::NotificationDisposition;
use crate::domain::settings::Language;
use crate::domain::settings::NotificationPermission;
use crate::domain::settings::Settings;
use crate::domain::usage::DashboardSnapshot;
use crate::domain::usage::QuotaFallbackLabel;
use crate::infra::credit_baseline::ResetCreditIncrease;
use chrono::DateTime;
use chrono::Local;
use chrono::Timelike;
use chrono::Utc;
use tauri::plugin::PermissionState;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

pub(crate) fn local_time_minutes(value: &str) -> u16 {
    value
        .split_once(':')
        .and_then(|(hour, minute)| Some((hour.parse::<u16>().ok()?, minute.parse::<u16>().ok()?)))
        .filter(|(hour, minute)| *hour < 24 && *minute < 60)
        .map(|(hour, minute)| hour * 60 + minute)
        .unwrap_or(0)
}

pub(crate) fn notification_policy(settings: &Settings) -> NotificationPolicy {
    let notifications = &settings.notifications;
    NotificationPolicy {
        enabled: notifications.enabled,
        low_remaining_enabled: notifications.low_quota_enabled,
        pace_deficit_enabled: notifications.pace_enabled,
        reset_enabled: notifications.reset_enabled,
        low_remaining_threshold: notifications.low_quota_threshold_percent,
        pace_deficit_threshold: notifications.pace_deficit_threshold_percent,
        quiet_hours: QuietHours {
            enabled: notifications.quiet_hours_enabled,
            start_minute: local_time_minutes(&notifications.quiet_hours_start),
            end_minute: local_time_minutes(&notifications.quiet_hours_end),
        },
    }
}

pub(crate) fn alerts_allowed(settings: &Settings) -> bool {
    let now = Local::now();
    let policy = notification_policy(settings);
    policy.enabled
        && !policy
            .quiet_hours
            .contains((now.hour() * 60 + now.minute()) as u16)
}

pub(crate) fn reset_credit_notification_allowed(
    settings: &Settings,
    local_minute_of_day: u16,
) -> bool {
    let policy = notification_policy(settings);
    policy.enabled
        && settings.notifications.reset_credit_enabled
        && !policy.quiet_hours.contains(local_minute_of_day)
}

/// 统一识别器只暴露脱敏窗口范围；调用层在同一成功快照中选择相同范围、
/// 最早重置的窗口，从而关联通知和趋势，同时不把上游窗口 ID 写入状态文件。
pub(crate) fn quota_window_index(
    snapshot: &DashboardSnapshot,
    window_seconds: i64,
) -> Option<usize> {
    snapshot
        .quota_windows
        .iter()
        .enumerate()
        .filter(|(_, window)| window.window_seconds == window_seconds && window.reset_at.is_some())
        .min_by_key(|(_, window)| window.reset_at)
        .map(|(index, _)| index)
}

pub(crate) fn localized_quota_label(
    window: &crate::domain::usage::QuotaWindow,
    language: Language,
) -> String {
    match (resolved_language(language), window.fallback_label) {
        (Language::ZhCn, QuotaFallbackLabel::FiveHour) => "5 小时限额".to_owned(),
        (Language::ZhCn, QuotaFallbackLabel::Weekly) => "周限额".to_owned(),
        (Language::ZhCn, QuotaFallbackLabel::Window) => "额度窗口".to_owned(),
        (_, QuotaFallbackLabel::FiveHour) => "5-hour limit".to_owned(),
        (_, QuotaFallbackLabel::Weekly) => "Weekly limit".to_owned(),
        (_, QuotaFallbackLabel::Window) => "Quota window".to_owned(),
    }
}

pub(crate) fn notification_body(
    batch: Option<&NotificationBatch>,
    snapshot: &DashboardSnapshot,
    reset_credit_increase: Option<&ResetCreditIncrease>,
    language: Language,
) -> String {
    let language = resolved_language(language);
    let mut lines = Vec::new();
    if let Some(increase) = reset_credit_increase {
        let line = match (language, increase.applicable_available_count) {
            (Language::ZhCn, Some(applicable)) => format!(
                "重置卡：新获得 {} 张，当前共 {} 张（可使用 {} 张）",
                increase.gained_count, increase.available_count, applicable
            ),
            (Language::ZhCn, None) => format!(
                "重置卡：新获得 {} 张，当前共 {} 张",
                increase.gained_count, increase.available_count
            ),
            (_, Some(applicable)) => format!(
                "Reset credits: received {}, {} total ({} currently usable)",
                increase.gained_count, increase.available_count, applicable
            ),
            (_, None) => format!(
                "Reset credits: received {}, {} total",
                increase.gained_count, increase.available_count
            ),
        };
        lines.push(line);
    }
    for item in batch.into_iter().flat_map(|value| &value.items) {
        let Some(window) = snapshot.quota_windows.get(item.window_index) else {
            continue;
        };
        let label = localized_quota_label(window, language);
        let reasons = item
            .reasons
            .iter()
            .map(|reason| match (language, reason) {
                (Language::ZhCn, NotificationReason::LowRemaining { remaining_percent }) => {
                    format!("剩余 {remaining_percent}%")
                }
                (Language::ZhCn, NotificationReason::PaceDeficit { deficit_points }) => {
                    format!("消耗进度快 {deficit_points} 个百分点")
                }
                (Language::ZhCn, NotificationReason::Reset) => "额度已重置".to_owned(),
                (_, NotificationReason::LowRemaining { remaining_percent }) => {
                    format!("{remaining_percent}% remaining")
                }
                (_, NotificationReason::PaceDeficit { deficit_points }) => {
                    format!("{deficit_points} points ahead of pace")
                }
                (_, NotificationReason::Reset) => "quota reset".to_owned(),
            })
            .collect::<Vec<_>>()
            .join(if language == Language::ZhCn {
                "、"
            } else {
                ", "
            });
        lines.push(format!("{label}: {reasons}"));
    }
    lines.join("\n")
}

pub(crate) fn send_usage_notifications(
    app: &AppHandle,
    state: &AppState,
    snapshot: &DashboardSnapshot,
    account_changed: bool,
    reset_credit_increase: Option<ResetCreditIncrease>,
) {
    // 与设置发布/baseline 重建互斥，避免用新阈值评估旧 tracker 后补发历史事件。
    let _notification_evaluation = state.notification_evaluation();
    let settings = state.current_settings();
    if account_changed {
        state.reset_notification_baseline();
    }
    let policy = notification_policy(&settings);
    let now_local = Local::now();
    let local_minute_of_day = now_local.hour() as u16 * 60 + now_local.minute() as u16;
    let windows = snapshot
        .quota_windows
        .iter()
        .map(|window| NotificationWindow {
            key: window.id.clone(),
            remaining_percent: window.remaining_percent,
            reset_at_unix_seconds: window.reset_at.map(|value| value.timestamp()),
            start_at_unix_seconds: window.start_at.map(|value| value.timestamp()),
            window_seconds: window.window_seconds,
            is_long_period: window.window_seconds >= 24 * 3600,
        })
        .collect::<Vec<_>>();
    let observed_at = snapshot.refreshed_at.unwrap_or_else(Utc::now);
    let pending_event = state.quota_auto_continue.pending_reset_event(observed_at);
    let matched_event = pending_event.as_ref().and_then(|event| {
        quota_window_index(snapshot, event.window_seconds).map(|window_index| {
            MatchedQuotaResetEvent {
                window_index,
                event,
            }
        })
    });
    let confirmed_resets = matched_event.as_slice();
    let batch = state
        .notification_tracker
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .evaluate_with_confirmed_resets(
            &NotificationSnapshot {
                observed_at_unix_seconds: observed_at.timestamp(),
                local_minute_of_day,
                windows: &windows,
            },
            &policy,
            confirmed_resets,
            |event_at| notification_event_is_quiet(&policy, event_at),
        );

    let matched_window_index = matched_event.as_ref().map(|matched| matched.window_index);
    if let Some(event) = pending_event
        .as_ref()
        .filter(|_| matched_window_index.is_some())
    {
        let disposition = if !policy.enabled || !policy.reset_enabled {
            Some(NotificationDisposition::SuppressedDisabled)
        } else if notification_event_is_quiet(&policy, event.detected_at.timestamp())
            || policy.quiet_hours.contains(local_minute_of_day)
        {
            Some(NotificationDisposition::SuppressedQuiet)
        } else {
            None
        };
        if let Some(disposition) = disposition {
            state.quota_auto_continue.mark_notification_suppressed(
                &event.event_id,
                disposition,
                Utc::now(),
            );
        }
    }

    let reset_credit_increase = reset_credit_increase
        .filter(|_| reset_credit_notification_allowed(&settings, local_minute_of_day));
    if batch.is_none() && reset_credit_increase.is_none() {
        return;
    }
    let language = resolved_language(settings.language);
    let title = if language == Language::ZhCn {
        "Codex 额度提醒"
    } else {
        "Codex quota alert"
    };
    let body = notification_body(
        batch.as_ref(),
        snapshot,
        reset_credit_increase.as_ref(),
        language,
    );
    if body.is_empty() {
        return;
    }
    let body = state
        .account
        .as_ref()
        .and_then(|a| a.config())
        .map_or_else(|| body.clone(), |a| format!("{}\n{}", a.label, body));
    let reset_event_id = pending_event.as_ref().and_then(|event| {
        let window_index = matched_window_index?;
        batch
            .as_ref()
            .is_some_and(|batch| {
                batch.items.iter().any(|item| {
                    item.window_index == window_index
                        && item.reasons.contains(&NotificationReason::Reset)
                })
            })
            .then_some(event.event_id.as_str())
    });
    // 先持久化领取通知事件，再调用平台，避免进程在 show() 附近退出后重复投递；
    // `queued` 只在平台调用成功后记录，保证审计日志描述的是真实调用结果。
    if let Some(event_id) = reset_event_id {
        if !state
            .quota_auto_continue
            .mark_notification_claimed(event_id, Utc::now())
        {
            log::warn!("额度重置通知未能持久化领取状态，已取消本次平台调用。");
            return;
        }
    }
    let platform_result = app.notification().builder().title(title).body(body).show();
    match platform_result {
        Ok(()) => {
            if let Some(event_id) = reset_event_id {
                if !state
                    .quota_auto_continue
                    .mark_notification_queued(event_id, Utc::now())
                {
                    log::warn!("额度重置通知平台调用成功，但处置结果未能持久化。");
                }
            }
        }
        Err(_) => {
            if let Some(event_id) = reset_event_id {
                state
                    .quota_auto_continue
                    .mark_notification_failed(event_id, Utc::now());
            }
            log::warn!("系统通知发送失败：类别=platform。");
        }
    }
}

pub(crate) fn notification_event_is_quiet(policy: &NotificationPolicy, event_at: i64) -> bool {
    DateTime::<Utc>::from_timestamp(event_at, 0)
        .map(|value| value.with_timezone(&Local))
        .is_some_and(|local| {
            policy
                .quiet_hours
                .contains(local.hour() as u16 * 60 + local.minute() as u16)
        })
}

pub(crate) fn map_notification_permission(permission: PermissionState) -> NotificationPermission {
    match permission {
        PermissionState::Granted => NotificationPermission::Granted,
        PermissionState::Denied => NotificationPermission::Denied,
        PermissionState::Prompt | PermissionState::PromptWithRationale => {
            NotificationPermission::Prompt
        }
    }
}

#[cfg(test)]
mod tests;
