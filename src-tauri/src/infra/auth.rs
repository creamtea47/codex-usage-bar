use serde_json::Value;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct AuthCredentials {
    pub access_token: String,
    pub account_id: Option<String>,
}

impl std::fmt::Debug for AuthCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthCredentials([redacted])")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("认证服务网络请求失败，请稍后重试。")]
    Network,
    #[error("认证服务请求频率受限。")]
    RateLimited,
    #[error("认证服务暂时不可用。")]
    ServiceUnavailable,
    #[error("无法保存刷新后的凭证。")]
    Persistence,
    #[error("未找到 Codex 登录信息，请先在 Codex 中登录。")]
    MissingFile,
    #[error("无法读取 Codex 登录信息，请重新登录 Codex。")]
    Unreadable,
    #[error("Codex 登录信息格式无效，请重新登录 Codex。")]
    InvalidJson,
    #[error("Codex 登录信息缺少访问凭据，请重新登录 Codex。")]
    MissingAccessToken,
}

/// 保持与旧版一致的优先级，但只读取第一个实际存在的认证文件。
pub fn resolve_auth_json_path() -> Result<PathBuf, AuthError> {
    let executable_dir = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let codex_home = env::var_os("CODEX_HOME").map(PathBuf::from);
    // Windows 使用 USERPROFILE，macOS 使用 HOME；优先级其余部分与旧版保持一致。
    let user_home = resolve_user_home();
    resolve_auth_json_path_from(
        executable_dir.as_deref(),
        codex_home.as_deref(),
        user_home.as_deref(),
    )
}

fn resolve_user_home() -> Option<PathBuf> {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
}

pub fn resolve_auth_json_path_from(
    executable_dir: Option<&Path>,
    codex_home: Option<&Path>,
    user_home: Option<&Path>,
) -> Result<PathBuf, AuthError> {
    let mut candidates = Vec::new();
    if let Some(directory) = executable_dir {
        candidates.push(directory.join("auth.json"));
    }
    if let Some(directory) = codex_home {
        candidates.push(directory.join("auth.json"));
    }
    if let Some(directory) = user_home {
        candidates.push(directory.join(".codex").join("auth.json"));
    }

    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or(AuthError::MissingFile)
}

/// 本读取入口不写文件也不向 IPC 返回认证材料；托管刷新和显式切换由 accounts 处理。
pub fn read_auth_credentials() -> Result<AuthCredentials, AuthError> {
    let auth_path = resolve_auth_json_path()?;
    read_auth_credentials_at(&auth_path)
}

/// 显式路径由 Rust 账号管理器提供，不能从展示账号的可变全局选择推断。
pub fn read_auth_credentials_at(auth_path: &Path) -> Result<AuthCredentials, AuthError> {
    let contents = fs::read_to_string(auth_path).map_err(|_| AuthError::Unreadable)?;
    let root: Value = serde_json::from_str(&contents).map_err(|_| AuthError::InvalidJson)?;
    let tokens = root.get("tokens").unwrap_or(&Value::Null);

    let access_token = tokens
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(AuthError::MissingAccessToken)?
        .to_owned();
    let account_id = tokens
        .get("account_id")
        .or_else(|| root.get("account_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    Ok(AuthCredentials {
        access_token,
        account_id,
    })
}

#[cfg(test)]
mod tests;
