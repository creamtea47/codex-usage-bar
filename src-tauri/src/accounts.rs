//! 凭证只在 Rust 中流转；展示选择、后台监测和 Codex 文件绑定是三个独立状态。
use crate::auth::{AuthCredentials, AuthError};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::sync::Mutex as AsyncMutex;

const MAX_AUTH_BYTES: u64 = 1024 * 1024;
const TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountConfig {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    #[serde(default)]
    pub auto_continue: bool,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Registry {
    accounts: Vec<AccountConfig>,
    selected_id: Option<String>,
    #[serde(default)]
    bound_id: Option<String>,
    #[serde(default)]
    codex_path: Option<PathBuf>,
}

pub struct AccountStore {
    pub directory: PathBuf,
    registry: Arc<Mutex<Registry>>,
    // ponytail: token 刷新暂用全局屏障；大量账号需要并行轮换时再拆为每账号锁加绑定屏障。
    /// 导入、绑定、删除与刷新共用屏障，避免轮换结果覆盖用户刚更新的凭证。
    pub operation: Arc<AsyncMutex<()>>,
}

pub struct ManagedAccount {
    pub id: String,
    pub store: Arc<AccountStore>,
    pub refresh_guard: AsyncMutex<()>,
    status: Mutex<String>,
    #[cfg(test)]
    token_endpoint: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub auto_continue: bool,
    pub model: Option<String>,
    pub bound_to_codex: bool,
    pub auth_status: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub can_refresh: bool,
}

fn locked<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn read_document(path: &Path) -> Result<Value, String> {
    if fs::metadata(path).map_err(|_| "authUnreadable")?.len() > MAX_AUTH_BYTES {
        return Err("authInvalid".into());
    }
    let bytes = fs::read(path).map_err(|_| "authUnreadable")?;
    if bytes.len() > MAX_AUTH_BYTES as usize {
        return Err("authInvalid".into());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "authInvalid")?;
    credentials(&value)?;
    Ok(value)
}

pub(crate) fn token_claims(token: Option<&str>) -> Value {
    token
        .and_then(|v| v.split('.').nth(1))
        .and_then(|v| URL_SAFE_NO_PAD.decode(v).ok())
        .and_then(|v| serde_json::from_slice(&v).ok())
        .unwrap_or(Value::Null)
}

pub fn credentials(doc: &Value) -> Result<AuthCredentials, String> {
    let access = doc
        .pointer("/tokens/access_token")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or("authInvalid")?;
    let claims = token_claims(Some(access));
    let explicit_id = doc
        .pointer("/tokens/account_id")
        .or_else(|| doc.get("account_id"))
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty());
    let claimed_id = claims
        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
        .and_then(Value::as_str);
    if explicit_id.zip(claimed_id).is_some_and(|(a, b)| a != b) {
        return Err("accountChanged".into());
    }
    let account_id = explicit_id.or(claimed_id).map(str::to_owned);
    Ok(AuthCredentials {
        access_token: access.to_owned(),
        account_id,
    })
}

/// 不以 access_token 作账号主键，轮换不能生成新分区。缺少稳定身份的文件拒绝托管。
pub fn document_id(doc: &Value) -> Result<String, String> {
    let credentials = credentials(doc)?;
    let identity = credentials
        .account_id
        .as_deref()
        .ok_or("accountIdentityMissing")?;
    Ok(hex::encode(Sha256::digest(format!(
        "codex-usage-bar:account:{identity}"
    ))))
}

fn expires_at(doc: &Value) -> Option<DateTime<Utc>> {
    let claims = token_claims(doc.pointer("/tokens/access_token").and_then(Value::as_str));
    claims
        .get("exp")
        .and_then(Value::as_i64)
        .and_then(|v| DateTime::from_timestamp(v, 0))
        .or_else(|| {
            doc.get("expires_at")
                .and_then(Value::as_i64)
                .and_then(|v| DateTime::from_timestamp(v, 0))
        })
}

