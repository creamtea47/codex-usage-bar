use crate::{
    auth::{read_auth_credentials, AuthCredentials, AuthError},
    models::DashboardSnapshot,
    usage::UsageAccountIdentity,
    usage_history::{account_fingerprint, generate_local_salt, AccountIdentity},
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex as StdMutex, MutexGuard as StdMutexGuard},
    time::Duration,
};
use tokio::sync::{watch, Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};

pub const STATE_FILE_NAME: &str = "quota-auto-continue.json";
const STATE_SCHEMA_VERSION: u32 = 1;
const MODEL_MANIFEST_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/models";
const RESPONSES_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/responses";
const CODEX_CLIENT_VERSION: &str = "0.146.0";
const CODEX_USER_AGENT: &str = "codex_cli_rs/0.146.0 (Ubuntu 22.4.0; x86_64) xterm-256color";
const MODEL_FALLBACK: &str = "gpt-5.4";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const WEEKLY_MIN_SECONDS: i64 = 6 * 24 * 60 * 60;
const WEEKLY_MAX_SECONDS: i64 = 8 * 24 * 60 * 60;
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
    pub enabled: bool,
    pub phase: QuotaAutoContinuePhase,
    pub target_reset_at: Option<DateTime<Utc>>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub attempted_count: u8,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error_code: Option<QuotaAutoContinueErrorCode>,
    pub selected_model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClaimedAttempt {
    pub target_reset_at: DateTime<Utc>,
    pub account_fingerprint: String,
    pub window_fingerprint: String,
    pub slot_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreflightDecision {
    Proceed,
    AlreadyAdvanced,
    AccountChanged,
}

#[derive(Debug, Clone)]
struct Observation {
    account_fingerprint: String,
    window_fingerprint: Option<String>,
    weekly_reset_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedRuntimeState {
    schema_version: u32,
    salt: String,
    #[serde(default)]
    account_fingerprint: Option<String>,
    #[serde(default)]
    window_fingerprint: Option<String>,
    #[serde(default)]
    target_reset_at: Option<DateTime<Utc>>,
    #[serde(default)]
    consumed_slots: [bool; 4],
    #[serde(default)]
    request_completed: bool,
    #[serde(default = "default_waiting_phase")]
    phase: QuotaAutoContinuePhase,
    #[serde(default)]
    last_attempt_at: Option<DateTime<Utc>>,
    #[serde(default)]
    last_success_at: Option<DateTime<Utc>>,
    #[serde(default)]
    last_error_code: Option<QuotaAutoContinueErrorCode>,
}

fn default_waiting_phase() -> QuotaAutoContinuePhase {
    QuotaAutoContinuePhase::WaitingForWeeklyWindow
}

impl PersistedRuntimeState {
    fn fresh() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            salt: generate_local_salt(),
            account_fingerprint: None,
            window_fingerprint: None,
            target_reset_at: None,
            consumed_slots: [false; 4],
            request_completed: false,
            phase: QuotaAutoContinuePhase::WaitingForWeeklyWindow,
            last_attempt_at: None,
            last_success_at: None,
            last_error_code: None,
        }
    }

    fn valid(&self) -> bool {
        self.schema_version == STATE_SCHEMA_VERSION
            && account_fingerprint(&self.salt, AccountIdentity::Token("validation")).is_ok()
    }

    fn reset_schedule(
        &mut self,
        account_fingerprint: String,
        window_fingerprint: Option<String>,
        reset_at: Option<DateTime<Utc>>,
    ) {
        self.account_fingerprint = Some(account_fingerprint);
        self.window_fingerprint = window_fingerprint;
        self.target_reset_at = reset_at;
        self.consumed_slots = [false; 4];
        self.request_completed = false;
        self.phase = if reset_at.is_some() {
            QuotaAutoContinuePhase::Scheduled
        } else {
            QuotaAutoContinuePhase::WaitingForWeeklyWindow
        };
        self.last_error_code = None;
    }

    fn attempted_count(&self) -> u8 {
        self.consumed_slots
            .iter()
            .filter(|consumed| **consumed)
            .count() as u8
    }

    fn next_attempt_at(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if self.request_completed {
            return None;
        }
        let target = self.target_reset_at?;
        let last_deadline = target + ChronoDuration::seconds(ATTEMPT_OFFSETS_SECONDS[3]);
        if now > last_deadline {
            // 唤醒后台循环，让它持久化“已错过”，而不是无限等待下一次设置变更。
            return Some(now);
        }
        let elapsed = (now - target).num_seconds();
        if elapsed >= 0
            && self
                .consumed_slots
                .iter()
                .enumerate()
                .any(|(index, consumed)| !*consumed && ATTEMPT_OFFSETS_SECONDS[index] <= elapsed)
        {
            return Some(now);
        }
        self.consumed_slots
            .iter()
            .enumerate()
            .find(|(index, consumed)| !**consumed && ATTEMPT_OFFSETS_SECONDS[*index] > elapsed)
            .map(|(index, _)| target + ChronoDuration::seconds(ATTEMPT_OFFSETS_SECONDS[index]))
    }
}

#[derive(Clone)]
struct QuotaAutoContinueClient {
    client: Client,
    models_endpoint: String,
    responses_endpoint: String,
}

#[derive(Debug)]
pub struct SendFailure {
    pub code: QuotaAutoContinueErrorCode,
    pub model: Option<String>,
}

