use crate::app::account_hub::emit_accounts_changed;
use crate::app::account_hub::publish_selection;
use crate::app::account_hub::AccountHub;
use crate::app::account_hub::AccountsResponse;
use crate::app::continuation_scheduler::quota_auto_continue_error_key;
use crate::app::continuation_scheduler::start_quota_auto_continue_loop;
use crate::app::refresh::start_refresh_loop;
use crate::infra::accounts::AccountConfig;
use crate::infra::desktop::SETTINGS_WINDOW_LABEL;
use crate::ipc::access::require_known_window;
use crate::ipc::access::require_window_label;
use crate::ipc::events::emit_quota_auto_continue_status;
use chrono::Utc;
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::AppHandle;
use tauri::State;
use tauri::WebviewWindow;
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub async fn get_accounts(
    window: WebviewWindow,
    hub: State<'_, Arc<AccountHub>>,
) -> Result<AccountsResponse, String> {
    require_known_window(&window)?;
    Ok(hub.response(window.label() == SETTINGS_WINDOW_LABEL).await)
}

#[tauri::command]
pub async fn select_account(
    account_id: String,
    window: WebviewWindow,
    app: AppHandle,
    hub: State<'_, Arc<AccountHub>>,
) -> Result<AccountsResponse, String> {
    require_known_window(&window)?;
    {
        let _guard = hub.store.operation.lock().await;
        hub.store.select(&account_id)?;
    }
    publish_selection(&app, &hub).await?;
    Ok(hub.response(window.label() == SETTINGS_WINDOW_LABEL).await)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResponse {
    imported: usize,
    errors: Vec<String>,
}

#[tauri::command]
pub async fn import_accounts(
    window: WebviewWindow,
    app: AppHandle,
    hub: State<'_, Arc<AccountHub>>,
) -> Result<ImportResponse, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    // 原生选择器与读取均留在 Rust；WebView 不获得凭证内容或文件路径。
    let handle = app.clone();
    let files = tauri::async_runtime::spawn_blocking(move || {
        handle
            .dialog()
            .file()
            .add_filter("Codex auth JSON", &["json"])
            .blocking_pick_files()
    })
    .await
    .map_err(|_| "dialogFailed")?;
    let mut result = ImportResponse {
        imported: 0,
        errors: Vec::new(),
    };
    let Some(files) = files else {
        return Ok(result);
    };
    let guard = hub.store.operation.lock().await;
    for file in files {
        match file
            .into_path()
            .map_err(|_| "authUnreadable".to_owned())
            .and_then(|p| hub.store.import(&p))
        {
            Ok(_) => result.imported += 1,
            Err(error) => {
                log::warn!("Account import failed: category={error}");
                result.errors.push(error);
            }
        }
    }
    let added = hub.ensure_runtimes()?;
    drop(guard);
    for state in added {
        start_refresh_loop(app.clone(), state.clone());
        start_quota_auto_continue_loop(app.clone(), state);
    }
    publish_selection(&app, &hub).await?;
    Ok(result)
}

#[tauri::command]
pub async fn update_account(
    config: AccountConfig,
    window: WebviewWindow,
    app: AppHandle,
    hub: State<'_, Arc<AccountHub>>,
) -> Result<(), String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let state = hub.resolve(Some(&config.id))?;
    {
        let _guard = hub.store.operation.lock().await;
        hub.store.update(config.clone())?;
    }
    state
        .quota_auto_continue
        .activate_cached_observation(config.enabled && config.auto_continue, Utc::now());
    let _guard = state.schedule_guard.lock().await;
    state.reschedule_from_now_locked().await;
    emit_accounts_changed(&app);
    emit_quota_auto_continue_status(&app, &state);
    Ok(())
}

#[tauri::command]
pub async fn remove_account(
    account_id: String,
    window: WebviewWindow,
    app: AppHandle,
    hub: State<'_, Arc<AccountHub>>,
) -> Result<(), String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let state = hub.resolve(Some(&account_id))?;
    let _refresh = state.refresh_guard.lock().await;
    let _execution = state
        .quota_auto_continue
        .execution_guard()
        .map_err(quota_auto_continue_error_key)?;
    {
        let _guard = hub.store.operation.lock().await;
        hub.store.remove(&account_id)?;
    }
    state.retired.store(true, Ordering::Release);
    hub.runtimes
        .lock()
        .unwrap_or_else(|v| v.into_inner())
        .remove(&account_id);
    state.notify_schedule_changed();
    state
        .quota_auto_continue
        .activate_cached_observation(false, Utc::now());
    publish_selection(&app, &hub).await
}

#[tauri::command]
pub async fn apply_codex_account(
    account_id: String,
    window: WebviewWindow,
    hub: State<'_, Arc<AccountHub>>,
    app: AppHandle,
) -> Result<(), String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let _guard = hub.store.operation.lock().await;
    hub.store
        .apply_to_codex(&account_id)
        .inspect_err(|code| log::warn!("Codex credential application rejected: category={code}"))?;
    emit_accounts_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn restore_codex_account(
    window: WebviewWindow,
    hub: State<'_, Arc<AccountHub>>,
    app: AppHandle,
) -> Result<(), String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let _guard = hub.store.operation.lock().await;
    hub.store
        .restore_codex()
        .inspect_err(|code| log::warn!("Codex credential restore rejected: category={code}"))?;
    emit_accounts_changed(&app);
    Ok(())
}