fn display_label(doc: &Value, id: &str) -> String {
    let claims = token_claims(doc.pointer("/tokens/id_token").and_then(Value::as_str));
    if let Some(email) = claims.get("email").and_then(Value::as_str) {
        if let Some((name, host)) = email.split_once('@') {
            return format!("{}***@{}", name.chars().next().unwrap_or('*'), host);
        }
    }
    format!("Account {}", &id[..8])
}

/// 临时文件与目标同目录，先刷盘再替换；写失败保留原件，不使用先删后写。
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("persistence")?;
    fs::create_dir_all(parent).map_err(|_| "persistence")?;
    let temporary = parent.join(format!(".account-{:016x}.tmp", rand::random::<u64>()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| "persistence")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| "persistence")?;
        }
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "persistence")?;
        drop(file);
        fs::rename(&temporary, path).map_err(|_| "persistence")
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(str::to_owned)
}

fn save_doc(path: &Path, doc: &Value) -> Result<(), String> {
    atomic_write(
        path,
        &serde_json::to_vec_pretty(doc).map_err(|_| "persistence")?,
    )
}

fn protect_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|_| "persistence")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|_| "persistence")?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 使用 SID，不依赖本地化用户名；不将命令结果（可能含用户信息）写入日志。
        let who = std::process::Command::new("whoami.exe")
            .args(["/user", "/fo", "csv", "/nh"])
            .creation_flags(0x08000000)
            .output()
            .map_err(|_| "permissions")?;
        if !who.status.success() {
            return Err("permissions".into());
        }
        let raw = String::from_utf8_lossy(&who.stdout);
        let sid = raw
            .split(',')
            .next_back()
            .unwrap_or("")
            .trim()
            .trim_matches('"');
        if !sid.starts_with("S-1-")
            || !sid
                .chars()
                .all(|c| c.is_ascii_digit() || c == '-' || c == 'S')
        {
            return Err("permissions".into());
        }
        let grant = format!("*{sid}:(OI)(CI)F");
        let output = std::process::Command::new("icacls.exe")
            .arg(path)
            .args(["/inheritance:r", "/grant:r", &grant, "/q"])
            .creation_flags(0x08000000)
            .output()
            .map_err(|_| "permissions")?;
        if !output.status.success() {
            return Err("permissions".into());
        }
    }
    Ok(())
}

impl AccountStore {
    pub fn open(directory: PathBuf) -> Result<Arc<Self>, String> {
        protect_directory(&directory)?;
        let path = directory.join("accounts.json");
        let registry = if path.exists() {
            serde_json::from_slice::<Registry>(&fs::read(&path).map_err(|_| "persistence")?)
                .map_err(|_| "accountsCorrupt")?
        } else {
            Registry::default()
        };
        // 元数据也是不可信的磁盘输入，账号 ID 永远不能成为目录穿越入口。
        if registry
            .accounts
            .iter()
            .map(|a| &a.id)
            .chain(registry.selected_id.iter())
            .chain(registry.bound_id.iter())
            .any(|id| id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("accountsCorrupt".into());
        }
        Ok(Arc::new(Self {
            directory,
            registry: Arc::new(Mutex::new(registry)),
            operation: Arc::new(AsyncMutex::new(())),
        }))
    }

    fn commit(&self, candidate: Registry) -> Result<(), String> {
        atomic_write(
            &self.directory.join("accounts.json"),
            &serde_json::to_vec_pretty(&candidate).map_err(|_| "persistence")?,
        )?;
        *locked(&self.registry) = candidate;
        Ok(())
    }
    pub fn configs(&self) -> Vec<AccountConfig> {
        locked(&self.registry).accounts.clone()
    }
    pub fn selected_id(&self) -> Option<String> {
        locked(&self.registry).selected_id.clone()
    }
    pub fn config(&self, id: &str) -> Option<AccountConfig> {
        locked(&self.registry)
            .accounts
            .iter()
            .find(|v| v.id == id)
            .cloned()
    }
    pub fn account_directory(&self, id: &str) -> PathBuf {
        self.directory.join(id)
    }
    pub fn auth_path(&self, id: &str) -> PathBuf {
        self.account_directory(id).join("auth.json")
    }
    pub fn bound_id(&self) -> Option<String> {
        locked(&self.registry).bound_id.clone()
    }

