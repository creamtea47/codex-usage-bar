use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 已确认重置事件在本机保持锁定的时长，避免相邻刷新重复广播或重复发送。
pub const RESET_EVENT_LOCK_SECONDS: i64 = 30 * 60;

/// 对外稳定的重置触发原因。枚举值同时供 IPC、通知和审计文件复用。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum QuotaResetReason {
    /// 额度由不足 100% 恢复到 100%，这是无需等待旧截止时间的强重置证据。
    QuotaRecovered,
    /// 已到达旧截止时间，且上游 `reset_at` 已推进到下一周期。
    DeadlineReached,
}

/// 系统通知对一个事件的最终处置；只有 `Pending` 可由调用层领取。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum NotificationDisposition {
    #[default]
    Pending,
    /// 已原子领取、尚未取得平台 `show()` 结果；崩溃恢复时不得再次投递。
    Claimed,
    Queued,
    Failed,
    SuppressedDisabled,
    SuppressedQuiet,
}

/// 跨自动接续、系统通知和趋势分段共享的已确认重置事件。
///
/// 事件只包含脱敏窗口指纹，不持久化上游窗口 ID、账号或认证信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaResetEvent {
    pub event_id: String,
    pub generation_id: String,
    pub window_fingerprint: String,
    pub window_seconds: i64,
    pub reason: QuotaResetReason,
    pub detected_at: DateTime<Utc>,
    pub expected_reset_at: DateTime<Utc>,
    pub next_reset_at: Option<DateTime<Utc>>,
    pub previous_remaining_percent: u8,
    pub current_remaining_percent: u8,
    pub lock_expires_at: DateTime<Utc>,
    #[serde(default)]
    pub notification_disposition: NotificationDisposition,
}

impl QuotaResetEvent {
    /// 30 分钟锁只负责事件去重；周期是否已满足仍由自动接续状态机决定。
    pub fn is_locked_at(&self, now: DateTime<Utc>) -> bool {
        now <= self.lock_expires_at
    }
}

/// 为一个待处理周期生成不含账号明文的稳定 generation ID。
///
/// 截止时间后续发生小幅校正时应保留已生成的 ID，不能重新调用本函数换代。
pub fn generation_id(
    salt: &str,
    account_fingerprint: &str,
    window_fingerprint: &str,
    expected_reset_at: DateTime<Utc>,
    generation_sequence: u64,
) -> String {
    stable_id(
        b"codex-usage-bar:quota-generation:v2\0",
        &[
            salt.as_bytes(),
            account_fingerprint.as_bytes(),
            window_fingerprint.as_bytes(),
            expected_reset_at.timestamp().to_string().as_bytes(),
            generation_sequence.to_string().as_bytes(),
        ],
    )
}

/// 每个 generation 最多产生一个确定性 event ID，重启后重复观察不会制造新事件。
pub fn event_id(salt: &str, generation_id: &str) -> String {
    stable_id(
        b"codex-usage-bar:quota-reset-event:v2\0",
        &[salt.as_bytes(), generation_id.as_bytes()],
    )
}

/// 构造带统一 30 分钟锁的事件，集中约束百分比和下一截止时间字段。
#[allow(clippy::too_many_arguments)]
pub fn confirmed_event(
    salt: &str,
    generation_id: String,
    window_fingerprint: String,
    window_seconds: i64,
    reason: QuotaResetReason,
    detected_at: DateTime<Utc>,
    expected_reset_at: DateTime<Utc>,
    next_reset_at: Option<DateTime<Utc>>,
    previous_remaining_percent: u8,
    current_remaining_percent: u8,
) -> QuotaResetEvent {
    QuotaResetEvent {
        event_id: event_id(salt, &generation_id),
        generation_id,
        window_fingerprint,
        window_seconds,
        reason,
        detected_at,
        expected_reset_at,
        next_reset_at,
        previous_remaining_percent: previous_remaining_percent.min(100),
        current_remaining_percent: current_remaining_percent.min(100),
        lock_expires_at: detected_at + Duration::seconds(RESET_EVENT_LOCK_SECONDS),
        notification_disposition: NotificationDisposition::Pending,
    }
}

fn stable_id(namespace: &[u8], parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(namespace);
    for part in parts {
        hasher.update(part);
        hasher.update(b"\0");
    }
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, 0).unwrap()
    }

    #[test]
    fn identifiers_are_stable_and_scoped_to_generation() {
        let first = generation_id("salt", "account", "weekly", at(1_000), 1);
        let same = generation_id("salt", "account", "weekly", at(1_000), 1);
        let next = generation_id("salt", "account", "weekly", at(1_000), 2);
        assert_eq!(first, same);
        assert_ne!(first, next);
        assert_eq!(event_id("salt", &first), event_id("salt", &same));
        assert_ne!(event_id("salt", &first), event_id("salt", &next));
    }

    #[test]
    fn confirmed_event_clamps_percentages_and_locks_for_thirty_minutes() {
        let detected_at = at(2_000);
        let event = confirmed_event(
            "salt",
            "generation".to_owned(),
            "window".to_owned(),
            604_800,
            QuotaResetReason::QuotaRecovered,
            detected_at,
            at(1_900),
            Some(at(606_700)),
            250,
            180,
        );
        assert_eq!(event.previous_remaining_percent, 100);
        assert_eq!(event.current_remaining_percent, 100);
        assert_eq!(event.lock_expires_at, at(3_800));
        assert!(event.is_locked_at(at(3_800)));
        assert!(!event.is_locked_at(at(3_801)));
    }
}
