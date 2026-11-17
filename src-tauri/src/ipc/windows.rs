use crate::app::lightweight;
use crate::app::state::resolved_language;
use crate::app::state::AppState;
use crate::infra::desktop::MAIN_WINDOW_LABEL;
use crate::infra::desktop::SETTINGS_WINDOW_LABEL;
use crate::ipc::access::require_window_label;
use std::sync::Arc;
use tauri::AppHandle;
use tauri::Emitter;
use tauri::Manager;
use tauri::State;
use tauri::WebviewWindow;

#[tauri::command]
pub(crate) async fn open_settings_window(
    section: Option<String>,
    window: WebviewWindow,
    app: AppHandle,
) -> Result<(), String> {
    require_window_label(&window, MAIN_WINDOW_LABEL)?;
    if section.as_deref().is_some_and(|value| value != "about") {
        return Err("无法打开指定的设置页面。".to_owned());
    }
    lightweight::show(&app, SETTINGS_WINDOW_LABEL, section.as_deref()).await?;
    if section.as_deref() == Some("about")
        && app
            .emit_to(SETTINGS_WINDOW_LABEL, "settings-navigate", "about")
            .is_err()
    {
        log::warn!("无法导航到更新设置页面。");
    }
    log::info!("已打开设置窗口。");
    Ok(())
}

#[tauri::command]
pub(crate) fn mark_main_size_manual(
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    require_window_label(&window, MAIN_WINDOW_LABEL)?;
    state.mark_main_size_manual()
}

#[tauri::command]
pub fn get_lightweight_measurement(
    window: WebviewWindow,
    app: AppHandle,
) -> Result<Option<String>, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let prefs = app.state::<Arc<AppState>>().current_settings();
    Ok(lightweight::summary(
        &app,
        resolved_language(prefs.language),
    ))
}

#[tauri::command]
pub async fn enter_lightweight(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let rt = app.state::<lightweight::Runtime>();
    let _guard = rt.transition.lock().await;
    lightweight::enter(&app).await
}