    pub fn select(&self, id: &str) -> Result<(), String> {
        if self.config(id).is_none() {
            return Err("accountMissing".into());
        }
        let mut next = locked(&self.registry).clone();
        next.selected_id = Some(id.to_owned());
        self.commit(next)
    }

    pub fn update(&self, config: AccountConfig) -> Result<(), String> {
        if config.label.trim().is_empty() || config.label.chars().count() > 120 {
            return Err("labelInvalid".into());
        }
        if config.model.as_deref().is_some_and(|m| {
            m.is_empty()
                || m.len() > 128
                || !m
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-._:/".contains(&c))
        }) {
            return Err("modelInvalid".into());
        }
        let mut next = locked(&self.registry).clone();
        let item = next
            .accounts
            .iter_mut()
            .find(|v| v.id == config.id)
            .ok_or("accountMissing")?;
        *item = config.clone();
        self.commit(next)?;
        log::info!(
            "Account settings saved: account={}, monitoring={}, continuation={}",
            &config.id[..8],
            config.enabled,
            config.auto_continue
        );
        Ok(())
    }

    pub fn import(&self, path: &Path) -> Result<String, String> {
        self.import_with_binding(path, codex_auth_path().ok())
    }

    pub(crate) fn import_with_binding(
        &self,
        path: &Path,
        binding: Option<PathBuf>,
    ) -> Result<String, String> {
        let doc = read_document(path)?;
        let id = document_id(&doc)?;
        let directory = self.account_directory(&id);
        fs::create_dir_all(&directory).map_err(|_| "persistence")?;
        let auth_path = self.auth_path(&id);
        let previous = if auth_path.exists() {
            Some(fs::read(&auth_path).map_err(|_| "persistence")?)
        } else {
            None
        };
        save_doc(&auth_path, &doc)?;
        let mut next = locked(&self.registry).clone();
        if !next.accounts.iter().any(|v| v.id == id) {
            next.accounts.push(AccountConfig {
                id: id.clone(),
                label: display_label(&doc, &id),
                enabled: true,
                auto_continue: false,
                model: None,
            });
        }
        if next.selected_id.is_none() {
            next.selected_id = Some(id.clone());
        }
        // 原文件恰为当前 Codex 文件时归属 Codex，不能从副本并发轮换同一 refresh_token。
        if let Some(codex_path) = binding {
            if read_document(&codex_path)
                .and_then(|v| document_id(&v))
                .ok()
                .as_deref()
                == Some(&id)
            {
                next.bound_id = Some(id.clone());
                next.codex_path = Some(codex_path);
            }
        }
        if let Err(error) = self.commit(next) {
            if let Some(previous) = previous {
                atomic_write(&auth_path, &previous)?;
            } else {
                fs::remove_file(&auth_path).map_err(|_| "persistence")?;
            }
            return Err(error);
        }
        log::info!("account imported: account={}", &id[..8]);
        Ok(id)
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        let mut next = locked(&self.registry).clone();
        if !next.accounts.iter().any(|a| a.id == id) {
            return Err("accountMissing".into());
        }
        next.accounts.retain(|v| v.id != id);
        if next.selected_id.as_deref() == Some(id) {
            next.selected_id = next.accounts.first().map(|v| v.id.clone());
        }
        // 移除只影响托管副本；Codex 原件仍由 Codex 管理，不替用户退出登录。
        if next.bound_id.as_deref() == Some(id) {
            next.bound_id = None;
            next.codex_path = None;
        }
        let path = self.auth_path(id);
        let previous = fs::read(&path).map_err(|_| "persistence")?;
        fs::remove_file(&path).map_err(|_| "persistence")?;
        if let Err(error) = self.commit(next) {
            atomic_write(&path, &previous)?;
            return Err(error);
        }
        log::info!("account credentials removed: account={}", &id[..8]);
        Ok(())
    }

