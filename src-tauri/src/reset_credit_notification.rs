use crate::{
    usage::{ResetCreditAvailability, UsageAccountIdentity},
    usage_history::{account_fingerprint, generate_local_salt, AccountIdentity},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex as StdMutex, MutexGuard as StdMutexGuard},
};

pub const STATE_FILE_NAME: &str = "reset-credit-notification.json";
const STATE_SCHEMA_VERSION: u32 = 1;

/// 一次确认的重置卡总数增长。只携带通知文案需要的非敏感计数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResetCreditIncrease {
    pub gained_count: u64,
    pub available_count: u64,
    pub applicable_available_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PersistedResetCreditState {
    schema_version: u32,
    salt: String,
    #[serde(default)]
    account_fingerprint: Option<String>,
    #[serde(default)]
    available_count: Option<u64>,
}

impl PersistedResetCreditState {
    fn fresh() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            salt: generate_local_salt(),
            account_fingerprint: None,
            available_count: None,
        }
    }

    fn valid(&self) -> bool {
        self.schema_version == STATE_SCHEMA_VERSION
            && account_fingerprint(&self.salt, AccountIdentity::Token("validation")).is_ok()
            && self.account_fingerprint.as_ref().is_none_or(|value| {
                value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
    }
}

/// 持久化重置卡数量基线，使应用重启后仍能识别离线期间到账的卡。
pub struct ResetCreditNotificationRuntime {
    path: PathBuf,
    state: StdMutex<PersistedResetCreditState>,
}

impl ResetCreditNotificationRuntime {
    pub fn new(path: PathBuf) -> Self {
        Self {
            state: StdMutex::new(load_state(&path)),
            path,
        }
    }

    fn state(&self) -> StdMutexGuard<'_, PersistedResetCreditState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 每次成功用量刷新都推进基线；开关和静默规则只决定是否展示返回的事件。
    /// 这样关闭通知期间发生的变化不会在重新开启后被当作新卡补报。
    pub fn observe(
        &self,
        identity: &UsageAccountIdentity,
        availability: Option<ResetCreditAvailability>,
    ) -> Option<ResetCreditIncrease> {
        let availability = availability?;
        let mut state = self.state();
        let identity = match identity {
            UsageAccountIdentity::AccountId(value) => AccountIdentity::AccountId(value),
            UsageAccountIdentity::Token(value) => AccountIdentity::Token(value),
        };
        let fingerprint = match account_fingerprint(&state.salt, identity) {
            Ok(value) => value,
            Err(_) => {
                log::warn!("重置卡检测无法建立脱敏账号指纹。");
                return None;
            }
        };

        let account_changed = state.account_fingerprint.as_deref() != Some(&fingerprint);
        let previous_count = (!account_changed)
            .then_some(state.available_count)
            .flatten();
        let event = previous_count
            .filter(|previous| availability.available_count > *previous)
            .map(|previous| ResetCreditIncrease {
                gained_count: availability.available_count - previous,
                available_count: availability.available_count,
                applicable_available_count: availability.applicable_available_count,
            });

        let changed = account_changed
            || state.available_count != Some(availability.available_count)
            || state.account_fingerprint.as_deref() != Some(&fingerprint);
        state.account_fingerprint = Some(fingerprint);
        state.available_count = Some(availability.available_count);
        if changed && save_state(&self.path, &state).is_err() {
            // 只记录固定类别；路径、账号指纹和具体数量都不会进入日志。
            log::warn!("重置卡检测状态保存失败：类别=storage。");
        }
        event
    }
}

fn load_state(path: &Path) -> PersistedResetCreditState {
    match fs::read(path) {
        Ok(contents) => serde_json::from_slice::<PersistedResetCreditState>(&contents)
            .ok()
            .filter(PersistedResetCreditState::valid)
            .unwrap_or_else(|| {
                log::warn!("重置卡检测状态无效，已安全重建基线。");
                PersistedResetCreditState::fresh()
            }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => PersistedResetCreditState::fresh(),
        Err(_) => {
            log::warn!("重置卡检测状态读取失败：类别=storage。");
            PersistedResetCreditState::fresh()
        }
    }
}

fn save_state(path: &Path, state: &PersistedResetCreditState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary_path = path.with_extension("json.tmp");
    let content = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
    let mut temporary = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary_path)?;
    temporary.write_all(&content)?;
    temporary.sync_all()?;
    drop(temporary);
    match fs::rename(&temporary_path, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temporary_path);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, time::SystemTime};

    fn temp_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("codex-reset-credit-{label}-{nonce}.json"))
    }

    fn identity(value: &str) -> UsageAccountIdentity {
        UsageAccountIdentity::AccountId(value.to_owned())
    }

    fn availability(available_count: u64, applicable: Option<u64>) -> ResetCreditAvailability {
        ResetCreditAvailability {
            available_count,
            applicable_available_count: applicable,
        }
    }

    fn cleanup(path: &Path) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(path.with_extension("json.tmp"));
    }

    #[test]
    fn first_observation_and_account_switch_only_establish_baselines() {
        let path = temp_path("baseline");
        let runtime = ResetCreditNotificationRuntime::new(path.clone());
        assert_eq!(
            runtime.observe(&identity("account-a"), Some(availability(1, Some(0)))),
            None
        );
        assert_eq!(
            runtime.observe(&identity("account-b"), Some(availability(4, Some(2)))),
            None
        );
        cleanup(&path);
    }

    #[test]
    fn reports_only_available_count_increases() {
        let path = temp_path("increase");
        let runtime = ResetCreditNotificationRuntime::new(path.clone());
        assert_eq!(
            runtime.observe(&identity("account"), Some(availability(0, Some(0)))),
            None
        );
        assert_eq!(
            runtime.observe(&identity("account"), Some(availability(1, Some(0)))),
            Some(ResetCreditIncrease {
                gained_count: 1,
                available_count: 1,
                applicable_available_count: Some(0),
            })
        );
        assert_eq!(
            runtime.observe(&identity("account"), Some(availability(3, Some(1)))),
            Some(ResetCreditIncrease {
                gained_count: 2,
                available_count: 3,
                applicable_available_count: Some(1),
            })
        );
        assert_eq!(
            runtime.observe(&identity("account"), Some(availability(3, Some(2)))),
            None
        );
        cleanup(&path);
    }

    #[test]
    fn decreases_advance_the_baseline_without_alerting() {
        let path = temp_path("decrease");
        let runtime = ResetCreditNotificationRuntime::new(path.clone());
        assert_eq!(
            runtime.observe(&identity("account"), Some(availability(2, None))),
            None
        );
        assert_eq!(
            runtime.observe(&identity("account"), Some(availability(0, None))),
            None
        );
        assert_eq!(
            runtime.observe(&identity("account"), Some(availability(1, None))),
            Some(ResetCreditIncrease {
                gained_count: 1,
                available_count: 1,
                applicable_available_count: None,
            })
        );
        cleanup(&path);
    }

    #[test]
    fn missing_observations_do_not_replace_the_previous_baseline() {
        let path = temp_path("missing");
        let runtime = ResetCreditNotificationRuntime::new(path.clone());
        assert_eq!(
            runtime.observe(&identity("account"), Some(availability(0, None))),
            None
        );
        assert_eq!(runtime.observe(&identity("account"), None), None);
        assert_eq!(
            runtime.observe(&identity("account"), Some(availability(1, None))),
            Some(ResetCreditIncrease {
                gained_count: 1,
                available_count: 1,
                applicable_available_count: None,
            })
        );
        cleanup(&path);
    }

    #[test]
    fn persisted_baseline_detects_an_increase_after_restart() {
        let path = temp_path("restart");
        {
            let runtime = ResetCreditNotificationRuntime::new(path.clone());
            assert_eq!(
                runtime.observe(&identity("account"), Some(availability(0, None))),
                None
            );
        }
        let restarted = ResetCreditNotificationRuntime::new(path.clone());
        assert_eq!(
            restarted.observe(&identity("account"), Some(availability(1, Some(1)))),
            Some(ResetCreditIncrease {
                gained_count: 1,
                available_count: 1,
                applicable_available_count: Some(1),
            })
        );
        cleanup(&path);
    }

    #[test]
    fn corrupt_state_rebuilds_without_reporting_existing_cards() {
        let path = temp_path("corrupt");
        fs::write(&path, b"not-json").unwrap();
        let runtime = ResetCreditNotificationRuntime::new(path.clone());
        assert_eq!(
            runtime.observe(&identity("account"), Some(availability(5, Some(0)))),
            None
        );
        cleanup(&path);
    }

    #[test]
    fn persisted_state_contains_only_redacted_baseline_fields() {
        let path = temp_path("redacted");
        let runtime = ResetCreditNotificationRuntime::new(path.clone());
        assert_eq!(
            runtime.observe(
                &identity("sensitive-account-id"),
                Some(availability(2, Some(1)))
            ),
            None
        );

        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let mut keys = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "accountFingerprint",
                "availableCount",
                "salt",
                "schemaVersion"
            ]
        );
        let rendered = value.to_string();
        assert!(!rendered.contains("sensitive-account-id"));
        assert!(!rendered.contains("applicableAvailableCount"));
        assert!(!path.with_extension("json.tmp").exists());
        cleanup(&path);
    }
}