impl QuotaAutoContinueClient {
    fn new() -> Result<Self, QuotaAutoContinueErrorCode> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| QuotaAutoContinueErrorCode::Network)?;
        Ok(Self {
            client,
            models_endpoint: MODEL_MANIFEST_ENDPOINT.to_owned(),
            responses_endpoint: RESPONSES_ENDPOINT.to_owned(),
        })
    }

    #[cfg(test)]
    fn with_endpoints(models_endpoint: String, responses_endpoint: String) -> Self {
        Self {
            client: Client::builder()
                .connect_timeout(Duration::from_millis(250))
                .timeout(Duration::from_millis(250))
                .build()
                .unwrap(),
            models_endpoint,
            responses_endpoint,
        }
    }

    async fn send_greeting(
        &self,
        expected_account: Option<(&str, &str)>,
    ) -> Result<String, SendFailure> {
        let credentials = read_auth_credentials().map_err(|error| SendFailure {
            code: auth_error_code(error),
            model: None,
        })?;
        self.send_greeting_with_credentials(credentials, expected_account)
            .await
    }

    async fn send_greeting_with_credentials(
        &self,
        credentials: AuthCredentials,
        expected_account: Option<(&str, &str)>,
    ) -> Result<String, SendFailure> {
        if let Some((expected, salt)) = expected_account {
            let identity = credentials
                .account_id
                .as_deref()
                .map(AccountIdentity::AccountId)
                .unwrap_or_else(|| AccountIdentity::Token(&credentials.access_token));
            let actual = account_fingerprint(salt, identity).map_err(|_| SendFailure {
                code: QuotaAutoContinueErrorCode::AccountChanged,
                model: None,
            })?;
            if actual != expected {
                return Err(SendFailure {
                    code: QuotaAutoContinueErrorCode::AccountChanged,
                    model: None,
                });
            }
        }

        let model = match self.fetch_preferred_model(&credentials).await {
            Ok(model) => model,
            Err(
                QuotaAutoContinueErrorCode::AuthMissing | QuotaAutoContinueErrorCode::AuthInvalid,
            ) => {
                return Err(SendFailure {
                    code: QuotaAutoContinueErrorCode::AuthInvalid,
                    model: None,
                });
            }
            Err(error) => {
                log::info!("额度自动接续模型清单不可用，使用兼容回退：类别={error:?}。");
                MODEL_FALLBACK.to_owned()
            }
        };
        log::info!("额度自动接续准备发送最小请求：模型={model}。");

        let payload = build_greeting_payload(&model);
        let mut request = self
            .client
            .post(&self.responses_endpoint)
            .bearer_auth(&credentials.access_token)
            .header("Accept", "text/event-stream")
            .header("Content-Type", "application/json")
            .header("OpenAI-Beta", "responses=experimental")
            .header("Originator", "codex_cli_rs")
            .header("Version", CODEX_CLIENT_VERSION)
            .header("User-Agent", CODEX_USER_AGENT)
            .json(&payload);
        if let Some(account_id) = credentials.account_id.as_deref() {
            request = request.header("ChatGPT-Account-Id", account_id);
        }
        let mut response = request.send().await.map_err(|_| SendFailure {
            code: QuotaAutoContinueErrorCode::Network,
            model: Some(model.clone()),
        })?;
        if !response.status().is_success() {
            return Err(SendFailure {
                code: status_error_code(response.status()),
                model: Some(model),
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(SendFailure {
                code: QuotaAutoContinueErrorCode::InvalidResponse,
                model: Some(model),
            });
        }

        let mut pending = Vec::<u8>::new();
        let mut received = 0_usize;
        while let Some(chunk) = response.chunk().await.map_err(|_| SendFailure {
            code: QuotaAutoContinueErrorCode::Network,
            model: Some(model.clone()),
        })? {
            received = received.saturating_add(chunk.len());
            if received > MAX_RESPONSE_BYTES {
                return Err(SendFailure {
                    code: QuotaAutoContinueErrorCode::InvalidResponse,
                    model: Some(model),
                });
            }
            pending.extend_from_slice(&chunk);
            while let Some(position) = pending.iter().position(|byte| *byte == b'\n') {
                let line = pending.drain(..=position).collect::<Vec<_>>();
                match parse_sse_line(&line) {
                    SseSignal::Completed => return Ok(model),
                    SseSignal::Failed => {
                        return Err(SendFailure {
                            code: QuotaAutoContinueErrorCode::InvalidResponse,
                            model: Some(model),
                        });
                    }
                    SseSignal::Continue => {}
                }
            }
        }
        if parse_sse_line(&pending) == SseSignal::Completed {
            Ok(model)
        } else {
            Err(SendFailure {
                code: QuotaAutoContinueErrorCode::InvalidResponse,
                model: Some(model),
            })
        }
    }

    async fn fetch_preferred_model(
        &self,
        credentials: &AuthCredentials,
    ) -> Result<String, QuotaAutoContinueErrorCode> {
        let mut request = self
            .client
            .get(&self.models_endpoint)
            .query(&[("client_version", CODEX_CLIENT_VERSION)])
            .bearer_auth(&credentials.access_token)
            .header("Accept", "application/json")
            .header("Originator", "codex_cli_rs")
            .header("Version", CODEX_CLIENT_VERSION)
            .header("User-Agent", CODEX_USER_AGENT);
        if let Some(account_id) = credentials.account_id.as_deref() {
            request = request.header("ChatGPT-Account-Id", account_id);
        }
        let response = request
            .send()
            .await
            .map_err(|_| QuotaAutoContinueErrorCode::Network)?;
        if !response.status().is_success() {
            return Err(status_error_code(response.status()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(QuotaAutoContinueErrorCode::InvalidResponse);
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|_| QuotaAutoContinueErrorCode::InvalidResponse)?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(QuotaAutoContinueErrorCode::InvalidResponse);
        }
        let payload: Value = serde_json::from_slice(&bytes)
            .map_err(|_| QuotaAutoContinueErrorCode::InvalidResponse)?;
        select_preferred_model(&payload).ok_or(QuotaAutoContinueErrorCode::NoTextModel)
    }
}

pub struct QuotaAutoContinueRuntime {
    path: PathBuf,
    persisted: StdMutex<PersistedRuntimeState>,
    observation: StdMutex<Option<Observation>>,
    schedule_sender: watch::Sender<u64>,
    execution_guard: AsyncMutex<()>,
    client: QuotaAutoContinueClient,
    /// 模型名只用于当前进程的状态页，不落盘，避免运行文件超出脱敏排期白名单。
    selected_model: StdMutex<Option<String>>,
}

impl QuotaAutoContinueRuntime {
    pub fn new(path: PathBuf) -> Result<Self, QuotaAutoContinueErrorCode> {
        let persisted = load_runtime_state(&path);
        let (schedule_sender, _) = watch::channel(0_u64);
        Ok(Self {
            path,
            persisted: StdMutex::new(persisted),
            observation: StdMutex::new(None),
            schedule_sender,
            execution_guard: AsyncMutex::new(()),
            client: QuotaAutoContinueClient::new()?,
            selected_model: StdMutex::new(None),
        })
    }

    fn persisted(&self) -> StdMutexGuard<'_, PersistedRuntimeState> {
        self.persisted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn observation(&self) -> StdMutexGuard<'_, Option<Observation>> {
        self.observation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn selected_model(&self) -> StdMutexGuard<'_, Option<String>> {
        self.selected_model
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn notify_schedule_changed(&self) {
        self.schedule_sender.send_modify(|revision| {
            *revision = revision.wrapping_add(1);
        });
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.schedule_sender.subscribe()
    }

    pub fn execution_guard(&self) -> Result<AsyncMutexGuard<'_, ()>, QuotaAutoContinueErrorCode> {
        self.execution_guard
            .try_lock()
            .map_err(|_| QuotaAutoContinueErrorCode::Busy)
    }

    pub fn observe_dashboard(
        &self,
        enabled: bool,
        identity: &UsageAccountIdentity,
        snapshot: &DashboardSnapshot,
        now: DateTime<Utc>,
    ) -> bool {
        let mut persisted = self.persisted();
        let identity = match identity {
            UsageAccountIdentity::AccountId(value) => AccountIdentity::AccountId(value),
            UsageAccountIdentity::Token(value) => AccountIdentity::Token(value),
        };
        let Ok(fingerprint) = account_fingerprint(&persisted.salt, identity) else {
            log::warn!("额度自动接续无法建立脱敏账号指纹。");
            return false;
        };
        let weekly = weekly_window(snapshot);
        let reset_at = weekly.and_then(|window| window.reset_at);
        let window_fingerprint = weekly.map(|window| {
            quota_window_fingerprint(&persisted.salt, &window.id, window.window_seconds)
        });
        *self.observation() = Some(Observation {
            account_fingerprint: fingerprint.clone(),
            window_fingerprint: window_fingerprint.clone(),
            weekly_reset_at: reset_at,
        });
        if !enabled {
            return false;
        }

        let previous = serde_json::to_vec(&*persisted).ok();
        if persisted.account_fingerprint.as_deref() != Some(&fingerprint)
            || window_fingerprint
                .as_ref()
                .is_some_and(|current| persisted.window_fingerprint.as_ref() != Some(current))
        {
            persisted.reset_schedule(fingerprint, window_fingerprint, reset_at);
        } else {
            match (persisted.target_reset_at, reset_at) {
                (None, value) => persisted.reset_schedule(fingerprint, window_fingerprint, value),
                (Some(previous_reset), Some(current_reset)) if current_reset > previous_reset => {
                    persisted.reset_schedule(fingerprint, window_fingerprint, Some(current_reset));
                }
                _ => {}
            }
        }
        if let Some(target) = persisted.target_reset_at {
            if !persisted.request_completed
                && now > target + ChronoDuration::seconds(ATTEMPT_OFFSETS_SECONDS[3])
            {
                persisted.phase = QuotaAutoContinuePhase::Missed;
            }
        }
        let changed = previous.as_deref() != serde_json::to_vec(&*persisted).ok().as_deref();
        if changed {
            if save_runtime_state(&self.path, &persisted).is_err() {
                log::warn!("额度自动接续状态持久化失败：类别=storage。");
            }
            drop(persisted);
            self.notify_schedule_changed();
        }
        changed
    }

    pub fn activate_cached_observation(&self, enabled: bool, now: DateTime<Utc>) {
        if !enabled {
            self.notify_schedule_changed();
            return;
        }
        let observation = self.observation().clone();
        let Some(observation) = observation else {
            self.notify_schedule_changed();
            return;
        };
        let mut persisted = self.persisted();
        if persisted.account_fingerprint.as_deref() != Some(&observation.account_fingerprint)
            || observation
                .window_fingerprint
                .as_ref()
                .is_some_and(|current| persisted.window_fingerprint.as_ref() != Some(current))
            || persisted.target_reset_at.is_none()
        {
            persisted.reset_schedule(
                observation.account_fingerprint,
                observation.window_fingerprint,
                observation.weekly_reset_at,
            );
        }
        if let Some(target) = persisted.target_reset_at {
            if now > target + ChronoDuration::seconds(ATTEMPT_OFFSETS_SECONDS[3]) {
                persisted.phase = QuotaAutoContinuePhase::Missed;
            }
        }
        if save_runtime_state(&self.path, &persisted).is_err() {
            log::warn!("额度自动接续状态持久化失败：类别=storage。");
        }
        drop(persisted);
        self.notify_schedule_changed();
    }

    pub fn status(&self, enabled: bool, now: DateTime<Utc>) -> QuotaAutoContinueStatus {
        let persisted = self.persisted();
        QuotaAutoContinueStatus {
            enabled,
            phase: if enabled {
                persisted.phase
            } else {
                QuotaAutoContinuePhase::Disabled
            },
            target_reset_at: persisted.target_reset_at,
            next_attempt_at: enabled.then(|| persisted.next_attempt_at(now)).flatten(),
            attempted_count: persisted.attempted_count(),
            last_attempt_at: persisted.last_attempt_at,
            last_success_at: persisted.last_success_at,
            last_error_code: persisted.last_error_code,
            selected_model: self.selected_model().clone(),
        }
    }

    pub fn claim_due_attempt(
        &self,
        enabled: bool,
        now: DateTime<Utc>,
    ) -> Result<Option<ClaimedAttempt>, QuotaAutoContinueErrorCode> {
        if !enabled {
            return Ok(None);
        }
        let mut persisted = self.persisted();
        if persisted.request_completed {
            return Ok(None);
        }
        let Some(target) = persisted.target_reset_at else {
            return Ok(None);
        };
        let Some(account) = persisted.account_fingerprint.clone() else {
            return Ok(None);
        };
        let Some(window) = persisted.window_fingerprint.clone() else {
            return Ok(None);
        };
        let elapsed = (now - target).num_seconds();
        if elapsed > ATTEMPT_OFFSETS_SECONDS[3] {
            persisted.phase = QuotaAutoContinuePhase::Missed;
            save_runtime_state(&self.path, &persisted)
                .map_err(|_| QuotaAutoContinueErrorCode::Persistence)?;
            drop(persisted);
            self.notify_schedule_changed();
            return Ok(None);
        }
        let selected = persisted
            .consumed_slots
            .iter()
            .enumerate()
            .filter(|(index, consumed)| !**consumed && ATTEMPT_OFFSETS_SECONDS[*index] <= elapsed)
            .map(|(index, _)| index)
            .max();
        let Some(slot_index) = selected else {
            return Ok(None);
        };
        // 睡眠恢复只执行最近一个槽；更早且未执行的槽同时标记为已跳过，避免连续补跑。
        for consumed in persisted.consumed_slots.iter_mut().take(slot_index + 1) {
            *consumed = true;
        }
        persisted.phase = QuotaAutoContinuePhase::Running;
        persisted.last_attempt_at = Some(now);
        persisted.last_error_code = None;
        save_runtime_state(&self.path, &persisted)
            .map_err(|_| QuotaAutoContinueErrorCode::Persistence)?;
        drop(persisted);
        self.notify_schedule_changed();
        Ok(Some(ClaimedAttempt {
            target_reset_at: target,
            account_fingerprint: account,
            window_fingerprint: window,
            slot_index,
        }))
    }

    pub fn preflight_decision(&self, attempt: &ClaimedAttempt) -> PreflightDecision {
        let observation = self.observation();
        let Some(observation) = observation.as_ref() else {
            return PreflightDecision::AccountChanged;
        };
        if observation.account_fingerprint != attempt.account_fingerprint
            || observation.window_fingerprint.as_deref() != Some(&attempt.window_fingerprint)
        {
            return PreflightDecision::AccountChanged;
        }
        if observation
            .weekly_reset_at
            .is_some_and(|reset| reset > attempt.target_reset_at)
        {
            PreflightDecision::AlreadyAdvanced
        } else {
            PreflightDecision::Proceed
        }
    }

    pub fn finish_failure(
        &self,
        attempt: &ClaimedAttempt,
        error: QuotaAutoContinueErrorCode,
        model: Option<String>,
    ) {
        let mut persisted = self.persisted();
        if !same_cycle(&persisted, attempt) {
            return;
        }
        persisted.last_error_code = Some(error);
        *self.selected_model() = model;
        persisted.phase = if matches!(
            error,
            QuotaAutoContinueErrorCode::AuthMissing | QuotaAutoContinueErrorCode::AuthInvalid
        ) {
            QuotaAutoContinuePhase::AuthenticationRequired
        } else if persisted.consumed_slots[3] {
            QuotaAutoContinuePhase::Failed
        } else {
            QuotaAutoContinuePhase::WaitingForRetry
        };
        if save_runtime_state(&self.path, &persisted).is_err() {
            log::warn!("额度自动接续失败状态未能持久化：类别=storage。");
        }
        drop(persisted);
        self.notify_schedule_changed();
    }

    pub fn finish_sent(&self, attempt: &ClaimedAttempt, model: String, now: DateTime<Utc>) {
        let mut persisted = self.persisted();
        if !same_cycle(&persisted, attempt) {
            return;
        }
        persisted.request_completed = true;
        persisted.phase = QuotaAutoContinuePhase::SentAwaitingConfirmation;
        persisted.last_success_at = Some(now);
        persisted.last_error_code = None;
        *self.selected_model() = Some(model);
        if save_runtime_state(&self.path, &persisted).is_err() {
            log::warn!("额度自动接续成功状态未能持久化：类别=storage。");
        }
        drop(persisted);
        self.notify_schedule_changed();
    }

    pub fn mark_already_advanced(&self, attempt: &ClaimedAttempt, now: DateTime<Utc>) {
        let mut persisted = self.persisted();
        if same_cycle(&persisted, attempt) {
            persisted.request_completed = true;
            persisted.phase = QuotaAutoContinuePhase::Succeeded;
            persisted.last_success_at = Some(now);
            persisted.last_error_code = None;
            if save_runtime_state(&self.path, &persisted).is_err() {
                log::warn!("额度自动接续确认状态未能持久化：类别=storage。");
            }
        }
        drop(persisted);
        self.notify_schedule_changed();
    }

    pub async fn send_for_attempt(&self, attempt: &ClaimedAttempt) -> Result<String, SendFailure> {
        let salt = self.persisted().salt.clone();
        self.client
            .send_greeting(Some((&attempt.account_fingerprint, &salt)))
            .await
    }

    pub async fn send_manual_test(&self) -> Result<String, SendFailure> {
        let guard = self.observation().clone().map(|observation| {
            let salt = self.persisted().salt.clone();
            (observation.account_fingerprint, salt)
        });
        self.client
            .send_greeting(
                guard
                    .as_ref()
                    .map(|(fingerprint, salt)| (fingerprint.as_str(), salt.as_str())),
            )
            .await
    }

    pub fn record_manual_success(&self, model: String, now: DateTime<Utc>) {
        let mut persisted = self.persisted();
        persisted.last_success_at = Some(now);
        persisted.last_error_code = None;
        *self.selected_model() = Some(model);
        if persisted.target_reset_at.is_none() {
            persisted.phase = QuotaAutoContinuePhase::Succeeded;
        }
        if save_runtime_state(&self.path, &persisted).is_err() {
            log::warn!("额度自动接续手动测试结果未能持久化：类别=storage。");
        }
        drop(persisted);
        self.notify_schedule_changed();
    }

    pub fn record_manual_failure(&self, failure: &SendFailure) {
        let mut persisted = self.persisted();
        persisted.last_error_code = Some(failure.code);
        *self.selected_model() = failure.model.clone();
        if persisted.target_reset_at.is_none() {
            persisted.phase = if matches!(
                failure.code,
                QuotaAutoContinueErrorCode::AuthMissing | QuotaAutoContinueErrorCode::AuthInvalid
            ) {
                QuotaAutoContinuePhase::AuthenticationRequired
            } else {
                QuotaAutoContinuePhase::Failed
            };
        }
        if save_runtime_state(&self.path, &persisted).is_err() {
            log::warn!("额度自动接续手动测试失败状态未能持久化：类别=storage。");
        }
        drop(persisted);
        self.notify_schedule_changed();
    }
}

fn same_cycle(state: &PersistedRuntimeState, attempt: &ClaimedAttempt) -> bool {
    state.account_fingerprint.as_deref() == Some(&attempt.account_fingerprint)
        && state.window_fingerprint.as_deref() == Some(&attempt.window_fingerprint)
        && state.target_reset_at == Some(attempt.target_reset_at)
}

fn weekly_window(snapshot: &DashboardSnapshot) -> Option<&crate::models::QuotaWindow> {
    snapshot
        .quota_windows
        .iter()
        .filter(|window| (WEEKLY_MIN_SECONDS..=WEEKLY_MAX_SECONDS).contains(&window.window_seconds))
        .filter(|window| window.reset_at.is_some())
        .min_by_key(|window| window.reset_at)
}

fn quota_window_fingerprint(salt: &str, window_id: &str, window_seconds: i64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"codex-usage-bar:quota-auto-continue:window:v1\0");
    hasher.update(salt.as_bytes());
    hasher.update(b"\0");
    hasher.update(window_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(window_seconds.to_le_bytes());
    hex::encode(hasher.finalize())
}

fn auth_error_code(error: AuthError) -> QuotaAutoContinueErrorCode {
    match error {
        AuthError::MissingFile => QuotaAutoContinueErrorCode::AuthMissing,
        _ => QuotaAutoContinueErrorCode::AuthInvalid,
    }
}

fn status_error_code(status: StatusCode) -> QuotaAutoContinueErrorCode {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => QuotaAutoContinueErrorCode::AuthInvalid,
        StatusCode::TOO_MANY_REQUESTS => QuotaAutoContinueErrorCode::RateLimited,
        value if value.is_server_error() => QuotaAutoContinueErrorCode::ServiceUnavailable,
        _ => QuotaAutoContinueErrorCode::InvalidResponse,
    }
}