    /// 绑定期间官方文件是唯一凭证来源；身份变化后禁止将新账号凭证写进旧账号。
    pub fn sync_bound(&self) -> Result<(), String> {
        let registry = locked(&self.registry).clone();
        if let (Some(id), Some(path)) = (registry.bound_id, registry.codex_path) {
            let doc = read_document(&path)?;
            if document_id(&doc)? != id {
                return Err("accountChanged".into());
            }
            save_doc(&self.auth_path(&id), &doc)?;
        }
        Ok(())
    }

    pub fn apply_to_codex(&self, id: &str) -> Result<(), String> {
        ensure_codex_stopped()?;
        let path = codex_auth_path()?;
        self.apply_at(id, &path, ensure_codex_stopped)
    }

    /// 替换前后均验证；可注入只读进程检查以测试失败回滚，不接触真实 Codex 文件。
    fn apply_at(
        &self,
        id: &str,
        path: &Path,
        stopped: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        stopped()?;
        if self.config(id).is_none() {
            return Err("accountMissing".into());
        }
        self.sync_bound()?;
        let doc = read_document(&self.auth_path(id))?;
        let previous = if path.exists() {
            Some(fs::read(path).map_err(|_| "authUnreadable")?)
        } else {
            None
        };
        if let Some(bytes) = previous.as_ref() {
            atomic_write(&self.directory.join("codex-auth.backup.json"), bytes)?;
        }
        stopped()?;
        save_doc(path, &doc)?;
        let mut next = locked(&self.registry).clone();
        next.bound_id = Some(id.into());
        next.codex_path = Some(path.to_path_buf());
        let result = read_document(path).and_then(|v| {
            if document_id(&v)? != id {
                Err("accountChanged".into())
            } else {
                self.commit(next)
            }
        });
        if result.is_err() {
            if let Some(bytes) = previous {
                atomic_write(path, &bytes)?;
            } else {
                fs::remove_file(path).map_err(|_| "persistence")?;
            }
        }
        log::info!(
            "Codex credential application: account={}, success={}",
            &id[..8],
            result.is_ok()
        );
        result
    }

    pub fn restore_codex(&self) -> Result<(), String> {
        ensure_codex_stopped()?;
        let path = codex_auth_path()?;
        self.restore_at(&path, ensure_codex_stopped)
    }

    fn restore_at(
        &self,
        path: &Path,
        stopped: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        stopped()?;
        self.sync_bound()?;
        let backup = self.directory.join("codex-auth.backup.json");
        // 原件可能是 API-key 登录文件；恢复必须支持该原始格式，而非强制 OAuth。
        let bytes = fs::read(&backup).map_err(|_| "authUnreadable")?;
        if bytes.len() > MAX_AUTH_BYTES as usize {
            return Err("authInvalid".into());
        }
        let mut doc: Value = serde_json::from_slice(&bytes).map_err(|_| "authInvalid")?;
        if !doc.is_object() {
            return Err("authInvalid".into());
        }
        let id = document_id(&doc).ok();
        // 已切走的账号可能在后台轮换，恢复时使用它的最新令牌，不能复活已作废的备份令牌。
        if let Some(id) = id.as_deref().filter(|id| self.config(id).is_some()) {
            let latest = read_document(&self.auth_path(id))?;
            for key in ["tokens", "last_refresh", "expires_at"] {
                if let Some(value) = latest.get(key) {
                    doc[key] = value.clone();
                }
            }
        }
        let old = fs::read(path).map_err(|_| "authUnreadable")?;
        stopped()?;
        save_doc(path, &doc)?;
        let mut next = locked(&self.registry).clone();
        next.bound_id = next
            .accounts
            .iter()
            .find(|v| Some(&v.id) == id.as_ref())
            .map(|v| v.id.clone());
        next.codex_path = Some(path.to_path_buf());
        let result = fs::read(path)
            .map_err(|_| "persistence".to_owned())
            .and_then(|bytes| {
                serde_json::from_slice::<Value>(&bytes).map_err(|_| "authInvalid".into())
            })
            .and_then(|saved| {
                if saved == doc {
                    self.commit(next)
                } else {
                    Err("persistence".into())
                }
            });
        if let Err(error) = result {
            atomic_write(path, &old)?;
            return Err(error);
        }
        log::info!("Codex credential backup restored");
        Ok(())
    }
}

