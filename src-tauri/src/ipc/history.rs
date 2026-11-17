use crate::app::account_hub;
use crate::app::account_hub::resolve;
use crate::app::history::AnalyticsRequest;
use crate::app::history::UsageHistoryResponse;
use crate::app::state::AppState;
use crate::domain::history::UsageHistoryRequest;
use crate::domain::settings::Settings;
use crate::infra::desktop::SETTINGS_WINDOW_LABEL;
use crate::ipc::access::require_window_label;
use std::sync::Arc;
use tauri::AppHandle;
use tauri::Manager;
use tauri::State;
use tauri::WebviewWindow;

#[tauri::command]
pub(crate) async fn get_usage_history(
    request: UsageHistoryRequest,
    account_id: Option<String>,
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<UsageHistoryResponse, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let state = account_hub::resolve(window.app_handle(), &state, account_id.as_deref())?;
    crate::app::history::query(&state, request).await
}

#[tauri::command]
pub(crate) async fn set_history_enabled(
    enabled: bool,
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Settings, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    crate::app::history::set_collection_enabled(&state, &app, enabled).await
}

#[tauri::command]
pub(crate) async fn clear_usage_history(
    account_id: Option<String>,
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let state = account_hub::resolve(window.app_handle(), &state, account_id.as_deref())?;
    crate::app::history::clear_account(&state, &app).await
}

#[tauri::command]
pub async fn get_usage_analytics(
    account_id: Option<String>,
    request: AnalyticsRequest,
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let state = resolve(&app, &state, account_id.as_deref())?;
    crate::app::history::analytics(&state, request).await
}
