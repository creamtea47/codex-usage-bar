use crate::app::notifications::notification_body;
use crate::app::notifications::reset_credit_notification_allowed;
use crate::domain::notifications::NotificationBatch;
use crate::domain::notifications::NotificationReason;
use crate::domain::settings::Language;
use crate::domain::settings::Settings;
use crate::domain::usage::DashboardSnapshot;
use crate::domain::usage::DashboardStatus;
use crate::domain::usage::QuotaFallbackLabel;
use crate::infra::credit_baseline::ResetCreditIncrease;

#[test]
fn notification_body_ignores_account_and_upstream_window_labels() {
    let snapshot = DashboardSnapshot {
        account_id: None,
        status: DashboardStatus::Ready,
        account_email_masked: Some("s***@private.example".to_owned()),
        extra_credits: None,
        plan_label: Some("Sensitive Plan".to_owned()),
        refreshed_at: None,
        next_refresh_at: None,
        message: None,
        quota_windows: vec![crate::domain::usage::QuotaWindow {
            id: "internal-stream".to_owned(),
            label: Some("Sensitive upstream label".to_owned()),
            fallback_label: QuotaFallbackLabel::Weekly,
            remaining_percent: 20,
            used_percent: 80,
            window_seconds: 7 * 24 * 60 * 60,
            reset_at: None,
            reset_after_seconds: 60,
            start_at: None,
            show_pace_marker: true,
            forecast: None,
        }],
    };
    let body = notification_body(
        Some(&NotificationBatch {
            items: vec![crate::domain::notifications::NotificationItem {
                window_index: 0,
                reasons: vec![NotificationReason::LowRemaining {
                    remaining_percent: 20,
                }],
            }],
            omitted_window_count: 0,
        }),
        &snapshot,
        None,
        Language::En,
    );

    assert_eq!(body, "Weekly limit: 20% remaining");
    assert!(!body.contains("private.example"));
    assert!(!body.contains("Sensitive"));
    assert!(!body.contains("internal-stream"));
}

#[test]
fn reset_credit_line_merges_with_quota_reasons_in_both_languages() {
    let snapshot = DashboardSnapshot {
        account_id: None,
        status: DashboardStatus::Ready,
        account_email_masked: None,
        extra_credits: None,
        plan_label: None,
        refreshed_at: None,
        next_refresh_at: None,
        message: None,
        quota_windows: vec![crate::domain::usage::QuotaWindow {
            id: "weekly".to_owned(),
            label: None,
            fallback_label: QuotaFallbackLabel::Weekly,
            remaining_percent: 20,
            used_percent: 80,
            window_seconds: 7 * 24 * 60 * 60,
            reset_at: None,
            reset_after_seconds: 60,
            start_at: None,
            show_pace_marker: true,
            forecast: None,
        }],
    };
    let batch = NotificationBatch {
        items: vec![crate::domain::notifications::NotificationItem {
            window_index: 0,
            reasons: vec![NotificationReason::LowRemaining {
                remaining_percent: 20,
            }],
        }],
        omitted_window_count: 0,
    };
    let increase = ResetCreditIncrease {
        gained_count: 1,
        available_count: 2,
        applicable_available_count: Some(0),
    };

    assert_eq!(
        notification_body(Some(&batch), &snapshot, Some(&increase), Language::ZhCn),
        "重置卡：新获得 1 张，当前共 2 张（可使用 0 张）\n周限额: 剩余 20%"
    );
    assert_eq!(
        notification_body(Some(&batch), &snapshot, Some(&increase), Language::En),
        "Reset credits: received 1, 2 total (0 currently usable)\nWeekly limit: 20% remaining"
    );
    assert_eq!(
        notification_body(None, &snapshot, Some(&increase), Language::ZhCn),
        "重置卡：新获得 1 张，当前共 2 张（可使用 0 张）"
    );
}

#[test]
fn reset_credit_alert_requires_both_switches_and_is_suppressed_during_quiet_hours() {
    let mut settings = Settings::default();
    assert!(!reset_credit_notification_allowed(&settings, 12 * 60));

    settings.notifications.enabled = true;
    settings.notifications.reset_credit_enabled = false;
    assert!(!reset_credit_notification_allowed(&settings, 12 * 60));

    settings.notifications.reset_credit_enabled = true;
    assert!(reset_credit_notification_allowed(&settings, 12 * 60));

    settings.notifications.quiet_hours_enabled = true;
    settings.notifications.quiet_hours_start = "22:00".to_owned();
    settings.notifications.quiet_hours_end = "08:00".to_owned();
    assert!(!reset_credit_notification_allowed(&settings, 23 * 60));
    assert!(reset_credit_notification_allowed(&settings, 12 * 60));
}
