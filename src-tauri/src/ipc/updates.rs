use crate::app::state::AppState;
use crate::domain::updates::AppUpdateInfo;
use crate::infra::app_update::check_for_update;
use crate::infra::app_update::install_pending_update;
use crate::infra::desktop::SETTINGS_WINDOW_LABEL;
use crate::ipc::access::require_known_window;
use crate::ipc::access::require_window_label;
use std::sync::Arc;
use tauri::AppHandle;
use tauri::State;
use tauri::WebviewWindow;

#[tauri::command]
pub(crate) async fn get_app_update_info(
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<AppUpdateInfo, String> {
    // 主卡只读取安全摘要以显示发现新版本的提醒；下载和安装始终限定在设置窗口。
    require_known_window(&window)?;
    Ok(state.shared.app_update.current_info().await)
}

#[tauri::command]
pub(crate) async fn check_app_update(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<AppUpdateInfo, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    check_for_update(&app, &state.shared.app_update, false).await
}

#[tauri::command]
pub(crate) async fn install_app_update(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    install_pending_update(&app, &state.shared.app_update).await
}