impl ManagedAccount {
    pub fn new(id: String, store: Arc<AccountStore>) -> Arc<Self> {
        Arc::new(Self {
            id,
            store,
            refresh_guard: AsyncMutex::new(()),
            status: Mutex::new("ready".into()),
            #[cfg(test)]
            token_endpoint: None,
        })
    }
    pub fn config(&self) -> Option<AccountConfig> {
        self.store.config(&self.id)
    }
    pub fn view(&self) -> Option<AccountView> {
        let c = self.config()?;
        let doc = read_document(&self.store.auth_path(&self.id)).ok();
        let bound = self.store.bound_id().as_deref() == Some(&self.id);
        let auth_status = if doc.is_none() {
            "authUnreadable".into()
        } else if doc.as_ref().and_then(|v| document_id(v).ok()).as_deref() != Some(&self.id) {
            "accountChanged".into()
        } else if doc
            .as_ref()
            .and_then(expires_at)
            .is_some_and(|v| v <= Utc::now())
            && bound
        {
            "codexRefreshRequired".into()
        } else {
            locked(&self.status).clone()
        };
        Some(AccountView {
            id: c.id,
            label: c.label,
            enabled: c.enabled,
            auto_continue: c.auto_continue,
            model: c.model,
            bound_to_codex: bound,
            auth_status,
            expires_at: doc.as_ref().and_then(expires_at),
            can_refresh: doc.as_ref().is_some_and(|v| {
                v.pointer("/tokens/refresh_token")
                    .and_then(Value::as_str)
                    .is_some_and(|v| !v.is_empty())
            }),
        })
    }
    pub fn identity(&self) -> Result<AuthCredentials, String> {
        let doc = read_document(&self.store.auth_path(&self.id))?;
        if document_id(&doc)? != self.id {
            return Err("accountChanged".into());
        }
        credentials(&doc)
    }

    pub async fn credentials(&self, force: bool) -> Result<AuthCredentials, AuthError> {
        let _guard = self.refresh_guard.lock().await;
        let _operation = self.store.operation.lock().await;
        let result = self.acquire(force).await;
        *locked(&self.status) = match &result {
            Ok(_) => "ready",
            Err(code) => code,
        }
        .to_owned();
        result.map_err(|code| match code.as_str() {
            "network" => AuthError::Network,
            "persistence" => AuthError::Persistence,
            _ => AuthError::Unreadable,
        })
    }

