use crate::app::account_hub::AccountHub;
use crate::infra::desktop::SETTINGS_WINDOW_LABEL;
use crate::ipc::access::require_window_label;
use tauri::AppHandle;
use tauri::State;
use tauri::WebviewWindow;

use crate::infra::login::authorize;
use crate::infra::login::BrowserLogin;
use crate::infra::login::TIMEOUT;
#[tauri::command]
pub async fn login_and_import(
    window: WebviewWindow,
    app: AppHandle,
    login: State<'_, BrowserLogin>,
    hub: State<'_, std::sync::Arc<AccountHub>>,
) -> Result<(), String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let mut cancelled = login.begin()?;
    let result = tokio::select! {
        biased;
        _ = cancelled.changed() => Err("loginCancelled".into()),
        result = tokio::time::timeout(TIMEOUT, authorize(&app)) => result.unwrap_or_else(|_| Err("loginTimeout".into())),
    };
    let result = match result {
        Ok(doc) => hub.import_login(&app, &doc).await,
        Err(error) => Err(error),
    };
    login.0.lock().unwrap_or_else(|v| v.into_inner()).take();
    use tauri::Emitter;
    let _ = app.emit_to(SETTINGS_WINDOW_LABEL, "account-login-finished", ());
    result
}

#[tauri::command]
pub fn cancel_account_login(
    window: WebviewWindow,
    login: State<'_, BrowserLogin>,
) -> Result<(), String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    if let Some(tx) = login.0.lock().unwrap_or_else(|v| v.into_inner()).as_mut() {
        tx.send_replace(true);
    }
    Ok(())
}

#[tauri::command]
pub fn account_login_pending(
    window: WebviewWindow,
    login: State<'_, BrowserLogin>,
) -> Result<bool, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    Ok(login.0.lock().unwrap_or_else(|v| v.into_inner()).is_some())
}
