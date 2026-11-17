use crate::app::account_hub::AccountHub;
use crate::app::state::normalize_general_settings;
use crate::app::state::resolved_language;
use crate::app::state::AppState;
use crate::app::tray;
use crate::app::windows::settings_window_title;
use crate::domain::settings::Settings;
use crate::infra::desktop::MAIN_WINDOW_LABEL;
use crate::infra::desktop::SETTINGS_WINDOW_LABEL;
use crate::ipc::access::require_known_window;
use crate::ipc::access::require_window_label;
use crate::ipc::events::emit_dashboard;
use std::sync::Arc;
use tauri::AppHandle;
use tauri::Emitter;
use tauri::Manager;
use tauri::State;
use tauri::WebviewWindow;
use tauri_plugin_autostart::ManagerExt;

#[tauri::command]
pub(crate) async fn get_settings(
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<Settings, String> {
    // 主卡用于首屏主题/锁定状态，设置窗口用于编辑；两者均只收到非敏感偏好。
    require_known_window(&window)?;
    Ok(state.current_settings())
}

#[tauri::command]
pub(crate) async fn save_settings(
    settings: Settings,
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Settings, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let previous_settings = state.current_settings();
    let next_settings = normalize_general_settings(settings, &previous_settings);
    let main_window = app.get_webview_window(MAIN_WINDOW_LABEL);
    if let Some(main_window) = &main_window {
        // 先应用可回滚的原生窗口状态，成功后才落盘，避免“UI 报错但配置已保存”。
        main_window
            .set_always_on_top(next_settings.always_on_top)
            .map_err(|_| "无法更新窗口置顶状态。".to_owned())?;
        if main_window
            .set_resizable(!next_settings.lock_position)
            .is_err()
        {
            let _ = main_window.set_always_on_top(previous_settings.always_on_top);
            return Err("无法更新窗口锁定状态。".to_owned());
        }
    }
    let should_reset_notification_baseline = next_settings.language != previous_settings.language
        || next_settings.notifications != previous_settings.notifications;
    // 设置发布与 deadline 替换共用一把排期锁：旧 sleep 要么先开始刷新，
    // 要么看到新的修订号，不能在两者之间使用已取消的截止时间。
    let schedule_guard = state.schedule_guard.lock().await;
    let stored = {
        let _notification_evaluation = state.notification_evaluation();
        let saved = match state.save_preferences(next_settings) {
            Ok(saved) => saved,
            Err(error) => {
                if let Some(window) = &main_window {
                    let _ = window.set_always_on_top(previous_settings.always_on_top);
                    let _ = window.set_resizable(!previous_settings.lock_position);
                }
                log::warn!("挂件设置持久化失败，已尝试恢复主窗口状态。");
                return Err(error);
            }
        };
        if should_reset_notification_baseline {
            state.reset_notification_baseline();
        }
        saved
    };
    let snapshot = state.reschedule_from_now_locked().await;
    drop(schedule_guard);
    if app.emit("settings-updated", stored.clone()).is_err() {
        log::warn!("无法向界面广播已保存设置。");
    }
    if let Some(settings_window) = app.get_webview_window(SETTINGS_WINDOW_LABEL) {
        let _ = settings_window.set_title(settings_window_title(stored.language));
    }
    tray::update_menu(&app, resolved_language(stored.language));
    if let Some(hub) = app.try_state::<Arc<AccountHub>>() {
        for child in hub.all() {
            if should_reset_notification_baseline {
                let _guard = child.notification_evaluation();
                child.reset_notification_baseline();
            }
            let _guard = child.schedule_guard.lock().await;
            let refreshed = child.reschedule_from_now_locked().await;
            emit_dashboard(&app, refreshed);
        }
    }
    emit_dashboard(&app, snapshot);
    Ok(stored)
}

#[tauri::command]
pub(crate) fn get_autostart(window: WebviewWindow, app: AppHandle) -> Result<bool, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    app.autolaunch()
        .is_enabled()
        .map_err(|_| "无法读取开机自启状态。".to_owned())
}

#[tauri::command]
pub(crate) fn set_autostart(
    enabled: bool,
    window: WebviewWindow,
    app: AppHandle,
) -> Result<bool, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let autostart = app.autolaunch();
    let result = if enabled {
        autostart.enable()
    } else {
        autostart.disable()
    };
    result.map_err(|_| "无法更新开机自启设置。".to_owned())?;
    log::info!("已更新开机自启状态：{}", enabled);
    Ok(enabled)
}