fn select_preferred_model(payload: &Value) -> Option<String> {
    let models = payload.get("models")?.as_array()?;
    let candidates = models
        .iter()
        .filter_map(|model| model.get("slug").and_then(Value::as_str))
        .map(str::trim)
        .filter(|slug| !slug.is_empty() && is_text_model(slug))
        .collect::<Vec<_>>();
    candidates
        .iter()
        .find(|slug| **slug == MODEL_FALLBACK)
        .or_else(|| candidates.first())
        .map(|slug| (*slug).to_owned())
}

fn is_text_model(slug: &str) -> bool {
    let slug = slug.to_ascii_lowercase();
    !slug.contains("image") && !slug.contains("video") && !slug.contains("audio")
}

fn build_greeting_payload(model: &str) -> Value {
    serde_json::json!({
        "model": model,
        "instructions": "Reply with a single short greeting.",
        "input": [{
            "role": "user",
            "content": [{"type": "input_text", "text": "hi"}]
        }],
        "stream": true,
        "store": false
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SseSignal {
    Continue,
    Completed,
    Failed,
}

fn parse_sse_line(line: &[u8]) -> SseSignal {
    let Ok(line) = std::str::from_utf8(line) else {
        return SseSignal::Continue;
    };
    let line = line.trim();
    let Some(data) = line.strip_prefix("data:") else {
        return SseSignal::Continue;
    };
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return SseSignal::Continue;
    }
    let Ok(payload) = serde_json::from_str::<Value>(data) else {
        return SseSignal::Continue;
    };
    match payload.get("type").and_then(Value::as_str) {
        Some("response.completed") => SseSignal::Completed,
        Some("response.failed" | "error") => SseSignal::Failed,
        _ => SseSignal::Continue,
    }
}

fn load_runtime_state(path: &Path) -> PersistedRuntimeState {
    let Ok(contents) = fs::read(path) else {
        return PersistedRuntimeState::fresh();
    };
    serde_json::from_slice::<PersistedRuntimeState>(&contents)
        .ok()
        .filter(PersistedRuntimeState::valid)
        .unwrap_or_else(|| {
            log::warn!("额度自动接续状态文件无效，已安全重建。");
            PersistedRuntimeState::fresh()
        })
}

fn save_runtime_state(path: &Path, state: &PersistedRuntimeState) -> io::Result<()> {
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
    use crate::models::{DashboardStatus, QuotaFallbackLabel, QuotaWindow};
    use std::{
        env,
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc::{self, Receiver},
        thread,
        time::SystemTime,
    };

    struct MockResponse {
        status: &'static str,
        content_type: &'static str,
        body: String,
        declared_length: Option<usize>,
        delay_millis: u64,
    }

    impl MockResponse {
        fn json(body: &str) -> Self {
            Self {
                status: "200 OK",
                content_type: "application/json",
                body: body.to_owned(),
                declared_length: None,
                delay_millis: 0,
            }
        }

        fn sse(body: &str) -> Self {
            Self {
                status: "200 OK",
                content_type: "text/event-stream",
                body: body.to_owned(),
                declared_length: None,
                delay_millis: 0,
            }
        }

        fn status(status: &'static str) -> Self {
            Self {
                status,
                content_type: "application/json",
                body: "{}".to_owned(),
                declared_length: None,
                delay_millis: 0,
            }
        }
    }

    /// 本地 TCP 服务只实现测试所需的最小 HTTP 子集，避免测试接触真实账号或外部网络。
    fn mock_http_server(
        responses: Vec<MockResponse>,
    ) -> (String, Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                let header_end = loop {
                    let read = stream.read(&mut buffer).unwrap_or(0);
                    if read == 0 {
                        break request.len();
                    }
                    request.extend_from_slice(&buffer[..read]);
                    if let Some(position) = request.windows(4).position(|part| part == b"\r\n\r\n")
                    {
                        break position + 4;
                    }
                };
                let header = String::from_utf8_lossy(&request[..header_end]).to_string();
                let content_length = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                while request.len() < header_end + content_length {
                    let read = stream.read(&mut buffer).unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                }
                let _ = sender.send(String::from_utf8_lossy(&request).to_string());
                if response.delay_millis > 0 {
                    thread::sleep(Duration::from_millis(response.delay_millis));
                }
                let declared_length = response.declared_length.unwrap_or(response.body.len());
                let wire = format!(
                    "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.status, response.content_type, declared_length, response.body
                );
                let _ = stream.write_all(wire.as_bytes());
            }
        });
        (format!("http://{address}"), receiver, handle)
    }

    fn mock_credentials() -> AuthCredentials {
        AuthCredentials {
            access_token: "test-access-token".to_owned(),
            account_id: Some("test-account-id".to_owned()),
        }
    }

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, 0).unwrap()
    }

    fn temp_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("codex-auto-continue-{name}-{nonce}.json"))
    }

    fn dashboard(reset_at: DateTime<Utc>) -> DashboardSnapshot {
        DashboardSnapshot {
            status: DashboardStatus::Ready,
            account_email_masked: None,
            plan_label: None,
            refreshed_at: Some(at(900)),
            next_refresh_at: None,
            message: None,
            quota_windows: vec![QuotaWindow {
                id: "weekly".to_owned(),
                label: None,
                fallback_label: QuotaFallbackLabel::Weekly,
                window_seconds: 7 * 24 * 60 * 60,
                used_percent: 0,
                remaining_percent: 100,
                reset_at: Some(reset_at),
                reset_after_seconds: 0,
                start_at: None,
                show_pace_marker: true,
                forecast: None,
            }],
        }
    }

    #[test]
    fn selects_stable_model_then_first_text_model() {
        let preferred = serde_json::json!({"models":[
            {"slug":"gpt-image-2"},
            {"slug":"gpt-5.6-sol"},
            {"slug":"gpt-5.4"}
        ]});
        assert_eq!(
            select_preferred_model(&preferred).as_deref(),
            Some("gpt-5.4")
        );
        let first = serde_json::json!({"models":[{"slug":"gpt-image-2"},{"slug":"gpt-5.6-sol"}]});
        assert_eq!(
            select_preferred_model(&first).as_deref(),
            Some("gpt-5.6-sol")
        );
    }

    #[test]
    fn payload_is_minimal_non_stored_streaming_greeting() {
        let payload = build_greeting_payload("gpt-5.4");
        assert_eq!(payload["model"], "gpt-5.4");
        assert_eq!(payload["input"][0]["content"][0]["text"], "hi");
        assert_eq!(payload["stream"], true);
        assert_eq!(payload["store"], false);
        assert!(!payload.to_string().contains("access_token"));
    }

    #[test]
    fn requires_response_completed_in_sse() {
        assert_eq!(
            parse_sse_line(b"data: {\"type\":\"response.completed\"}\n"),
            SseSignal::Completed
        );
        assert_eq!(
            parse_sse_line(b"data: {\"type\":\"response.failed\"}\n"),
            SseSignal::Failed
        );
        assert_eq!(parse_sse_line(b"data: [DONE]\n"), SseSignal::Continue);
    }

    #[tokio::test]
    async fn sends_required_identity_headers_and_minimal_non_stored_payload() {
        let (base, requests, server) = mock_http_server(vec![
            MockResponse::json(r#"{"models":[{"slug":"image-1"},{"slug":"gpt-5.4"}]}"#),
            MockResponse::sse("data: {\"type\":\"response.completed\"}\n\n"),
        ]);
        let client = QuotaAutoContinueClient::with_endpoints(
            format!("{base}/models"),
            format!("{base}/responses"),
        );

        let selected = client
            .send_greeting_with_credentials(mock_credentials(), None)
            .await
            .unwrap();
        assert_eq!(selected, "gpt-5.4");

        let models_request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
        let response_request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
        server.join().unwrap();
        let models_lower = models_request.to_ascii_lowercase();
        assert!(models_lower.starts_with("get /models?client_version=0.146.0"));
        assert!(models_lower.contains("authorization: bearer test-access-token"));
        assert!(models_lower.contains("chatgpt-account-id: test-account-id"));
        assert!(models_lower.contains("originator: codex_cli_rs"));

        let response_lower = response_request.to_ascii_lowercase();
        assert!(response_lower.starts_with("post /responses"));
        assert!(response_lower.contains("accept: text/event-stream"));
        assert!(response_lower.contains("openai-beta: responses=experimental"));
        assert!(response_lower.contains("version: 0.146.0"));
        let body = response_request.split("\r\n\r\n").nth(1).unwrap();
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["input"][0]["content"][0]["text"], "hi");
        assert_eq!(payload["stream"], true);
        assert_eq!(payload["store"], false);
        assert_eq!(payload["model"], "gpt-5.4");
    }

    #[tokio::test]
    async fn falls_back_to_compatible_model_when_manifest_is_unavailable() {
        let (base, _requests, server) = mock_http_server(vec![
            MockResponse::status("503 Service Unavailable"),
            MockResponse::sse("data: {\"type\":\"response.completed\"}\n\n"),
        ]);
        let client = QuotaAutoContinueClient::with_endpoints(
            format!("{base}/models"),
            format!("{base}/responses"),
        );

        let selected = client
            .send_greeting_with_credentials(mock_credentials(), None)
            .await
            .unwrap();
        assert_eq!(selected, MODEL_FALLBACK);
        server.join().unwrap();
    }

    #[tokio::test]
    async fn maps_auth_and_rate_limit_failures_to_stable_result_codes() {
        let (auth_base, _requests, auth_server) =
            mock_http_server(vec![MockResponse::status("401 Unauthorized")]);
        let auth_client = QuotaAutoContinueClient::with_endpoints(
            format!("{auth_base}/models"),
            format!("{auth_base}/responses"),
        );
        let auth_failure = auth_client
            .send_greeting_with_credentials(mock_credentials(), None)
            .await
            .unwrap_err();
        assert_eq!(auth_failure.code, QuotaAutoContinueErrorCode::AuthInvalid);
        assert!(auth_failure.model.is_none());
        auth_server.join().unwrap();

        let (limit_base, _requests, limit_server) = mock_http_server(vec![
            MockResponse::json(r#"{"models":[{"slug":"text-model"}]}"#),
            MockResponse::status("429 Too Many Requests"),
        ]);
        let limit_client = QuotaAutoContinueClient::with_endpoints(
            format!("{limit_base}/models"),
            format!("{limit_base}/responses"),
        );
        let limit_failure = limit_client
            .send_greeting_with_credentials(mock_credentials(), None)
            .await
            .unwrap_err();
        assert_eq!(limit_failure.code, QuotaAutoContinueErrorCode::RateLimited);
        assert_eq!(limit_failure.model.as_deref(), Some("text-model"));
        limit_server.join().unwrap();
    }

    #[tokio::test]
    async fn rejects_failed_incomplete_and_oversized_response_streams() {
        for body in [
            "data: {\"type\":\"response.failed\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\"}\n\n",
        ] {
            let (base, _requests, server) = mock_http_server(vec![
                MockResponse::json(r#"{"models":[{"slug":"text-model"}]}"#),
                MockResponse::sse(body),
            ]);
            let client = QuotaAutoContinueClient::with_endpoints(
                format!("{base}/models"),
                format!("{base}/responses"),
            );
            let failure = client
                .send_greeting_with_credentials(mock_credentials(), None)
                .await
                .unwrap_err();
            assert_eq!(failure.code, QuotaAutoContinueErrorCode::InvalidResponse);
            server.join().unwrap();
        }

        let mut oversized = MockResponse::sse("");
        oversized.declared_length = Some(MAX_RESPONSE_BYTES + 1);
        let (base, _requests, server) = mock_http_server(vec![
            MockResponse::json(r#"{"models":[{"slug":"text-model"}]}"#),
            oversized,
        ]);
        let client = QuotaAutoContinueClient::with_endpoints(
            format!("{base}/models"),
            format!("{base}/responses"),
        );
        let failure = client
            .send_greeting_with_credentials(mock_credentials(), None)
            .await
            .unwrap_err();
        assert_eq!(failure.code, QuotaAutoContinueErrorCode::InvalidResponse);
        server.join().unwrap();
    }

    #[tokio::test]
    async fn enforces_total_request_timeout_without_retrying_inside_one_slot() {
        let mut delayed_manifest = MockResponse::json(r#"{"models":[{"slug":"text-model"}]}"#);
        delayed_manifest.delay_millis = 800;
        let (base, requests, server) = mock_http_server(vec![
            delayed_manifest,
            MockResponse::sse("data: {\"type\":\"response.completed\"}\n\n"),
        ]);
        let client = QuotaAutoContinueClient::with_endpoints(
            format!("{base}/models"),
            format!("{base}/responses"),
        );
        let failure = client
            .send_greeting_with_credentials(mock_credentials(), None)
            .await
            .unwrap_err();
        assert_eq!(failure.code, QuotaAutoContinueErrorCode::Network);
        assert!(requests.recv_timeout(Duration::from_secs(1)).is_ok());
        server.join().unwrap();
    }

    #[test]
    fn schedules_immediate_and_three_retry_slots_without_back_to_back_catchup() {
        let path = temp_path("schedule");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        runtime.observe_dashboard(
            true,
            &UsageAccountIdentity::AccountId("account-a".to_owned()),
            &dashboard(at(1_000)),
            at(900),
        );

        let immediate = runtime.claim_due_attempt(true, at(1_000)).unwrap().unwrap();
        assert_eq!(immediate.slot_index, 0);
        runtime.finish_failure(&immediate, QuotaAutoContinueErrorCode::Network, None);
        assert_eq!(
            runtime.status(true, at(1_001)).next_attempt_at,
            Some(at(1_060))
        );

        // 睡眠到 +6 分钟时只执行最近的 +5 分钟槽，并跳过 +1 分钟槽。
        let resumed = runtime.claim_due_attempt(true, at(1_360)).unwrap().unwrap();
        assert_eq!(resumed.slot_index, 2);
        assert_eq!(runtime.status(true, at(1_360)).attempted_count, 3);
        runtime.finish_failure(&resumed, QuotaAutoContinueErrorCode::Network, None);
        assert_eq!(
            runtime.status(true, at(1_361)).next_attempt_at,
            Some(at(2_800))
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn runtime_state_replaces_atomically_and_contains_only_redacted_scheduler_fields() {
        let path = temp_path("state-schema");
        let mut state = PersistedRuntimeState::fresh();
        state.account_fingerprint = Some("a".repeat(64));
        state.window_fingerprint = Some("b".repeat(64));
        state.target_reset_at = Some(at(1_000));
        save_runtime_state(&path, &state).unwrap();
        state.phase = QuotaAutoContinuePhase::WaitingForRetry;
        state.last_error_code = Some(QuotaAutoContinueErrorCode::Network);
        save_runtime_state(&path, &state).unwrap();

        let bytes = fs::read(&path).unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        let object = value.as_object().unwrap();
        let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "accountFingerprint",
                "consumedSlots",
                "lastAttemptAt",
                "lastErrorCode",
                "lastSuccessAt",
                "phase",
                "requestCompleted",
                "salt",
                "schemaVersion",
                "targetResetAt",
                "windowFingerprint",
            ]
        );
        let serialized = String::from_utf8(bytes).unwrap().to_ascii_lowercase();
        for forbidden in [
            "access_token",
            "refresh_token",
            "account_id",
            "email",
            "authorization",
            "selectedmodel",
            "response.completed",
        ] {
            assert!(!serialized.contains(forbidden));
        }
        assert_eq!(
            load_runtime_state(&path).phase,
            QuotaAutoContinuePhase::WaitingForRetry
        );
        assert!(!path.with_extension("json.tmp").exists());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn misses_cycle_after_thirty_minutes_and_persists_consumed_slot_before_send() {
        let path = temp_path("missed");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        runtime.observe_dashboard(
            true,
            &UsageAccountIdentity::AccountId("account-a".to_owned()),
            &dashboard(at(1_000)),
            at(900),
        );
        assert!(runtime
            .claim_due_attempt(true, at(2_801))
            .unwrap()
            .is_none());
        assert_eq!(
            runtime.status(true, at(2_801)).phase,
            QuotaAutoContinuePhase::Missed
        );
        let reloaded = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        assert_eq!(
            reloaded.status(true, at(2_801)).phase,
            QuotaAutoContinuePhase::Missed
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn account_switch_replaces_old_schedule_and_preflight_rejects_old_attempt() {
        let path = temp_path("account-switch");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        runtime.observe_dashboard(
            true,
            &UsageAccountIdentity::AccountId("account-a".to_owned()),
            &dashboard(at(1_000)),
            at(900),
        );
        let attempt = runtime.claim_due_attempt(true, at(1_000)).unwrap().unwrap();
        runtime.observe_dashboard(
            true,
            &UsageAccountIdentity::AccountId("account-b".to_owned()),
            &dashboard(at(2_000)),
            at(1_001),
        );
        assert_eq!(
            runtime.preflight_decision(&attempt),
            PreflightDecision::AccountChanged
        );
        assert_eq!(
            runtime.status(true, at(1_001)).target_reset_at,
            Some(at(2_000))
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn advanced_deadline_suppresses_scheduled_send() {
        let path = temp_path("advanced");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        runtime.observe_dashboard(
            true,
            &UsageAccountIdentity::AccountId("account-a".to_owned()),
            &dashboard(at(1_000)),
            at(900),
        );
        let attempt = runtime.claim_due_attempt(true, at(1_000)).unwrap().unwrap();
        runtime.observe_dashboard(
            true,
            &UsageAccountIdentity::AccountId("account-a".to_owned()),
            &dashboard(at(1_000 + 7 * 24 * 60 * 60)),
            at(1_001),
        );
        assert_eq!(
            runtime.preflight_decision(&attempt),
            PreflightDecision::AlreadyAdvanced
        );
        let _ = fs::remove_file(path);
    }
}
