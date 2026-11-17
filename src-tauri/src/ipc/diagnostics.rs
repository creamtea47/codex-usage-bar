use crate::app::continuation::QuotaAutoContinuePhase;
use crate::app::notifications::map_notification_permission;
use crate::app::state::resolved_language;
use crate::app::state::AppState;
use crate::domain::history::HistoryStorageStatus;
use crate::domain::settings::Language;
use crate::domain::settings::NotificationPermission;
use crate::domain::usage::DashboardStatus;
use crate::infra::desktop::SETTINGS_WINDOW_LABEL;
use crate::ipc::access::has_exact_window_label;
use crate::ipc::access::require_window_label;
use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use std::fmt::Write as _;
use std::sync::Arc;
use tauri::AppHandle;
use tauri::Manager;
use tauri::State;
use tauri::WebviewWindow;
use tauri_plugin_notification::NotificationExt;

/// 设置页只能上报预先约定的脱敏故障类别，禁止把异常文本、路径或页面数据写入日志。
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SettingsUiFaultCode {
    TrendsRenderFailed,
}

impl SettingsUiFaultCode {
    fn as_log_code(self) -> &'static str {
        match self {
            Self::TrendsRenderFailed => "trends-render-failed",
        }
    }
}

pub(crate) fn validate_settings_ui_fault_report(
    actual_label: &str,
    fault_code: SettingsUiFaultCode,
) -> Result<&'static str, String> {
    if !has_exact_window_label(actual_label, SETTINGS_WINDOW_LABEL) {
        return Err("当前窗口无权执行该操作。".to_owned());
    }
    Ok(fault_code.as_log_code())
}

#[tauri::command]
pub(crate) fn report_settings_ui_fault(
    fault_code: SettingsUiFaultCode,
    window: WebviewWindow,
) -> Result<(), String> {
    let stable_code = validate_settings_ui_fault_report(window.label(), fault_code)?;
    log::warn!("设置界面报告故障：类别={stable_code}。");
    Ok(())
}

pub(crate) fn language_name(language: Language) -> &'static str {
    match language {
        Language::System => "system",
        Language::ZhCn => "zh-CN",
        Language::En => "en",
    }
}

pub(crate) fn theme_name(theme: &crate::domain::settings::Theme) -> &'static str {
    match theme {
        crate::domain::settings::Theme::System => "system",
        crate::domain::settings::Theme::Light => "light",
        crate::domain::settings::Theme::Dark => "dark",
    }
}

pub(crate) fn notification_permission_name(permission: &NotificationPermission) -> &'static str {
    match permission {
        NotificationPermission::Granted => "granted",
        NotificationPermission::Denied => "denied",
        NotificationPermission::Prompt => "prompt",
        NotificationPermission::Unavailable => "unavailable",
    }
}

pub(crate) fn dashboard_status_name(status: &DashboardStatus) -> &'static str {
    match status {
        DashboardStatus::Loading => "loading",
        DashboardStatus::Ready => "ready",
        DashboardStatus::Stale => "stale",
        DashboardStatus::Error => "error",
    }
}

pub(crate) fn dashboard_error_code_name(
    code: Option<crate::domain::usage::DashboardErrorCode>,
) -> &'static str {
    match code {
        Some(crate::domain::usage::DashboardErrorCode::AuthMissing) => "authMissing",
        Some(crate::domain::usage::DashboardErrorCode::AuthInvalid) => "authInvalid",
        Some(crate::domain::usage::DashboardErrorCode::Network) => "network",
        Some(crate::domain::usage::DashboardErrorCode::RateLimited) => "rateLimited",
        Some(crate::domain::usage::DashboardErrorCode::ServiceUnavailable) => "serviceUnavailable",
        Some(crate::domain::usage::DashboardErrorCode::InvalidResponse) => "invalidResponse",
        Some(crate::domain::usage::DashboardErrorCode::LocalBridge) => "localBridge",
        None => "none",
    }
}

pub(crate) fn history_storage_status_name(status: HistoryStorageStatus) -> &'static str {
    match status {
        HistoryStorageStatus::Missing => "missing",
        HistoryStorageStatus::Ready => "ready",
        HistoryStorageStatus::RecoveredCorrupt => "recoveredCorrupt",
        HistoryStorageStatus::RecoveredUnsupported => "recoveredUnsupported",
        HistoryStorageStatus::Unavailable => "unavailable",
    }
}

pub(crate) fn quota_auto_continue_phase_name(phase: QuotaAutoContinuePhase) -> &'static str {
    match phase {
        QuotaAutoContinuePhase::Disabled => "disabled",
        QuotaAutoContinuePhase::WaitingForWeeklyWindow => "waitingForWeeklyWindow",
        QuotaAutoContinuePhase::Scheduled => "scheduled",
        QuotaAutoContinuePhase::Running => "running",
        QuotaAutoContinuePhase::WaitingForRetry => "waitingForRetry",
        QuotaAutoContinuePhase::Succeeded => "succeeded",
        QuotaAutoContinuePhase::SentAwaitingConfirmation => "sentAwaitingConfirmation",
        QuotaAutoContinuePhase::AuthenticationRequired => "authenticationRequired",
        QuotaAutoContinuePhase::Missed => "missed",
        QuotaAutoContinuePhase::Failed => "failed",
    }
}

pub(crate) fn diagnostic_timestamp(value: Option<DateTime<Utc>>) -> String {
    value
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| "none".to_owned())
}