    async fn acquire(&self, force: bool) -> Result<AuthCredentials, String> {
        if self.config().is_none() {
            return Err("accountMissing".into());
        }
        if self.store.bound_id().as_deref() == Some(&self.id) {
            self.store.sync_bound()?;
            let doc = read_document(&self.store.auth_path(&self.id))?;
            if expires_at(&doc).is_some_and(|v| v <= Utc::now()) {
                return Err("codexRefreshRequired".into());
            }
            return credentials(&doc);
        }
        let path = self.store.auth_path(&self.id);
        let mut doc = read_document(&path)?;
        if document_id(&doc)? != self.id {
            return Err("accountChanged".into());
        }
        if !force && expires_at(&doc).is_none_or(|v| v > Utc::now() + Duration::minutes(30)) {
            return credentials(&doc);
        }
        let token = doc
            .pointer("/tokens/refresh_token")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty());
        let Some(token) = token.map(str::to_owned) else {
            // 没有 refresh_token 的导入文件仍可用到 access_token 真正到期。
            if !force && expires_at(&doc).is_some_and(|v| v > Utc::now()) {
                return credentials(&doc);
            }
            return Err("refreshTokenMissing".into());
        };
        let client_id = doc
            .get("client_id")
            .and_then(Value::as_str)
            .unwrap_or(CLIENT_ID)
            .to_owned();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|_| "network")?;
        let endpoint = TOKEN_ENDPOINT;
        #[cfg(test)]
        let endpoint = self.token_endpoint.as_deref().unwrap_or(endpoint);
        let mut response = client
            .post(endpoint)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", &token),
                ("client_id", &client_id),
            ])
            .send()
            .await
            .map_err(|_| "network")?;
        if !response.status().is_success() {
            return Err(if response.status().is_client_error() {
                "authenticationRequired"
            } else {
                "network"
            }
            .into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "network")? {
            if bytes.len() + chunk.len() > MAX_AUTH_BYTES as usize {
                return Err("authInvalid".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let refreshed: Value = serde_json::from_slice(&bytes).map_err(|_| "authInvalid")?;
        merge_tokens(&mut doc, &refreshed)?;
        if document_id(&doc)? != self.id {
            return Err("accountChanged".into());
        }
        save_doc(&path, &doc)?;
        log::info!("OAuth refresh persisted: account={}", &self.id[..8]);
        credentials(&doc)
    }
}

fn merge_tokens(doc: &mut Value, response: &Value) -> Result<(), String> {
    if response
        .get("access_token")
        .and_then(Value::as_str)
        .is_none_or(|v| v.is_empty())
    {
        return Err("authInvalid".into());
    }
    for key in ["access_token", "refresh_token", "id_token"] {
        if let Some(token) = response
            .get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        {
            doc["tokens"][key] = Value::String(token.into());
        }
    }
    doc["last_refresh"] = Value::String(Utc::now().to_rfc3339());
    if let Some(seconds) = response.get("expires_in").and_then(Value::as_i64) {
        doc["expires_at"] = (Utc::now().timestamp() + seconds.clamp(0, 365 * 86400)).into();
    }
    Ok(())
}

fn codex_auth_path() -> Result<PathBuf, String> {
    let (path, storage) = codex_auth_location()?;
    if storage != "file" {
        return Err("authStoreUnsupported".into());
    }
    Ok(path)
}

/// 登录来源展示与原有切换使用同一解析入口；返回路径不意味着运行中进程已加载该文件。
pub(crate) fn codex_auth_location() -> Result<(PathBuf, String), String> {
    let directory = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(|v| PathBuf::from(v).join(".codex"))
        })
        .ok_or("authUnreadable")?;
    let config_path = directory.join("config.toml");
    let storage = if config_path.exists() {
        let source = fs::read_to_string(config_path).map_err(|_| "authUnreadable")?;
        let config: toml::Value = toml::from_str(&source).map_err(|_| "authStoreUnsupported")?;
        config
            .get("cli_auth_credentials_store")
            .and_then(toml::Value::as_str)
            .unwrap_or("file")
            .to_owned()
    } else {
        "file".to_owned()
    };
    Ok((directory.join("auth.json"), storage))
}

fn ensure_codex_stopped() -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let output = std::process::Command::new("tasklist.exe")
            .args(["/FO", "CSV", "/NH"])
            .creation_flags(0x08000000)
            .output()
            .map_err(|_| "processCheckFailed")?;
        if !output.status.success() {
            return Err("processCheckFailed".into());
        }
        let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
        if text.lines().any(|v| {
            let name = v.split(',').next().unwrap_or("").trim_matches('"');
            matches!(name, "codex.exe" | "chatgpt.exe")
        }) {
            return Err("codexRunning".into());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err("switchUnsupported".into())
    }
}

