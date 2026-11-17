use crate::domain::history::account_fingerprint;
use crate::domain::history::generate_local_salt;
use crate::domain::history::AccountIdentity;
use crate::infra::usage::ResetCreditAvailability;
use crate::infra::usage::UsageAccountIdentity;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex as StdMutex, MutexGuard as StdMutexGuard},
};

#[derive(Debug, PartialEq, Eq)]
pub enum ExtraIncrease {
    Balance(String, String),
    Available,
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extra_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extra: Option<crate::domain::usage::ExtraCredits>,
}

impl PersistedResetCreditState {
    fn fresh() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            salt: generate_local_salt(),
            account_fingerprint: None,
            available_count: None,
            extra_fingerprint: None,
            extra: None,
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

    pub fn observe_extra(
        &self,
        identity: &UsageAccountIdentity,
        credits: Option<&crate::domain::usage::ExtraCredits>,
    ) -> Option<ExtraIncrease> {
        let mut next = credits?.clone();
        let mut state = self.state();
        let identity = match identity {
            UsageAccountIdentity::AccountId(id) => AccountIdentity::AccountId(id),
            UsageAccountIdentity::Token(token) => AccountIdentity::Token(token),
        };
        let fingerprint = account_fingerprint(&state.salt, identity).ok()?;
        let previous = state
            .extra
            .as_ref()
            .filter(|_| state.extra_fingerprint.as_ref() == Some(&fingerprint));
        let event = previous.and_then(|old| {
            use crate::domain::usage::parse_balance;
            if old.unlimited != Some(true) && next.unlimited != Some(true) {
                if let Some((before, after)) = old
                    .balance
                    .as_deref()
                    .and_then(parse_balance)
                    .zip(next.balance.as_deref().and_then(parse_balance))
                {
                    return (after > before).then(|| {
                        ExtraIncrease::Balance(
                            (after - before).normalize().to_string(),
                            after.normalize().to_string(),
                        )
                    });
                }
            }
            (old.has_credits == Some(false)
                && old.unlimited != Some(true)
                && (next.has_credits == Some(true) || next.unlimited == Some(true)))
            .then_some(ExtraIncrease::Available)
        });
        if let Some(old) = previous {
            // A sparse response may contain just one of the three credit fields.
            // Keep the last observation of each field for the next comparison.
            if next.balance.is_none() {
                next.balance.clone_from(&old.balance);
            }
            next.has_credits = next.has_credits.or(old.has_credits);
            next.unlimited = next.unlimited.or(old.unlimited);
        }
        if previous == Some(&next) {
            return None;
        }
        state.extra_fingerprint = Some(fingerprint);
        state.extra = Some(next);
        if save_state(&self.path, &state).is_err() {
            log::warn!("Could not persist credit notification baseline");
        }
        event
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
mod tests;
