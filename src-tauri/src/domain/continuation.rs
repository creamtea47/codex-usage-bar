use crate::domain::history::{account_fingerprint, AccountIdentity};
use crate::domain::{
    history::generate_local_salt,
    quota_reset::{
        confirmed_event, generation_id, NotificationDisposition, QuotaResetEvent, QuotaResetReason,
    },
    usage::DashboardSnapshot,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResponseDetails {
    pub started_at: Option<DateTime<Utc>>,
    pub response_model: Option<String>,
    pub http_status: Option<u16>,
    pub duration_ms: u64,
    pub text: String,
    pub truncated: bool,
    pub error_message: Option<String>,
    /// POST 的交付状态不确定时不补发，避免连接超时造成重复消费。
    pub delivery_uncertain: bool,
}

pub(crate) const STATE_SCHEMA_VERSION: u32 = 2;
pub(crate) const WEEKLY_MIN_SECONDS: i64 = 6 * 24 * 60 * 60;
pub(crate) const WEEKLY_MAX_SECONDS: i64 = 8 * 24 * 60 * 60;
pub const ATTEMPT_OFFSETS_SECONDS: [i64; 4] = [0, 60, 5 * 60, 30 * 60];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum QuotaAutoContinuePhase {
    Disabled,
    WaitingForWeeklyWindow,
    Scheduled,
    Running,
    WaitingForRetry,
    Succeeded,
    SentAwaitingConfirmation,
    AuthenticationRequired,
    Missed,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum QuotaAutoContinueErrorCode {
    AuthMissing,
    AuthInvalid,
    Network,
    RateLimited,
    ServiceUnavailable,
    InvalidResponse,
    NoTextModel,
    AccountChanged,
    Persistence,
    Busy,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaAutoContinueStatus {
    pub account_id: Option<String>,
    pub enabled: bool,
    pub phase: QuotaAutoContinuePhase,
    pub target_reset_at: Option<DateTime<Utc>>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub attempted_count: u8,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error_code: Option<QuotaAutoContinueErrorCode>,
    pub selected_model: Option<String>,
    pub last_automatic_result: Option<QuotaAutoContinueResult>,
    pub last_manual_result: Option<QuotaAutoContinueResult>,
    pub last_trigger_reason: Option<QuotaResetReason>,
    pub last_reset_detected_at: Option<DateTime<Utc>>,
}

/// 自动发送与手动测试分开保存，避免“最近成功”掩盖本周期是否真正自动执行。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct QuotaAutoContinueResult {
    pub attempted_at: Option<DateTime<Utc>>,
    pub success_at: Option<DateTime<Utc>>,
    pub error_code: Option<QuotaAutoContinueErrorCode>,
    pub model: Option<String>,
    pub slot_index: Option<u8>,
    #[serde(default)]
    pub details: Option<ResponseDetails>,
}

/// 趋势侧车只需要稳定 generation 与脱敏窗口范围，不需要读取调度内部状态。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaGenerationContext {
    pub generation_id: String,
    pub window_fingerprint: String,
    pub window_seconds: i64,
}

#[derive(Debug, Clone)]
pub struct ClaimedAttempt {
    pub target_reset_at: DateTime<Utc>,
    pub account_fingerprint: String,
    pub window_fingerprint: String,
    pub slot_index: usize,
    pub generation_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreflightDecision {
    Proceed,
    AlreadyAdvanced,
    AccountChanged,
}

#[derive(Debug, Clone)]
pub(crate) struct Observation {
    pub(crate) account_fingerprint: String,
    pub(crate) window_fingerprint: Option<String>,
    pub(crate) weekly_reset_at: Option<DateTime<Utc>>,
    pub(crate) window_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PersistedCycleState {
    pub(crate) generation_id: String,
    pub(crate) expected_reset_at: DateTime<Utc>,
    #[serde(default)]
    pub(crate) confirmed_event: Option<QuotaResetEvent>,
    #[serde(default)]
    pub(crate) consumed_slots: [bool; 4],
    #[serde(default)]
    pub(crate) request_completed: bool,
    #[serde(default = "default_scheduled_phase")]
    pub(crate) phase: QuotaAutoContinuePhase,
}

pub(crate) fn default_scheduled_phase() -> QuotaAutoContinuePhase {
    QuotaAutoContinuePhase::Scheduled
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PersistedObservation {
    pub(crate) reset_at: Option<DateTime<Utc>>,
    pub(crate) remaining_percent: u8,
    pub(crate) observed_at: DateTime<Utc>,
}

/// 已确认事件的去重锁独立于 active cycle 生命周期，避免消费者结案并提升周期后丢锁。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PersistedEventLock {
    pub(crate) event_id: String,
    pub(crate) generation_id: String,
    pub(crate) lock_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PersistedRuntimeState {
    pub(crate) schema_version: u32,
    pub(crate) salt: String,
    #[serde(default)]
    pub(crate) account_fingerprint: Option<String>,
    #[serde(default)]
    pub(crate) window_fingerprint: Option<String>,
    #[serde(default)]
    pub(crate) window_seconds: Option<i64>,
    #[serde(default)]
    pub(crate) generation_sequence: u64,
    #[serde(default)]
    pub(crate) active_cycle: Option<PersistedCycleState>,
    #[serde(default)]
    pub(crate) pending_reset_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub(crate) last_event_lock: Option<PersistedEventLock>,
    /// v1 升级后的首个成功快照只建立安全 baseline，不能解释为刚发生的重置。
    #[serde(default)]
    pub(crate) migration_baseline_pending: bool,
    #[serde(default)]
    pub(crate) last_observation: Option<PersistedObservation>,
    #[serde(default)]
    pub(crate) last_automatic_result: Option<QuotaAutoContinueResult>,
    #[serde(default)]
    pub(crate) last_manual_result: Option<QuotaAutoContinueResult>,
    #[serde(default)]
    pub(crate) last_trigger_reason: Option<QuotaResetReason>,
    #[serde(default)]
    pub(crate) last_reset_detected_at: Option<DateTime<Utc>>,
}

/// v0.4.0 的 schema v1 只保存单一 target；显式结构用于无损迁移，禁止反序列化失败后静默丢排期。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PersistedRuntimeStateV1 {
    pub(crate) schema_version: u32,
    pub(crate) salt: String,
    #[serde(default)]
    pub(crate) account_fingerprint: Option<String>,
    #[serde(default)]
    pub(crate) window_fingerprint: Option<String>,
    #[serde(default)]
    pub(crate) target_reset_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub(crate) consumed_slots: [bool; 4],
    #[serde(default)]
    pub(crate) request_completed: bool,
    #[serde(default = "default_waiting_phase")]
    pub(crate) phase: QuotaAutoContinuePhase,
    #[serde(default)]
    pub(crate) last_attempt_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub(crate) last_success_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub(crate) last_error_code: Option<QuotaAutoContinueErrorCode>,
}

pub(crate) fn default_waiting_phase() -> QuotaAutoContinuePhase {
    QuotaAutoContinuePhase::WaitingForWeeklyWindow
}

impl PersistedRuntimeState {
    pub(crate) fn fresh() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            salt: generate_local_salt(),
            account_fingerprint: None,
            window_fingerprint: None,
            window_seconds: None,
            generation_sequence: 0,
            active_cycle: None,
            pending_reset_at: None,
            last_event_lock: None,
            migration_baseline_pending: false,
            last_observation: None,
            last_automatic_result: None,
            last_manual_result: None,
            last_trigger_reason: None,
            last_reset_detected_at: None,
        }
    }

    pub(crate) fn valid(&self) -> bool {
        self.schema_version == STATE_SCHEMA_VERSION
            && account_fingerprint(&self.salt, AccountIdentity::Token("validation")).is_ok()
    }

    pub(crate) fn start_cycle(
        &mut self,
        account_fingerprint: String,
        window_fingerprint: Option<String>,
        window_seconds: Option<i64>,
        reset_at: Option<DateTime<Utc>>,
    ) {
        self.account_fingerprint = Some(account_fingerprint);
        self.window_fingerprint = window_fingerprint;
        self.window_seconds = window_seconds;
        self.pending_reset_at = None;
        self.last_event_lock = None;
        self.generation_sequence = self.generation_sequence.saturating_add(1);
        self.active_cycle = reset_at.and_then(|expected_reset_at| {
            Some(PersistedCycleState {
                generation_id: generation_id(
                    &self.salt,
                    self.account_fingerprint.as_deref()?,
                    self.window_fingerprint.as_deref()?,
                    expected_reset_at,
                    self.generation_sequence,
                ),
                expected_reset_at,
                confirmed_event: None,
                consumed_slots: [false; 4],
                request_completed: false,
                phase: QuotaAutoContinuePhase::Scheduled,
            })
        });
    }

    pub(crate) fn attempted_count(&self) -> u8 {
        self.active_cycle
            .as_ref()
            .map(|cycle| {
                cycle
                    .consumed_slots
                    .iter()
                    .filter(|consumed| **consumed)
                    .count() as u8
            })
            .unwrap_or(0)
    }

    pub(crate) fn next_attempt_at(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let cycle = self.active_cycle.as_ref()?;
        if cycle.request_completed {
            return None;
        }
        let target = attempt_anchor(cycle);
        if cycle.confirmed_event.is_none() && now >= cycle.expected_reset_at {
            let reset_advanced = self
                .pending_reset_at
                .or_else(|| {
                    self.last_observation
                        .as_ref()
                        .and_then(|value| value.reset_at)
                })
                .is_some_and(|value| value > cycle.expected_reset_at);
            // 截止时间单独不是重置证据；等待下一次只读刷新，避免后台循环零延迟自旋。
            return Some(if reset_advanced {
                now
            } else {
                now + ChronoDuration::seconds(60)
            });
        }
        let last_deadline = target + ChronoDuration::seconds(ATTEMPT_OFFSETS_SECONDS[3]);
        if now > last_deadline {
            // 唤醒后台循环，让它持久化“已错过”，而不是无限等待下一次设置变更。
            return Some(now);
        }
        let elapsed = (now - target).num_seconds();
        if elapsed >= 0
            && self
                .active_cycle
                .as_ref()?
                .consumed_slots
                .iter()
                .enumerate()
                .any(|(index, consumed)| !*consumed && ATTEMPT_OFFSETS_SECONDS[index] <= elapsed)
        {
            return Some(now);
        }
        cycle
            .consumed_slots
            .iter()
            .enumerate()
            .find(|(index, consumed)| !**consumed && ATTEMPT_OFFSETS_SECONDS[*index] > elapsed)
            .map(|(index, _)| target + ChronoDuration::seconds(ATTEMPT_OFFSETS_SECONDS[index]))
    }

    pub(crate) fn phase(&self) -> QuotaAutoContinuePhase {
        self.active_cycle
            .as_ref()
            .map(|cycle| cycle.phase)
            .unwrap_or(QuotaAutoContinuePhase::WaitingForWeeklyWindow)
    }
}

pub(crate) fn same_cycle(state: &PersistedRuntimeState, attempt: &ClaimedAttempt) -> bool {
    state.account_fingerprint.as_deref() == Some(&attempt.account_fingerprint)
        && state.window_fingerprint.as_deref() == Some(&attempt.window_fingerprint)
        && state.active_cycle.as_ref().is_some_and(|cycle| {
            cycle.generation_id == attempt.generation_id
                && cycle.expected_reset_at == attempt.target_reset_at
        })
}

pub(crate) fn attempt_anchor(cycle: &PersistedCycleState) -> DateTime<Utc> {
    match cycle.confirmed_event.as_ref() {
        Some(event) if event.reason == QuotaResetReason::QuotaRecovered => event.detected_at,
        _ => cycle.expected_reset_at,
    }
}

pub(crate) fn promote_resolved_cycle(
    state: &mut PersistedRuntimeState,
    observed_reset_at: Option<DateTime<Utc>>,
) {
    let Some(cycle) = state.active_cycle.as_ref().filter(|cycle| {
        cycle.request_completed
            && cycle.confirmed_event.as_ref().is_none_or(|event| {
                !matches!(
                    event.notification_disposition,
                    NotificationDisposition::Pending | NotificationDisposition::Claimed
                )
            })
    }) else {
        return;
    };
    let next = state
        .pending_reset_at
        .or_else(|| observed_reset_at.filter(|value| *value > cycle.expected_reset_at));
    let Some(next) = next else {
        return;
    };
    // 重置事件生成的 generation 代表“重置后的新周期”；提升下一截止时间时必须沿用它，
    // 否则趋势会在恢复到 100% 的下一次刷新才断代，形成一次多余且滞后的分段。
    let generation_id = cycle.generation_id.clone();
    state.pending_reset_at = None;
    state.active_cycle = Some(PersistedCycleState {
        generation_id,
        expected_reset_at: next,
        confirmed_event: None,
        consumed_slots: [false; 4],
        request_completed: false,
        phase: QuotaAutoContinuePhase::Scheduled,
    });
}

/// 确认事件时冻结当前 generation；锁内重复观察返回 None，锁后新的恢复沿用窗口但换代 ID。
pub(crate) fn confirm_event_in_state(
    state: &mut PersistedRuntimeState,
    reason: QuotaResetReason,
    detected_at: DateTime<Utc>,
    previous_remaining_percent: u8,
    current_remaining_percent: u8,
    next_reset_at: Option<DateTime<Utc>>,
) -> Option<QuotaResetEvent> {
    if state
        .last_event_lock
        .as_ref()
        .is_some_and(|lock| detected_at <= lock.lock_expires_at)
    {
        return None;
    }
    let existing = state
        .active_cycle
        .as_ref()
        .and_then(|cycle| cycle.confirmed_event.as_ref())
        .cloned();
    if existing
        .as_ref()
        .is_some_and(|event| event.is_locked_at(detected_at))
    {
        return None;
    }

    // baseline generation 只描述尚未确认的旧周期；确认重置的同一快照必须先换代，
    // 使趋势、通知与自动接续从这一刻起共享同一个新 generation/event。
    let expected_reset_at = state.active_cycle.as_ref()?.expected_reset_at;
    let account = state.account_fingerprint.as_deref()?;
    let window = state.window_fingerprint.as_deref()?;
    state.generation_sequence = state.generation_sequence.saturating_add(1);
    let cycle = state.active_cycle.as_mut()?;
    cycle.generation_id = generation_id(
        &state.salt,
        account,
        window,
        expected_reset_at,
        state.generation_sequence,
    );
    cycle.confirmed_event = None;
    cycle.consumed_slots = [false; 4];
    cycle.request_completed = false;
    cycle.phase = QuotaAutoContinuePhase::Scheduled;

    let cycle = state.active_cycle.as_ref()?;
    let expected_reset_at = cycle.expected_reset_at;
    let generation_id = cycle.generation_id.clone();
    let window_fingerprint = state.window_fingerprint.clone()?;
    let window_seconds = state.window_seconds?;
    if let Some(next) = next_reset_at.filter(|value| *value > expected_reset_at) {
        state.pending_reset_at = Some(state.pending_reset_at.map_or(next, |known| known.max(next)));
    }
    let event = confirmed_event(
        &state.salt,
        generation_id,
        window_fingerprint,
        window_seconds,
        reason,
        detected_at,
        expected_reset_at,
        next_reset_at,
        previous_remaining_percent,
        current_remaining_percent,
    );
    state.last_event_lock = Some(PersistedEventLock {
        event_id: event.event_id.clone(),
        generation_id: event.generation_id.clone(),
        lock_expires_at: event.lock_expires_at,
    });
    let cycle = state.active_cycle.as_mut()?;
    cycle.confirmed_event = Some(event.clone());
    cycle.consumed_slots = [false; 4];
    cycle.request_completed = false;
    cycle.phase = QuotaAutoContinuePhase::Scheduled;
    state.last_trigger_reason = Some(reason);
    state.last_reset_detected_at = Some(detected_at);
    Some(event)
}

pub(crate) fn latest_compat_result(
    state: &PersistedRuntimeState,
) -> Option<QuotaAutoContinueResult> {
    match (
        state.last_automatic_result.as_ref(),
        state.last_manual_result.as_ref(),
    ) {
        (Some(automatic), Some(manual)) => {
            let automatic_at = automatic.success_at.or(automatic.attempted_at);
            let manual_at = manual.success_at.or(manual.attempted_at);
            if manual_at > automatic_at {
                Some(manual.clone())
            } else {
                Some(automatic.clone())
            }
        }
        (Some(automatic), None) => Some(automatic.clone()),
        (None, Some(manual)) => Some(manual.clone()),
        (None, None) => None,
    }
}

pub(crate) fn error_code_name(error: QuotaAutoContinueErrorCode) -> &'static str {
    match error {
        QuotaAutoContinueErrorCode::AuthMissing => "authMissing",
        QuotaAutoContinueErrorCode::AuthInvalid => "authInvalid",
        QuotaAutoContinueErrorCode::Network => "network",
        QuotaAutoContinueErrorCode::RateLimited => "rateLimited",
        QuotaAutoContinueErrorCode::ServiceUnavailable => "serviceUnavailable",
        QuotaAutoContinueErrorCode::InvalidResponse => "invalidResponse",
        QuotaAutoContinueErrorCode::NoTextModel => "noTextModel",
        QuotaAutoContinueErrorCode::AccountChanged => "accountChanged",
        QuotaAutoContinueErrorCode::Persistence => "persistence",
        QuotaAutoContinueErrorCode::Busy => "busy",
    }
}

pub(crate) fn weekly_window(
    snapshot: &DashboardSnapshot,
) -> Option<&crate::domain::usage::QuotaWindow> {
    snapshot
        .quota_windows
        .iter()
        .filter(|window| (WEEKLY_MIN_SECONDS..=WEEKLY_MAX_SECONDS).contains(&window.window_seconds))
        .filter(|window| window.reset_at.is_some())
        .min_by_key(|window| window.reset_at)
}

pub(crate) fn quota_window_fingerprint(salt: &str, window_id: &str, window_seconds: i64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"codex-usage-bar:quota-auto-continue:window:v1\0");
    hasher.update(salt.as_bytes());
    hasher.update(b"\0");
    hasher.update(window_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(window_seconds.to_le_bytes());
    hex::encode(hasher.finalize())
}
