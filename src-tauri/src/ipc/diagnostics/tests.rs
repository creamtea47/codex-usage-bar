use super::*;
use crate::domain::settings::Theme;
use crate::domain::usage::DashboardErrorCode;
use crate::domain::usage::DashboardStatus;
use crate::infra::desktop::MAIN_WINDOW_LABEL;
use crate::infra::desktop::SETTINGS_WINDOW_LABEL;
use crate::ipc::diagnostics::validate_settings_ui_fault_report;
use crate::ipc::diagnostics::SettingsUiFaultCode;

#[test]
fn diagnostics_are_limited_to_the_fixed_safe_field_set() {
    let permission = NotificationPermission::Denied;
    let status = DashboardStatus::Stale;
    let theme = Theme::Dark;
    let report = build_diagnostics(DiagnosticsSummary {
        app_version: "0.3.0",
        platform: "test-os",
        architecture: "test-arch",
        language_preference: Language::ZhCn,
        resolved_language: Language::ZhCn,
        theme: &theme,
        refresh_interval_seconds: 300,
        consecutive_failures: 2,
        notifications_enabled: false,
        notification_permission: &permission,
        dashboard_status: &status,
        dashboard_error_code: Some(DashboardErrorCode::Network),
        window_count: 2,
        last_refresh_at: None,
        update_status: "current",
        update_last_checked_at: None,
        history_enabled: true,
        history_sample_count: 12,
        history_storage_status: HistoryStorageStatus::Ready,
        minimize_to_tray_on_close: true,
        quota_auto_continue_enabled: true,
        quota_auto_continue_phase: QuotaAutoContinuePhase::Scheduled,
    });
    let fields = report
        .lines()
        .map(|line| line.split_once(':').unwrap().0)
        .collect::<Vec<_>>();
    assert_eq!(
        fields,
        vec![
            "CodexUsageBar diagnostics schema",
            "appVersion",
            "platform",
            "architecture",
            "languagePreference",
            "resolvedLanguage",
            "theme",
            "refreshIntervalSeconds",
            "consecutiveFailures",
            "notificationsEnabled",
            "notificationPermission",
            "dashboardStatus",
            "dashboardErrorCode",
            "windowCount",
            "lastRefreshAt",
            "updateStatus",
            "updateLastCheckedAt",
            "historyEnabled",
            "historySampleCount",
            "historyStorageStatus",
            "minimizeToTrayOnClose",
            "quotaAutoContinueEnabled",
            "quotaAutoContinuePhase",
        ]
    );
    let normalized = report.to_ascii_lowercase();
    for forbidden in [
        "accesstoken",
        "accountemail",
        "authpath",
        "proxy",
        "https://",
        "remainingpercent",
        "resetat",
        "historypoints",
        "rawerror",
        "rawlog",
    ] {
        assert!(
            !normalized.contains(forbidden),
            "found {forbidden} in {report}"
        );
    }
}

#[test]
fn settings_ui_fault_reporting_is_fixed_and_window_scoped() {
    let fault_code: SettingsUiFaultCode =
        serde_json::from_str(r#""trends-render-failed""#).unwrap();
    assert_eq!(
        validate_settings_ui_fault_report(SETTINGS_WINDOW_LABEL, fault_code),
        Ok("trends-render-failed")
    );
    assert!(validate_settings_ui_fault_report(MAIN_WINDOW_LABEL, fault_code).is_err());
    assert!(serde_json::from_str::<SettingsUiFaultCode>(r#""raw-error-text""#).is_err());
}
