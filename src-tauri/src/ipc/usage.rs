use crate::app::account_hub;
use crate::app::account_hub::AccountHub;
use crate::app::refresh::finalize_refresh_result;
use crate::app::state::AppState;
use crate::domain::usage::DashboardSnapshot;
use crate::infra::desktop::MAIN_WINDOW_LABEL;
use crate::infra::desktop::SETTINGS_WINDOW_LABEL;
use crate::infra::usage;
use crate::ipc::access::require_window_label;
use chrono::Utc;
use std::sync::Arc;
use tauri::AppHandle;
use tauri::Manager;
use tauri::State;
use tauri::WebviewWindow;

#[tauri::command]
pub(crate) async fn get_dashboard(
    account_id: Option<String>,
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<DashboardSnapshot, String> {
    let state = account_hub::resolve(window.app_handle(), &state, account_id.as_deref())?;
    // 此命令只服务主悬浮卡，设置窗口不会接触用量快照。
    require_window_label(&window, MAIN_WINDOW_LABEL)?;
    Ok(state.current_snapshot().await)
}

#[tauri::command]
pub(crate) async fn refresh_dashboard(
    account_id: Option<String>,
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<DashboardSnapshot, String> {
    let state = account_hub::resolve(window.app_handle(), &state, account_id.as_deref())?;
    require_window_label(&window, MAIN_WINDOW_LABEL)?;
    let result = state.refresh_dashboard().await;
    Ok(finalize_refresh_result(&app, &state, result).await)
}

#[tauri::command]
pub async fn get_reset_credit_details(
    account_id: String,
    force: Option<bool>,
    window: WebviewWindow,
    hub: State<'_, Arc<AccountHub>>,
) -> Result<crate::domain::reset_credit_details::ResetCreditDetails, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let runtime = hub.resolve(Some(&account_id))?;
    let _flight = runtime.reset_credit_query_guard.lock().await;
    let generation = {
        let cache = runtime
            .reset_credit_cache
            .lock()
            .unwrap_or_else(|v| v.into_inner());
        if !force.unwrap_or(false) {
            if let Some(value) = cache.get(Utc::now()) {
                return Ok(value);
            }
        }
        cache.generation()
    };
    let account = runtime.account.as_ref().ok_or("accountMissing")?;
    let result = async {
        let credentials = account.credentials(false).await?;
        match runtime
            .usage_client
            .fetch_reset_credit_details(credentials)
            .await
        {
            Err(usage::UsageError::Unauthorized) => {
                runtime
                    .usage_client
                    .fetch_reset_credit_details(account.credentials(true).await?)
                    .await
            }
            value => value,
        }
    }
    .await;
    if let Err(error) = &result {
        log::warn!(
            "Reset credit detail read failed: account={}, category={:?}",
            &account_id[..8],
            error.code()
        );
    }
    let response = runtime
        .reset_credit_cache
        .lock()
        .unwrap_or_else(|v| v.into_inner())
        .finish(
            generation,
            &account_id,
            result.map_err(|e| e.code()),
            Utc::now(),
        );
    Ok(response)
}
