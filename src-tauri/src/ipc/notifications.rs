use crate::app::notifications::map_notification_permission;
use crate::app::state::resolved_language;
use crate::app::state::AppState;
use crate::domain::settings::Language;
use crate::domain::settings::NotificationEnableResult;
use crate::domain::settings::NotificationPermission;
use crate::infra::desktop::SETTINGS_WINDOW_LABEL;
use crate::ipc::access::require_window_label;
use crate::ipc::events::emit_dashboard;
use std::sync::Arc;
use tauri::plugin::PermissionState;
use tauri::AppHandle;
use tauri::Emitter;
use tauri::State;
use tauri::WebviewWindow;
use tauri_plugin_notification::NotificationExt;

#[tauri::command]
pub(crate) async fn set_notifications_enabled(
    enabled: bool,
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<NotificationEnableResult, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let permission = if enabled {
        let current = app
            .notification()
            .permission_state()
            .map_err(|_| "notificationUnavailable".to_owned())?;
        let resolved = match current {
            PermissionState::Prompt | PermissionState::PromptWithRationale => app
                .notification()
                .request_permission()
                .map_err(|_| "notificationUnavailable".to_owned())?,
            state => state,
        };
        map_notification_permission(resolved)
    } else {
        app.notification()
            .permission_state()
            .map(map_notification_permission)
            .unwrap_or(NotificationPermission::Unavailable)
    };
    let actual_enabled = enabled && permission == NotificationPermission::Granted;
    let schedule_guard = state.schedule_guard.lock().await;
    let saved = {
        let _notification_evaluation = state.notification_evaluation();
        let mut settings = state.current_settings();
        settings.notifications.enabled = actual_enabled;
        let saved = state.save_preferences(settings)?;
        state.reset_notification_baseline();
        saved
    };
    let snapshot = state.reschedule_from_now_locked().await;
    drop(schedule_guard);
    if app.emit("settings-updated", saved.clone()).is_err() {
        log::warn!("无法广播通知设置变更。");
    }
    emit_dashboard(&app, snapshot);
    Ok(NotificationEnableResult {
        enabled: actual_enabled,
        permission,
        settings: saved,
    })
}

#[tauri::command]
pub(crate) fn send_test_notification(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let settings = state.current_settings();
    if !settings.notifications.enabled {
        return Err("notificationDisabled".to_owned());
    }
    if app
        .notification()
        .permission_state()
        .map_err(|_| "notificationUnavailable".to_owned())?
        != PermissionState::Granted
    {
        return Err("notificationPermissionDenied".to_owned());
    }
    let language = resolved_language(settings.language);
    let (title, body) = if language == Language::ZhCn {
        ("CodexUsageBar 通知测试", "通知已成功提交给系统。")
    } else {
        (
            "CodexUsageBar notification test",
            "The notification was submitted to the system.",
        )
    };
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|_| "notificationUnavailable".to_owned())
}
