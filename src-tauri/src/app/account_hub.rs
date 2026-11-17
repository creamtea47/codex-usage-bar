use crate::app::continuation::STATE_FILE_NAME as QUOTA_AUTO_CONTINUE_STATE_FILE_NAME;
use crate::app::continuation_scheduler::quota_auto_continue_error_key;
use crate::app::continuation_scheduler::start_quota_auto_continue_loop;
use crate::app::refresh::refresh_and_emit;
use crate::app::refresh::start_refresh_loop;
use crate::app::state::AppState;
use crate::domain::history as usage_history;
use crate::domain::history::AccountIdentity;
use crate::domain::usage::DashboardSnapshot;
use crate::infra::accounts;
use crate::infra::auth;
use crate::infra::credit_baseline::STATE_FILE_NAME as RESET_CREDIT_NOTIFICATION_STATE_FILE_NAME;
use crate::infra::usage::UsageClient;
use crate::ipc::events::emit_dashboard;
use crate::ipc::events::emit_quota_auto_continue_status;
use crate::ipc::events::emit_usage_history_updated;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tauri::AppHandle;
use tauri::Emitter;
use tauri::Manager;

use crate::infra::accounts::AccountStore;
use crate::infra::accounts::AccountView;
use crate::infra::accounts::ManagedAccount;
use std::{fs, path::Path};

/// 复用现有单账号状态机；每个状态机独立锁定刷新、通知与接续，共享窗口设置和历史存储。
pub(crate) struct AccountHub {
    pub store: Arc<AccountStore>,
    pub(crate) root: Arc<AppState>,
    pub(crate) runtimes: StdMutex<HashMap<String, Arc<AppState>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRow {
    #[serde(flatten)]
    pub(crate) account: AccountView,
    pub(crate) dashboard: DashboardSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) details: Option<crate::infra::account_profile::AccountDetails>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountsResponse {
    pub(crate) selected_id: Option<String>,
    pub(crate) accounts: Vec<AccountRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) codex_login: Option<crate::infra::account_profile::CodexLoginSource>,
}

impl AccountHub {
    pub fn new(root: Arc<AppState>) -> Result<Arc<Self>, String> {
        let store = AccountStore::open(root.shared.settings_path.with_file_name("accounts"))?;
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

    pub async fn import_login(
        &self,
        app: &AppHandle,
        doc: &serde_json::Value,
    ) -> Result<(), String> {
        let guard = self.store.operation.lock().await;
        let id = self
            .store
            .import_document(doc, accounts::ImportSource::BrowserLogin)?;
        let added = self.ensure_runtimes()?;
        drop(guard);
        let already_running = !added
            .iter()
            .any(|s| s.account.as_ref().is_some_and(|a| a.id == id));
        for state in added {
            start_refresh_loop(app.clone(), state.clone());
            start_quota_auto_continue_loop(app.clone(), state);
        }
        if already_running {
            let state = self.resolve(Some(&id))?;
            refresh_and_emit(app, &state).await;
        }
        publish_selection(app, self).await
    }

    pub fn all(&self) -> Vec<Arc<AppState>> {
        self.runtimes
            .lock()
            .unwrap_or_else(|v| v.into_inner())
            .values()
            .cloned()
            .collect()
    }

    pub(crate) fn ensure_runtimes(&self) -> Result<Vec<Arc<AppState>>, String> {
        let mut runtimes = self.runtimes.lock().unwrap_or_else(|v| v.into_inner());
        let mut added = Vec::new();
        for config in self.store.configs() {
            if runtimes.contains_key(&config.id) {
                continue;
            }
            let account = ManagedAccount::new(config.id.clone(), self.store.clone());
            let directory = self.store.account_directory(&config.id);
            self.migrate_runtime(&account, &directory)?;
            let child = AppState::for_account(
                UsageClient::new().map_err(|_| "network")?,
                self.root.shared.clone(),
                &directory,
                Some(account),
            )
            .map_err(quota_auto_continue_error_key)?;
            let child = Arc::new(child);
            runtimes.insert(config.id.clone(), child.clone());
            added.push(child);
        }
        Ok(added)
    }

    pub(crate) fn migrate_runtime(
        &self,
        account: &ManagedAccount,
        directory: &Path,
    ) -> Result<(), String> {
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
            let source = self.root.shared.settings_path.with_file_name(name);
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

    pub(crate) async fn response(&self, settings_window: bool) -> AccountsResponse {
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
                        crate::infra::account_profile::account_details(&doc, cached.as_ref(), &path)
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

#[cfg(test)]
mod tests;
