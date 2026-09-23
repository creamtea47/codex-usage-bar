use super::*;
use crate::accounts::{AccountConfig, AccountStore, AccountView, ManagedAccount};
use std::{fs, path::Path};
use tauri_plugin_dialog::DialogExt;

/// 复用现有单账号状态机；每个状态机独立锁定刷新、通知与接续，共享窗口设置和历史存储。
pub(crate) struct AccountHub {
    pub store: Arc<AccountStore>,
    root: Arc<AppState>,
    runtimes: StdMutex<HashMap<String, Arc<AppState>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRow {
    #[serde(flatten)]
    account: AccountView,
    dashboard: DashboardSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<crate::account_profile::AccountDetails>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountsResponse {
    selected_id: Option<String>,
    accounts: Vec<AccountRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    codex_login: Option<crate::account_profile::CodexLoginSource>,
}

impl AccountHub {
    pub fn new(root: Arc<AppState>) -> Result<Arc<Self>, String> {
        let store = AccountStore::open(root.settings_path.with_file_name("accounts"))?;
        // 首次升级导入当前文件并保持其官方刷新归属；历史仍使用原文件与原盐值。
        if store.configs().is_empty() && !store.directory.join("accounts.json").exists() {
            if let Ok(path) = auth::resolve_auth_json_path() {
                if store.import(&path).is_err() {
                    log::warn!("初始账号导入失败：保留原认证文件。");
                }
            }
        }
        let hub = Arc::new(Self {
            store,
            root,
            runtimes: StdMutex::new(HashMap::new()),
        });
        hub.ensure_runtimes()?;
        Ok(hub)
    }

    pub fn all(&self) -> Vec<Arc<AppState>> {
        self.runtimes
            .lock()
            .unwrap_or_else(|v| v.into_inner())
            .values()
            .cloned()
            .collect()
    }

    fn ensure_runtimes(&self) -> Result<Vec<Arc<AppState>>, String> {
        let mut runtimes = self.runtimes.lock().unwrap_or_else(|v| v.into_inner());
        let mut added = Vec::new();
        for config in self.store.configs() {
            if runtimes.contains_key(&config.id) {
                continue;
            }
            let account = ManagedAccount::new(config.id.clone(), self.store.clone());
            let directory = self.store.account_directory(&config.id);
            self.migrate_runtime(&account, &directory)?;
            let mut child = AppState::new(
                UsageClient::new().map_err(|_| "network")?,
                self.root.stored_settings().clone(),
                directory.join("settings.json"),
            )
            .map_err(quota_auto_continue_error_key)?;
            child.settings_path = self.root.settings_path.clone();
            child.history_path = self.root.history_path.clone();
            child.stored_settings = self.root.stored_settings.clone();
            child.usage_history = self.root.usage_history.clone();
            child.update_check_sender = self.root.update_check_sender.clone();
            child.quota_auto_continue.bind_account(account.clone());
            child.account = Some(account);
            let child = Arc::new(child);
            runtimes.insert(config.id.clone(), child.clone());
            added.push(child);
        }
        Ok(added)
    }

    fn migrate_runtime(&self, account: &ManagedAccount, directory: &Path) -> Result<(), String> {
        // 一个凭证缺失不能阻止其他账号和设置窗口启动；该账号稍后显示认证错误。
        let Ok(credentials) = account.identity() else {
            return Ok(());
        };
        let identity = credentials
            .account_id
            .as_deref()
            .map(AccountIdentity::AccountId)
            .unwrap_or_else(|| AccountIdentity::Token(&credentials.access_token));
        for name in [
            QUOTA_AUTO_CONTINUE_STATE_FILE_NAME,
            RESET_CREDIT_NOTIFICATION_STATE_FILE_NAME,
        ] {
            let source = self.root.settings_path.with_file_name(name);
            let target = directory.join(name);
            if target.exists() || !source.exists() {
                continue;
            }
            let Ok(bytes) = fs::read(&source) else {
                continue;
            };
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            let matches = value
                .get("salt")
                .and_then(serde_json::Value::as_str)
                .and_then(|salt| usage_history::account_fingerprint(salt, identity).ok())
                .is_some_and(|fp| {
                    value
                        .get("accountFingerprint")
                        .and_then(serde_json::Value::as_str)
                        == Some(&fp)
                });
            if matches {
                accounts::atomic_write(&target, &bytes)?;
                if name == QUOTA_AUTO_CONTINUE_STATE_FILE_NAME {
                    let mut config = account.config().ok_or("accountMissing")?;
                    config.auto_continue = self.root.current_settings().quota_auto_continue_enabled;
                    self.store.update(config)?;
                }
            }
        }
        Ok(())
    }

    pub fn resolve(&self, id: Option<&str>) -> Result<Arc<AppState>, String> {
        let id = id.map(str::to_owned).or_else(|| self.store.selected_id());
        match id {
            Some(id) => {
                if self.store.config(&id).is_none() {
                    return Err("accountMissing".into());
                }
                self.runtimes
                    .lock()
                    .unwrap_or_else(|v| v.into_inner())
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| "accountMissing".into())
            }
            None => Ok(self.root.clone()),
        }
    }

    async fn response(&self, settings_window: bool) -> AccountsResponse {
        let mut rows = Vec::new();
        for config in self.store.configs() {
            if let Ok(runtime) = self.resolve(Some(&config.id)) {
                if let Some(account) = runtime.account.as_ref().and_then(|a| a.view()) {
                    let details = settings_window.then(|| {
                        let path = self.store.auth_path(&config.id);
                        let doc = accounts::read_document(&path).unwrap_or(serde_json::Value::Null);
                        let cached = runtime
                            .account_profile
                            .lock()
                            .unwrap_or_else(|v| v.into_inner());
                        crate::account_profile::account_details(&doc, cached.as_ref(), &path)
                    });
                    rows.push(AccountRow {
                        account,
                        dashboard: runtime.current_snapshot().await,
                        details,
                    });
                }
            }
        }
        AccountsResponse {
            selected_id: self.store.selected_id(),
            accounts: rows,
            codex_login: settings_window.then(|| self.store.inspect_codex_login()),
        }
    }
}

pub(crate) fn resolve(
    app: &AppHandle,
    root: &Arc<AppState>,
    id: Option<&str>,
) -> Result<Arc<AppState>, String> {
    match app.try_state::<Arc<AccountHub>>() {
        Some(hub) => hub.resolve(id),
        None => Ok(root.clone()),
    }
}

pub(crate) fn is_selected(app: &AppHandle, state: &AppState) -> bool {
    let id = state.account.as_ref().map(|a| a.id.clone());
    app.try_state::<Arc<AccountHub>>()
        .is_none_or(|hub| hub.store.selected_id() == id)
}

pub(crate) fn emit_accounts_changed(app: &AppHandle) {
    let _ = app.emit("accounts-updated", ());
}

pub(crate) async fn publish_selection(app: &AppHandle, hub: &AccountHub) -> Result<(), String> {
    let state = hub.resolve(None)?;
    let snapshot = state.current_snapshot().await;
    state.apply_auto_main_height(app, &snapshot);
    emit_accounts_changed(app);
    emit_dashboard(app, snapshot);
    emit_usage_history_updated(app, state.account.as_ref().map(|a| a.id.clone()));
    emit_quota_auto_continue_status(app, &state);
    Ok(())
}

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

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AnalyticsRequest {
    #[serde(rename_all = "camelCase")]
    Range {
        window_id: String,
        start_at: DateTime<Utc>,
        end_at: DateTime<Utc>,
    },
    #[serde(rename_all = "camelCase")]
    Cycles {
        window_id: String,
        request: UsageHistoryRequest,
    },
    #[serde(rename_all = "camelCase")]
    Samples {
        window_id: String,
        cycle_id: String,
        offset: usize,
    },
}

#[tauri::command]
pub async fn get_reset_credit_details(
    account_id: String,
    force: Option<bool>,
    window: WebviewWindow,
    hub: State<'_, Arc<AccountHub>>,
) -> Result<crate::reset_credit_details::ResetCreditDetails, String> {
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
    let mut history = state.usage_history.lock().await;
    state.select_history_account(&mut history)?;
    let now = Utc::now();
    let value = match request {
        AnalyticsRequest::Range {
            window_id,
            start_at,
            end_at,
        } => serde_json::to_value(
            history
                .history
                .range_summary(&window_id, start_at, end_at, now)?,
        ),
        AnalyticsRequest::Cycles { window_id, request } => {
            serde_json::to_value(history.history.cycle_summaries(&window_id, request, now)?)
        }
        AnalyticsRequest::Samples {
            window_id,
            cycle_id,
            offset,
        } => serde_json::to_value(
            history
                .history
                .cycle_samples(&window_id, &cycle_id, offset, now)?,
        ),
    };
    value.map_err(|_| "invalidResponse".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn main_window_account_response_excludes_profile_and_login_paths() {
        let root = std::env::temp_dir().join(format!(
            "codex-account-view-check-{}",
            rand::random::<u64>()
        ));
        fs::create_dir_all(&root).unwrap();
        let state = Arc::new(
            AppState::new(
                UsageClient::new().unwrap(),
                StoredSettings::default(),
                root.join("settings.json"),
            )
            .unwrap(),
        );
        let store = AccountStore::open(root.join("accounts")).unwrap();
        let source = root.join("source.json");
        accounts::atomic_write(
            &source,
            br#"{"tokens":{"account_id":"test-identity","access_token":"test-secret"}}"#,
        )
        .unwrap();
        let id = store.import_with_binding(&source, None).unwrap();
        let hub = AccountHub {
            store,
            root: state,
            runtimes: StdMutex::new(HashMap::new()),
        };
        hub.ensure_runtimes().unwrap();
        let runtime = hub.resolve(Some(&id)).unwrap();
        *runtime.account_profile.lock().unwrap() = Some(
            crate::account_profile::parse_usage_profile(
                &serde_json::json!({"email":"private@example.com","user_id":"raw-user","account_id":"test-identity"}),
                Utc::now(),
            ),
        );
        let json = serde_json::to_string(&hub.response(false).await).unwrap();
        for value in [
            "private@example.com",
            "raw-user",
            "test-identity",
            "test-secret",
            "managedAuthPath",
            "codexLogin",
            "details",
        ] {
            assert!(!json.contains(value), "private field escaped: {value}");
        }
        assert!(root.starts_with(std::env::temp_dir()));
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("codex-account-view-check-"));
        fs::remove_dir_all(root).unwrap();
    }
}