pub(crate) struct DiagnosticsSummary<'a> {
    app_version: &'a str,
    platform: &'a str,
    architecture: &'a str,
    language_preference: Language,
    resolved_language: Language,
    theme: &'a crate::domain::settings::Theme,
    refresh_interval_seconds: u64,
    consecutive_failures: u32,
    notifications_enabled: bool,
    notification_permission: &'a NotificationPermission,
    dashboard_status: &'a DashboardStatus,
    dashboard_error_code: Option<crate::domain::usage::DashboardErrorCode>,
    window_count: usize,
    last_refresh_at: Option<DateTime<Utc>>,
    update_status: &'a str,
    update_last_checked_at: Option<DateTime<Utc>>,
    history_enabled: bool,
    history_sample_count: u32,
    history_storage_status: HistoryStorageStatus,
    minimize_to_tray_on_close: bool,
    quota_auto_continue_enabled: bool,
    quota_auto_continue_phase: QuotaAutoContinuePhase,
}

/// 诊断输出只接受固定白名单摘要，类型中没有账号、Token、路径、URL、百分比、
/// reset 时间、历史点或原始错误/日志，因此这些数据无法被意外格式化进报告。
pub(crate) fn build_diagnostics(summary: DiagnosticsSummary<'_>) -> String {
    let mut report = String::with_capacity(768);
    let _ = writeln!(report, "CodexUsageBar diagnostics schema: 2");
    let _ = writeln!(report, "appVersion: {}", summary.app_version);
    let _ = writeln!(report, "platform: {}", summary.platform);
    let _ = writeln!(report, "architecture: {}", summary.architecture);
    let _ = writeln!(
        report,
        "languagePreference: {}",
        language_name(summary.language_preference)
    );
    let _ = writeln!(
        report,
        "resolvedLanguage: {}",
        language_name(summary.resolved_language)
    );
    let _ = writeln!(report, "theme: {}", theme_name(summary.theme));
    let _ = writeln!(
        report,
        "refreshIntervalSeconds: {}",
        summary.refresh_interval_seconds
    );
    let _ = writeln!(
        report,
        "consecutiveFailures: {}",
        summary.consecutive_failures
    );
    let _ = writeln!(
        report,
        "notificationsEnabled: {}",
        summary.notifications_enabled
    );
    let _ = writeln!(
        report,
        "notificationPermission: {}",
        notification_permission_name(summary.notification_permission)
    );
    let _ = writeln!(
        report,
        "dashboardStatus: {}",
        dashboard_status_name(summary.dashboard_status)
    );
    let _ = writeln!(
        report,
        "dashboardErrorCode: {}",
        dashboard_error_code_name(summary.dashboard_error_code)
    );
    let _ = writeln!(report, "windowCount: {}", summary.window_count);
    let _ = writeln!(
        report,
        "lastRefreshAt: {}",
        diagnostic_timestamp(summary.last_refresh_at)
    );
    let _ = writeln!(report, "updateStatus: {}", summary.update_status);
    let _ = writeln!(
        report,
        "updateLastCheckedAt: {}",
        diagnostic_timestamp(summary.update_last_checked_at)
    );
    let _ = writeln!(report, "historyEnabled: {}", summary.history_enabled);
    let _ = writeln!(
        report,
        "historySampleCount: {}",
        summary.history_sample_count
    );
    let _ = writeln!(
        report,
        "historyStorageStatus: {}",
        history_storage_status_name(summary.history_storage_status)
    );
    let _ = writeln!(
        report,
        "minimizeToTrayOnClose: {}",
        summary.minimize_to_tray_on_close
    );
    let _ = writeln!(
        report,
        "quotaAutoContinueEnabled: {}",
        summary.quota_auto_continue_enabled
    );
    let _ = writeln!(
        report,
        "quotaAutoContinuePhase: {}",
        quota_auto_continue_phase_name(summary.quota_auto_continue_phase)
    );
    report
}

#[tauri::command]
pub(crate) async fn get_diagnostics(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let settings = state.current_settings();
    let snapshot = state.current_snapshot().await;
    let update = state.shared.app_update.current_info().await;
    let runtime = state.shared.usage_history.lock().await;
    let history = runtime.history.summary();
    let permission = app
        .notification()
        .permission_state()
        .map(map_notification_permission)
        .unwrap_or(NotificationPermission::Unavailable);
    let update_status = if update.checked_at.is_none() {
        "notChecked"
    } else if update.update_available {
        "available"
    } else {
        "current"
    };
    let quota_auto_continue = state
        .quota_auto_continue
        .status(settings.quota_auto_continue_enabled, Utc::now());

    Ok(build_diagnostics(DiagnosticsSummary {
        app_version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        language_preference: settings.language,
        resolved_language: resolved_language(settings.language),
        theme: &settings.theme,
        refresh_interval_seconds: settings.refresh_interval_seconds,
        consecutive_failures: state.consecutive_failures(),
        notifications_enabled: settings.notifications.enabled,
        notification_permission: &permission,
        dashboard_status: &snapshot.status,
        dashboard_error_code: snapshot.message,
        window_count: app.webview_windows().len(),
        last_refresh_at: snapshot.refreshed_at,
        update_status,
        update_last_checked_at: update.checked_at,
        history_enabled: settings.history_enabled,
        history_sample_count: history.sample_count,
        history_storage_status: runtime.storage_status,
        minimize_to_tray_on_close: settings.minimize_to_tray_on_close,
        quota_auto_continue_enabled: settings.quota_auto_continue_enabled,
        quota_auto_continue_phase: quota_auto_continue.phase,
    }))
}

#[cfg(test)]
mod tests;
