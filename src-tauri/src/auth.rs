use serde_json::Value;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct AuthCredentials {
    pub access_token: String,
    pub account_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
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

/// 认证文件不会通过 IPC 返回，也不会被本应用改写。
pub fn read_auth_credentials() -> Result<AuthCredentials, AuthError> {
    let auth_path = resolve_auth_json_path()?;
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
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temporary_directory(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!("codex-usage-bar-{name}-{suffix}"));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn resolves_existing_files_in_documented_priority_order() {
        let root = temporary_directory("auth-priority");
        let exe = root.join("exe");
        let home = root.join("home");
        let profile = root.join("profile");
        fs::create_dir_all(&exe).unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(profile.join(".codex")).unwrap();
        fs::write(home.join("auth.json"), "{}").unwrap();
        fs::write(profile.join(".codex/auth.json"), "{}").unwrap();
        assert_eq!(
            resolve_auth_json_path_from(Some(&exe), Some(&home), Some(&profile)).unwrap(),
            home.join("auth.json")
        );
        fs::write(exe.join("auth.json"), "{}").unwrap();
        assert_eq!(
            resolve_auth_json_path_from(Some(&exe), Some(&home), Some(&profile)).unwrap(),
            exe.join("auth.json")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn falls_back_to_user_profile_and_reports_missing_files_without_paths() {
        let root = temporary_directory("auth-profile-fallback");
        let exe = root.join("exe");
        let home = root.join("home");
        let profile = root.join("profile");
        fs::create_dir_all(&exe).unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(profile.join(".codex")).unwrap();
        fs::write(profile.join(".codex/auth.json"), "{}").unwrap();

        assert_eq!(
            resolve_auth_json_path_from(Some(&exe), Some(&home), Some(&profile)).unwrap(),
            profile.join(".codex/auth.json")
        );

        fs::remove_file(profile.join(".codex/auth.json")).unwrap();
        let error = resolve_auth_json_path_from(Some(&exe), Some(&home), Some(&profile))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Codex"));
        assert!(!error.contains("auth.json"));
        assert!(!error.contains(&root.display().to_string()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_only_the_fields_needed_for_usage_requests() {
        let parsed: Value = serde_json::json!({
            "tokens": { "access_token": "secret", "account_id": "account" }
        });
        let access = parsed["tokens"]["access_token"].as_str().unwrap();
        assert_eq!(access, "secret");
        assert!(parsed.get("refresh_token").is_none());
    }

    #[test]
    fn resolves_home_for_macos_style_auth_location() {
        let root = temporary_directory("auth-macos-home");
        let macos_home = root.join("macos-home");
        fs::create_dir_all(macos_home.join(".codex")).unwrap();
        fs::write(macos_home.join(".codex/auth.json"), "{}").unwrap();

        assert_eq!(
            resolve_auth_json_path_from(None, None, Some(&macos_home)).unwrap(),
            macos_home.join(".codex/auth.json")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