#[cfg(test)]
pub(crate) fn account_for_protocol_test(endpoint: String, model: String) -> Arc<ManagedAccount> {
    let directory =
        std::env::temp_dir().join(format!("codex-account-check-{}", rand::random::<u64>()));
    fs::create_dir_all(&directory).unwrap();
    let store = Arc::new(AccountStore {
        directory,
        registry: Arc::new(Mutex::new(Registry::default())),
        operation: Arc::new(AsyncMutex::new(())),
    });
    let source = store.directory.join("source.json");
    save_doc(&source,&serde_json::json!({"tokens":{"account_id":"test-account-id","access_token":"test-access-token","refresh_token":"fake-refresh"},"expires_at":Utc::now().timestamp()+7200})).unwrap();
    let id = store.import_with_binding(&source, None).unwrap();
    let mut config = store.config(&id).unwrap();
    config.model = Some(model);
    store.update(config).unwrap();
    let mut account = ManagedAccount::new(id, store);
    Arc::get_mut(&mut account).unwrap().token_endpoint = Some(endpoint);
    account
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated_store() -> Arc<AccountStore> {
        let directory =
            std::env::temp_dir().join(format!("codex-account-check-{}", rand::random::<u64>()));
        fs::create_dir_all(&directory).unwrap();
        Arc::new(AccountStore {
            directory,
            registry: Arc::new(Mutex::new(Registry::default())),
            operation: Arc::new(AsyncMutex::new(())),
        })
    }

    fn auth_document(access: &str) -> Value {
        serde_json::json!({"tokens":{"account_id":"isolated-account","access_token":access,"refresh_token":"fake-refresh-secret"},"expires_at":Utc::now().timestamp()+60})
    }

    #[test]
    fn codex_switch_is_guarded_preserves_provider_and_rolls_back_registry_failure() {
        let store = isolated_store();
        let source = store.directory.join("source.json");
        save_doc(&source, &auth_document("test-secret")).unwrap();
        let id = store.import_with_binding(&source, None).unwrap();
        let codex = store.directory.join("codex");
        fs::create_dir(&codex).unwrap();
        let target = codex.join("auth.json");
        let original = br#"{"OPENAI_API_KEY":"fake-key","auth_mode":"apikey"}"#;
        atomic_write(&target, original).unwrap();
        let config = codex.join("config.toml");
        atomic_write(&config, b"model_provider = 'custom'\n").unwrap();
        assert_eq!(
            store
                .apply_at(&id, &target, || Err("codexRunning".into()))
                .unwrap_err(),
            "codexRunning"
        );
        assert_eq!(fs::read(&target).unwrap(), original);
        store.apply_at(&id, &target, || Ok(())).unwrap();
        assert_eq!(document_id(&read_document(&target).unwrap()).unwrap(), id);
        assert_eq!(fs::read(&config).unwrap(), b"model_provider = 'custom'\n");
        store.restore_at(&target, || Ok(())).unwrap();
        let restored: Value = serde_json::from_slice(&fs::read(&target).unwrap()).unwrap();
        assert_eq!(restored["OPENAI_API_KEY"], "fake-key");
        assert!(store.bound_id().is_none());
        fs::remove_file(store.directory.join("accounts.json")).unwrap();
        fs::create_dir(store.directory.join("accounts.json")).unwrap();
        let before = fs::read(&target).unwrap();
        assert!(store.apply_at(&id, &target, || Ok(())).is_err());
        assert_eq!(fs::read(&target).unwrap(), before);
        let absent = codex.join("absent.json");
        assert!(store.apply_at(&id, &absent, || Ok(())).is_err());
        assert!(!absent.exists());
        clean(&store);
    }

    fn clean(store: &AccountStore) {
        assert!(store.directory.starts_with(std::env::temp_dir()));
        assert!(store
            .directory
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("codex-account-check-"));
        fs::remove_dir_all(&store.directory).unwrap();
    }

    #[test]
    fn import_deduplicates_rolls_back_failed_save_and_removes_only_copy() {
        let store = isolated_store();
        let source = store.directory.join("source.json");
        save_doc(&source, &auth_document("first-secret")).unwrap();
        let source_bytes = fs::read(&source).unwrap();
        let id = store.import_with_binding(&source, None).unwrap();
        assert_eq!(fs::read(&source).unwrap(), source_bytes);
        save_doc(&source, &auth_document("second-secret")).unwrap();
        assert_eq!(store.import_with_binding(&source, None).unwrap(), id);
        assert_eq!(store.configs().len(), 1);
        assert!(!store.config(&id).unwrap().auto_continue);
        let previous = fs::read(store.auth_path(&id)).unwrap();
        fs::remove_file(store.directory.join("accounts.json")).unwrap();
        fs::create_dir(store.directory.join("accounts.json")).unwrap();
        save_doc(&source, &auth_document("third-secret")).unwrap();
        assert!(store.import_with_binding(&source, None).is_err());
        assert_eq!(fs::read(store.auth_path(&id)).unwrap(), previous);
        assert!(store.remove(&id).is_err());
        assert_eq!(fs::read(store.auth_path(&id)).unwrap(), previous);
        fs::remove_dir(store.directory.join("accounts.json")).unwrap();
        store.remove(&id).unwrap();
        assert!(source.exists());
        assert!(!store.auth_path(&id).exists());
        clean(&store);
    }

    #[tokio::test]
    async fn bound_account_follows_codex_and_rejects_external_identity_change() {
        let store = isolated_store();
        let source = store.directory.join("codex-auth.json");
        save_doc(&source, &auth_document("first-secret")).unwrap();
        let id = store
            .import_with_binding(&source, Some(source.clone()))
            .unwrap();
        let account = ManagedAccount::new(id.clone(), store.clone());
        save_doc(&source, &auth_document("new-secret")).unwrap();
        assert_eq!(
            account.credentials(false).await.unwrap().access_token,
            "new-secret"
        );
        let view = serde_json::to_string(&account.view()).unwrap();
        assert!(!view.contains("new-secret") && !view.contains("fake-refresh-secret"));
        let mut other = auth_document("other-secret");
        other["tokens"]["account_id"] = "other-account".into();
        save_doc(&source, &other).unwrap();
        assert!(account.credentials(false).await.is_err());
        assert_eq!(
            credentials(&read_document(&store.auth_path(&id)).unwrap())
                .unwrap()
                .access_token,
            "new-secret"
        );
        clean(&store);
    }

    #[tokio::test]
    async fn oauth_rotation_is_persisted_before_next_read_without_touching_source() {
        use std::io::Read;
        let store = isolated_store();
        let source = store.directory.join("source.json");
        save_doc(&source, &auth_document("old-secret")).unwrap();
        let id = store.import_with_binding(&source, None).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&request);
                if let Some((headers, body)) = text.split_once("\r\n\r\n") {
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|v| v.parse::<usize>().ok())
                        })
                        .unwrap();
                    if body.len() >= length {
                        assert!(body.contains("grant_type=refresh_token"));
                        break;
                    }
                }
            }
            let body = r#"{"access_token":"rotated-secret","refresh_token":"new-refresh-secret","expires_in":3600}"#;
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        let mut account = ManagedAccount::new(id.clone(), store.clone());
        Arc::get_mut(&mut account).unwrap().token_endpoint = Some(endpoint);
        let (one, two) = tokio::join!(account.credentials(false), account.credentials(false));
        assert_eq!(one.unwrap().access_token, "rotated-secret");
        assert_eq!(two.unwrap().access_token, "rotated-secret");
        server.join().unwrap();
        let saved = read_document(&store.auth_path(&id)).unwrap();
        assert_eq!(saved["tokens"]["refresh_token"], "new-refresh-secret");
        assert_eq!(document_id(&saved).unwrap(), id);
        assert_eq!(
            read_document(&source).unwrap()["tokens"]["access_token"],
            "old-secret"
        );
        clean(&store);
    }
    #[test]
    fn rotation_keeps_identity_and_missing_refresh_token() {
        let mut doc = serde_json::json!({"tokens":{"access_token":"old", "refresh_token":"keep", "account_id":"test-account"}});
        let id = document_id(&doc).unwrap();
        merge_tokens(&mut doc, &serde_json::json!({"access_token":"new"})).unwrap();
        assert_eq!(document_id(&doc).unwrap(), id);
        assert_eq!(doc["tokens"]["refresh_token"], "keep");
        assert!(
            document_id(&serde_json::json!({"tokens":{"access_token":"no-identity"}})).is_err()
        );
    }
    #[test]
    fn atomic_replacement_retains_valid_json() {
        let path =
            std::env::temp_dir().join(format!("usage-account-{}.json", rand::random::<u64>()));
        atomic_write(&path, b"{\"old\":true}").unwrap();
        atomic_write(&path, b"{\"new\":true}").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"{\"new\":true}");
        fs::remove_file(path).unwrap();
    }
}
