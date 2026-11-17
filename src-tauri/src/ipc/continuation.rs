use crate::app::account_hub;
use crate::app::account_hub::AccountHub;
use crate::app::continuation as quota_auto_continue;
use crate::app::continuation::QuotaAutoContinueStatus;
use crate::app::continuation_scheduler::quota_auto_continue_error_key;
use crate::app::refresh::refresh_and_emit;
use crate::app::state::AppState;
use crate::infra::desktop::SETTINGS_WINDOW_LABEL;
use crate::ipc::access::require_window_label;
use crate::ipc::events::emit_quota_auto_continue_status;
use chrono::Utc;
use std::sync::Arc;
use tauri::AppHandle;
use tauri::Emitter;
use tauri::Manager;
use tauri::State;
use tauri::WebviewWindow;

#[tauri::command]
pub(crate) fn get_quota_auto_continue_status(
    account_id: Option<String>,
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<QuotaAutoContinueStatus, String> {
    let state = account_hub::resolve(window.app_handle(), &state, account_id.as_deref())?;
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let enabled = state.current_settings().quota_auto_continue_enabled;
    Ok(state.quota_auto_continue.status(enabled, Utc::now()))
}

#[tauri::command]
pub(crate) async fn set_quota_auto_continue_enabled(
    enabled: bool,
    account_id: Option<String>,
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<QuotaAutoContinueStatus, String> {
    let state = account_hub::resolve(window.app_handle(), &state, account_id.as_deref())?;
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let mut next = state.current_settings();
    next.quota_auto_continue_enabled = enabled;
    let saved = if let Some(account) = &state.account {
        let _guard = account.store.operation.lock().await;
        let mut config = account.config().ok_or("accountMissing")?;
        config.auto_continue = enabled;
        account.store.update(config)?;
        account_hub::emit_accounts_changed(&app);
        next
    } else {
        state.save_preferences(next)?
    };
    state
        .quota_auto_continue
        .activate_cached_observation(enabled, Utc::now());
    if account_hub::is_selected(&app, &state) && app.emit("settings-updated", saved).is_err() {
        log::warn!("无法广播额度自动接续设置变更。");
    }
    if enabled
        && state
            .quota_auto_continue
            .status(true, Utc::now())
            .target_reset_at
            .is_none()
    {
        let _ = refresh_and_emit(&app, &state).await;
    }
    emit_quota_auto_continue_status(&app, &state);
    log::info!("额度自动接续设置已保存：启用={enabled}。");
    Ok(state.quota_auto_continue.status(enabled, Utc::now()))
}

#[tauri::command]
pub(crate) async fn test_quota_auto_continue(
    account_id: Option<String>,
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<QuotaAutoContinueStatus, String> {
    let state = account_hub::resolve(window.app_handle(), &state, account_id.as_deref())?;
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let _guard = state
        .quota_auto_continue
        .execution_guard()
        .map_err(quota_auto_continue_error_key)?;
    let enabled = state.current_settings().quota_auto_continue_enabled;
    let mut running = state.quota_auto_continue.status(enabled, Utc::now());
    running.phase = crate::app::continuation::QuotaAutoContinuePhase::Running;
    let _ = app.emit_to(
        SETTINGS_WINDOW_LABEL,
        "quota-auto-continue-updated",
        running,
    );

    match state.quota_auto_continue.send_manual_test().await {
        Ok(model) => {
            state
                .quota_auto_continue
                .record_manual_success(model.clone(), Utc::now());
            log::info!("额度自动接续手动测试成功：模型={model}。");
            let _ = refresh_and_emit(&app, &state).await;
            emit_quota_auto_continue_status(&app, &state);
            Ok(state.quota_auto_continue.status(enabled, Utc::now()))
        }
        Err(failure) => {
            log::warn!(
                "额度自动接续手动测试失败：类别={:?}、模型={}。",
                failure.code,
                failure.model.as_deref().unwrap_or("未选择")
            );
            state.quota_auto_continue.record_manual_failure(&failure);
            emit_quota_auto_continue_status(&app, &state);
            Err(quota_auto_continue_error_key(failure.code))
        }
    }
}

#[tauri::command]
pub async fn get_account_models(
    account_id: String,
    window: WebviewWindow,
    hub: State<'_, Arc<AccountHub>>,
) -> Result<Vec<quota_auto_continue::ModelOption>, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    hub.resolve(Some(&account_id))?
        .quota_auto_continue
        .models()
        .await
        .map_err(quota_auto_continue_error_key)
}
