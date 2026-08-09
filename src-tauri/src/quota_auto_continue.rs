use crate::{
    auth::{read_auth_credentials, AuthCredentials, AuthError},
    models::DashboardSnapshot,
    quota_audit::{QuotaAuditAction, QuotaAuditLog, QuotaAuditRecord},
    quota_reset::{
        confirmed_event, generation_id, NotificationDisposition, QuotaResetEvent, QuotaResetReason,
    },
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
const STATE_SCHEMA_VERSION: u32 = 2;
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
struct Observation {
    account_fingerprint: String,
    window_fingerprint: Option<String>,
    weekly_reset_at: Option<DateTime<Utc>>,
    window_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedCycleState {
    generation_id: String,
    expected_reset_at: DateTime<Utc>,
    #[serde(default)]
    confirmed_event: Option<QuotaResetEvent>,
    #[serde(default)]
    consumed_slots: [bool; 4],
    #[serde(default)]
    request_completed: bool,
    #[serde(default = "default_scheduled_phase")]
    phase: QuotaAutoContinuePhase,
}

fn default_scheduled_phase() -> QuotaAutoContinuePhase {
    QuotaAutoContinuePhase::Scheduled
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedObservation {
    reset_at: Option<DateTime<Utc>>,
    remaining_percent: u8,
    observed_at: DateTime<Utc>,
}

/// 已确认事件的去重锁独立于 active cycle 生命周期，避免消费者结案并提升周期后丢锁。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedEventLock {
    event_id: String,
    generation_id: String,
    lock_expires_at: DateTime<Utc>,
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
    window_seconds: Option<i64>,
    #[serde(default)]
    generation_sequence: u64,
    #[serde(default)]
    active_cycle: Option<PersistedCycleState>,
    #[serde(default)]
    pending_reset_at: Option<DateTime<Utc>>,
    #[serde(default)]
    last_event_lock: Option<PersistedEventLock>,
    /// v1 升级后的首个成功快照只建立安全 baseline，不能解释为刚发生的重置。
    #[serde(default)]
    migration_baseline_pending: bool,
    #[serde(default)]
    last_observation: Option<PersistedObservation>,
    #[serde(default)]
    last_automatic_result: Option<QuotaAutoContinueResult>,
    #[serde(default)]
    last_manual_result: Option<QuotaAutoContinueResult>,
    #[serde(default)]
    last_trigger_reason: Option<QuotaResetReason>,
    #[serde(default)]
    last_reset_detected_at: Option<DateTime<Utc>>,
}

/// v0.4.0 的 schema v1 只保存单一 target；显式结构用于无损迁移，禁止反序列化失败后静默丢排期。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedRuntimeStateV1 {
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

    fn valid(&self) -> bool {
        self.schema_version == STATE_SCHEMA_VERSION
            && account_fingerprint(&self.salt, AccountIdentity::Token("validation")).is_ok()
    }

    fn start_cycle(
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

    fn attempted_count(&self) -> u8 {
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

    fn next_attempt_at(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
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

    fn phase(&self) -> QuotaAutoContinuePhase {
        self.active_cycle
            .as_ref()
            .map(|cycle| cycle.phase)
            .unwrap_or(QuotaAutoContinuePhase::WaitingForWeeklyWindow)
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
    audit: QuotaAuditLog,
    persisted: StdMutex<PersistedRuntimeState>,
    observation: StdMutex<Option<Observation>>,
    schedule_sender: watch::Sender<u64>,
    execution_guard: AsyncMutex<()>,
    client: QuotaAutoContinueClient,
    /// 当前模型缓存用于运行中即时展示；分离后的自动/手动结果也会保存模型名，但审计不写模型。
    selected_model: StdMutex<Option<String>>,
}

impl QuotaAutoContinueRuntime {
    pub fn new(path: PathBuf) -> Result<Self, QuotaAutoContinueErrorCode> {
        Self::new_with_client(path, QuotaAutoContinueClient::new()?)
    }

    fn new_with_client(
        path: PathBuf,
        client: QuotaAutoContinueClient,
    ) -> Result<Self, QuotaAutoContinueErrorCode> {
        let (mut persisted, migrated) = load_runtime_state(&path);
        let interrupted_claim = persisted
            .active_cycle
            .as_mut()
            .and_then(|cycle| cycle.confirmed_event.as_mut())
            .filter(|event| event.notification_disposition == NotificationDisposition::Claimed)
            .map(|event| {
                // Claim 后崩溃不能重投；启动时归档为失败，解除周期提升阻塞并保留审计证据。
                event.notification_disposition = NotificationDisposition::Failed;
                event.clone()
            });
        let audit_directory = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let audit = QuotaAuditLog::new(audit_directory, Utc::now());
        if (migrated || interrupted_claim.is_some())
            && save_runtime_state(&path, &persisted).is_err()
        {
            return Err(QuotaAutoContinueErrorCode::Persistence);
        }
        if migrated {
            let _ = audit.append(&QuotaAuditRecord::info(
                Utc::now(),
                QuotaAuditAction::StateMigrated,
            ));
        }
        if let Some(event) = interrupted_claim {
            let mut record =
                QuotaAuditRecord::warning(Utc::now(), QuotaAuditAction::NotificationFailed)
                    .with_event(event.event_id, event.generation_id, event.reason);
            record.error_code = Some("claimInterrupted".to_owned());
            let _ = audit.append(&record);
        }
        let (schedule_sender, _) = watch::channel(0_u64);
        Ok(Self {
            path,
            audit,
            persisted: StdMutex::new(persisted),
            observation: StdMutex::new(None),
            schedule_sender,
            execution_guard: AsyncMutex::new(()),
            client,
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

    fn audit(&self, record: QuotaAuditRecord) {
        if self.audit.append(&record).is_err() {
            log::warn!("额度自动接续审计写入失败：类别=storage。");
        }
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
        // 指纹计算只需 salt；先复制并释放 state 锁，再更新 observation，
        // 最后重新锁 state，确保任何路径都不会同时持有两把互斥锁。
        let salt = self.persisted().salt.clone();
        let identity = match identity {
            UsageAccountIdentity::AccountId(value) => AccountIdentity::AccountId(value),
            UsageAccountIdentity::Token(value) => AccountIdentity::Token(value),
        };
        let Ok(fingerprint) = account_fingerprint(&salt, identity) else {
            log::warn!("额度自动接续无法建立脱敏账号指纹。");
            return false;
        };
        let weekly = weekly_window(snapshot);
        let reset_at = weekly.and_then(|window| window.reset_at);
        let remaining_percent = weekly.map(|window| window.remaining_percent);
        let window_seconds = weekly.map(|window| window.window_seconds);
        let window_fingerprint =
            weekly.map(|window| quota_window_fingerprint(&salt, &window.id, window.window_seconds));
        *self.observation() = Some(Observation {
            account_fingerprint: fingerprint.clone(),
            window_fingerprint: window_fingerprint.clone(),
            weekly_reset_at: reset_at,
            window_seconds,
        });
        let mut persisted = self.persisted();

        let previous = serde_json::to_vec(&*persisted).ok();
        let identity_changed = persisted.account_fingerprint.as_deref() != Some(&fingerprint)
            || window_fingerprint
                .as_ref()
                .is_some_and(|current| persisted.window_fingerprint.as_ref() != Some(current));
        let mut audit_records = Vec::new();
        if identity_changed {
            // 账号/窗口切换后的首个快照只建立 baseline，禁止补发旧账号事件。
            persisted.start_cycle(fingerprint, window_fingerprint, window_seconds, reset_at);
            persisted.migration_baseline_pending = false;
            persisted.last_observation =
                remaining_percent.map(|remaining_percent| PersistedObservation {
                    reset_at,
                    remaining_percent,
                    observed_at: now,
                });
        } else if persisted.migration_baseline_pending && weekly.is_some() {
            let expired_target_advanced = persisted.active_cycle.as_ref().is_some_and(|cycle| {
                cycle.expected_reset_at <= now
                    && reset_at.is_some_and(|value| value > cycle.expected_reset_at)
            });
            if expired_target_advanced || persisted.active_cycle.is_none() {
                // 旧 target 已经过期时，以首帧当前 reset_at 建立纯 baseline；不生成 event，
                // 不占 slot，也不把 v1 的过期排期解释成一次刚发生的重置。
                persisted.start_cycle(fingerprint, window_fingerprint, window_seconds, reset_at);
            } else if window_seconds.is_some() && persisted.window_seconds != window_seconds {
                // 未来 target 与已占槽位原样保留，仅补齐 v1 缺失的真实窗口秒数。
                persisted.window_seconds = window_seconds;
            }
            persisted.pending_reset_at = None;
            persisted.migration_baseline_pending = false;
            persisted.last_observation =
                remaining_percent.map(|remaining_percent| PersistedObservation {
                    reset_at,
                    remaining_percent,
                    observed_at: now,
                });
        } else {
            promote_resolved_cycle(&mut persisted, reset_at);
            if persisted.active_cycle.is_none() {
                persisted.start_cycle(fingerprint, window_fingerprint, window_seconds, reset_at);
            }

            let previous_observation = persisted.last_observation.clone();
            let quota_recovered = previous_observation.as_ref().is_some_and(|previous| {
                previous.remaining_percent < 100 && remaining_percent == Some(100)
            });
            if quota_recovered {
                let previous_remaining = previous_observation
                    .as_ref()
                    .map(|value| value.remaining_percent)
                    .unwrap_or(100);
                if let Some(event) = confirm_event_in_state(
                    &mut persisted,
                    QuotaResetReason::QuotaRecovered,
                    now,
                    previous_remaining,
                    100,
                    reset_at,
                ) {
                    audit_records.push(reset_confirmed_audit(&event, now));
                    if !enabled {
                        if let Some(cycle) = persisted.active_cycle.as_mut() {
                            cycle.request_completed = true;
                            cycle.phase = QuotaAutoContinuePhase::Succeeded;
                        }
                    }
                }
            }

            let mut pending_candidate = None;
            if let Some(cycle) = persisted.active_cycle.as_mut() {
                if let Some(current_reset) =
                    reset_at.filter(|value| *value > cycle.expected_reset_at)
                {
                    // 旧截止前观察到的任何更晚 reset_at 都只暂存为下一周期；
                    // 当前 active.expected 绝不能被刷新覆盖，否则旧截止到达时会漏确认事件。
                    pending_candidate = Some(current_reset);
                }

                let anchor = attempt_anchor(cycle);
                if cycle.confirmed_event.is_some()
                    && !cycle.request_completed
                    && now > anchor + ChronoDuration::seconds(ATTEMPT_OFFSETS_SECONDS[3])
                {
                    cycle.phase = QuotaAutoContinuePhase::Missed;
                    cycle.request_completed = true;
                    if let Some(event) = cycle.confirmed_event.as_ref() {
                        audit_records.push(
                            QuotaAuditRecord::warning(now, QuotaAuditAction::CycleMissed)
                                .with_event(
                                    event.event_id.clone(),
                                    event.generation_id.clone(),
                                    event.reason,
                                ),
                        );
                    }
                }
            }
            if let Some(next) = pending_candidate {
                persisted.pending_reset_at = Some(
                    persisted
                        .pending_reset_at
                        .map_or(next, |known| known.max(next)),
                );
            }

            let deadline_advanced = persisted.active_cycle.as_ref().is_some_and(|cycle| {
                cycle.confirmed_event.is_none()
                    && now >= cycle.expected_reset_at
                    && persisted
                        .pending_reset_at
                        .or(reset_at)
                        .is_some_and(|value| value > cycle.expected_reset_at)
            });
            if deadline_advanced {
                let previous_remaining = previous_observation
                    .as_ref()
                    .map(|value| value.remaining_percent)
                    .or(remaining_percent)
                    .unwrap_or(100);
                let current_remaining = remaining_percent.unwrap_or(previous_remaining);
                let next_reset_at = persisted.pending_reset_at.or(reset_at);
                if let Some(event) = confirm_event_in_state(
                    &mut persisted,
                    QuotaResetReason::DeadlineReached,
                    now,
                    previous_remaining,
                    current_remaining,
                    next_reset_at,
                ) {
                    audit_records.push(reset_confirmed_audit(&event, now));
                    if !enabled {
                        // 重置事件仍供通知和趋势消费，但关闭自动接续时本周期绝不补发历史 hi。
                        if let Some(cycle) = persisted.active_cycle.as_mut() {
                            cycle.request_completed = true;
                            cycle.phase = QuotaAutoContinuePhase::Succeeded;
                        }
                    }
                }
            }
            persisted.last_observation =
                remaining_percent.map(|remaining_percent| PersistedObservation {
                    reset_at,
                    remaining_percent,
                    observed_at: now,
                });
        }
        let changed = previous.as_deref() != serde_json::to_vec(&*persisted).ok().as_deref();
        if changed {
            if save_runtime_state(&self.path, &persisted).is_err() {
                log::warn!("额度自动接续状态持久化失败：类别=storage。");
            }
            drop(persisted);
            for record in audit_records {
                self.audit(record);
            }
            self.notify_schedule_changed();
        }
        changed
    }

    pub fn activate_cached_observation(&self, enabled: bool, now: DateTime<Utc>) {
        if !enabled {
            let mut persisted = self.persisted();
            let mut changed = false;
            if let Some(cycle) = persisted
                .active_cycle
                .as_mut()
                .filter(|cycle| cycle.confirmed_event.is_some() && !cycle.request_completed)
            {
                // 用户关闭开关即永久结案当前事件的自动消费者；通知 Pending 保持不变，
                // 之后重新开启也不能补发关闭期间的历史 hi。
                cycle.request_completed = true;
                cycle.phase = QuotaAutoContinuePhase::Succeeded;
                changed = true;
            }
            if changed && save_runtime_state(&self.path, &persisted).is_err() {
                log::warn!("额度自动接续禁用结案未能持久化：类别=storage。");
            }
            drop(persisted);
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
            || persisted.active_cycle.is_none()
        {
            persisted.start_cycle(
                observation.account_fingerprint,
                observation.window_fingerprint,
                observation.window_seconds,
                observation.weekly_reset_at,
            );
        }
        if let Some(cycle) = persisted.active_cycle.as_mut() {
            if cycle.confirmed_event.is_some()
                && now > attempt_anchor(cycle) + ChronoDuration::seconds(ATTEMPT_OFFSETS_SECONDS[3])
            {
                cycle.phase = QuotaAutoContinuePhase::Missed;
                cycle.request_completed = true;
            }
        }
        if save_runtime_state(&self.path, &persisted).is_err() {
            log::warn!("额度自动接续状态持久化失败：类别=storage。");
        }
        drop(persisted);
        self.notify_schedule_changed();
    }

    pub fn status(&self, enabled: bool, now: DateTime<Utc>) -> QuotaAutoContinueStatus {
        let selected_model = self.selected_model().clone();
        let persisted = self.persisted();
        let latest = latest_compat_result(&persisted);
        QuotaAutoContinueStatus {
            enabled,
            phase: if enabled {
                persisted.phase()
            } else {
                QuotaAutoContinuePhase::Disabled
            },
            target_reset_at: persisted
                .active_cycle
                .as_ref()
                .map(|cycle| cycle.expected_reset_at),
            next_attempt_at: enabled.then(|| persisted.next_attempt_at(now)).flatten(),
            attempted_count: persisted.attempted_count(),
            last_attempt_at: persisted
                .last_automatic_result
                .as_ref()
                .and_then(|result| result.attempted_at),
            last_success_at: latest.as_ref().and_then(|result| result.success_at),
            last_error_code: latest.as_ref().and_then(|result| result.error_code),
            selected_model: selected_model.or_else(|| latest.and_then(|result| result.model)),
            last_automatic_result: persisted.last_automatic_result.clone(),
            last_manual_result: persisted.last_manual_result.clone(),
            last_trigger_reason: persisted.last_trigger_reason,
            last_reset_detected_at: persisted.last_reset_detected_at,
        }
    }

    /// 仅向调用层返回尚未处置的重置通知事件；30 分钟锁只用于事件去重，
    /// 未处置通知必须跨重启保留，不能因为进程停机超过锁期而静默丢失。
    pub fn pending_reset_event(&self, _now: DateTime<Utc>) -> Option<QuotaResetEvent> {
        self.persisted()
            .active_cycle
            .as_ref()
            .and_then(|cycle| cycle.confirmed_event.as_ref())
            .filter(|event| event.notification_disposition == NotificationDisposition::Pending)
            .cloned()
    }

    pub fn current_generation(&self) -> Option<QuotaGenerationContext> {
        let persisted = self.persisted();
        let cycle = persisted.active_cycle.as_ref()?;
        Some(QuotaGenerationContext {
            generation_id: cycle.generation_id.clone(),
            window_fingerprint: persisted.window_fingerprint.clone()?,
            window_seconds: persisted.window_seconds?,
        })
    }

    pub fn mark_notification_queued(&self, event_id: &str, now: DateTime<Utc>) -> bool {
        self.mark_notification_disposition(
            event_id,
            NotificationDisposition::Queued,
            QuotaAuditAction::NotificationQueued,
            now,
        )
    }

    /// 在调用平台 `show()` 前原子领取事件；领取成功后 pending 查询立即不可见，保证至多一次。
    pub fn mark_notification_claimed(&self, event_id: &str, now: DateTime<Utc>) -> bool {
        self.mark_notification_disposition(
            event_id,
            NotificationDisposition::Claimed,
            QuotaAuditAction::NotificationClaimed,
            now,
        )
    }

    pub fn mark_notification_failed(&self, event_id: &str, now: DateTime<Utc>) -> bool {
        self.mark_notification_disposition(
            event_id,
            NotificationDisposition::Failed,
            QuotaAuditAction::NotificationFailed,
            now,
        )
    }

    pub fn mark_notification_suppressed(
        &self,
        event_id: &str,
        disposition: NotificationDisposition,
        now: DateTime<Utc>,
    ) -> bool {
        if !matches!(
            disposition,
            NotificationDisposition::SuppressedDisabled | NotificationDisposition::SuppressedQuiet
        ) {
            return false;
        }
        self.mark_notification_disposition(
            event_id,
            disposition,
            QuotaAuditAction::NotificationSuppressed,
            now,
        )
    }

    fn mark_notification_disposition(
        &self,
        event_id: &str,
        disposition: NotificationDisposition,
        action: QuotaAuditAction,
        now: DateTime<Utc>,
    ) -> bool {
        let mut persisted = self.persisted();
        let Some(event) = persisted
            .active_cycle
            .as_mut()
            .and_then(|cycle| cycle.confirmed_event.as_mut())
            .filter(|event| event.event_id == event_id)
        else {
            return false;
        };
        let transition_allowed = match disposition {
            NotificationDisposition::Claimed => {
                event.notification_disposition == NotificationDisposition::Pending
            }
            NotificationDisposition::Queued | NotificationDisposition::Failed => {
                event.notification_disposition == NotificationDisposition::Claimed
            }
            NotificationDisposition::SuppressedDisabled
            | NotificationDisposition::SuppressedQuiet => {
                event.notification_disposition == NotificationDisposition::Pending
            }
            NotificationDisposition::Pending => false,
        };
        if !transition_allowed {
            return false;
        }
        event.notification_disposition = disposition;
        let mut audit = QuotaAuditRecord::info(now, action).with_event(
            event.event_id.clone(),
            event.generation_id.clone(),
            event.reason,
        );
        audit.error_code = match disposition {
            NotificationDisposition::Failed => Some("platform".to_owned()),
            NotificationDisposition::SuppressedDisabled => Some("suppressedDisabled".to_owned()),
            NotificationDisposition::SuppressedQuiet => Some("suppressedQuiet".to_owned()),
            NotificationDisposition::Pending
            | NotificationDisposition::Claimed
            | NotificationDisposition::Queued => None,
        };
        if save_runtime_state(&self.path, &persisted).is_err() {
            log::warn!("额度重置通知处置状态持久化失败：类别=storage。");
            return false;
        }
        drop(persisted);
        self.audit(audit);
        self.notify_schedule_changed();
        true
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
        let Some(mut cycle) = persisted.active_cycle.take() else {
            return Ok(None);
        };
        if cycle.request_completed {
            persisted.active_cycle = Some(cycle);
            return Ok(None);
        }
        let Some(account) = persisted.account_fingerprint.clone() else {
            persisted.active_cycle = Some(cycle);
            return Ok(None);
        };
        let Some(window) = persisted.window_fingerprint.clone() else {
            persisted.active_cycle = Some(cycle);
            return Ok(None);
        };
        let mut reset_audit = None;
        let deadline_advanced = persisted
            .pending_reset_at
            .or_else(|| {
                persisted
                    .last_observation
                    .as_ref()
                    .and_then(|value| value.reset_at)
            })
            .filter(|value| *value > cycle.expected_reset_at);
        if cycle.confirmed_event.is_none()
            && now >= cycle.expected_reset_at
            && deadline_advanced.is_some()
        {
            persisted.active_cycle = Some(cycle);
            let remaining = persisted
                .last_observation
                .as_ref()
                .map(|value| value.remaining_percent)
                .unwrap_or(100);
            let pending_reset_at = persisted.pending_reset_at;
            if let Some(event) = confirm_event_in_state(
                &mut persisted,
                QuotaResetReason::DeadlineReached,
                now,
                remaining,
                remaining,
                pending_reset_at,
            ) {
                reset_audit = Some(reset_confirmed_audit(&event, now));
            }
            cycle = persisted
                .active_cycle
                .take()
                .expect("active cycle retained");
        }
        if cycle.confirmed_event.is_none() {
            persisted.active_cycle = Some(cycle);
            return Ok(None);
        }
        let target = attempt_anchor(&cycle);
        let elapsed = (now - target).num_seconds();
        if elapsed > ATTEMPT_OFFSETS_SECONDS[3] {
            cycle.phase = QuotaAutoContinuePhase::Missed;
            cycle.request_completed = true;
            let missed_audit = cycle.confirmed_event.as_ref().map(|event| {
                QuotaAuditRecord::warning(now, QuotaAuditAction::CycleMissed).with_event(
                    event.event_id.clone(),
                    event.generation_id.clone(),
                    event.reason,
                )
            });
            persisted.active_cycle = Some(cycle);
            save_runtime_state(&self.path, &persisted)
                .map_err(|_| QuotaAutoContinueErrorCode::Persistence)?;
            drop(persisted);
            if let Some(record) = reset_audit {
                self.audit(record);
            }
            if let Some(record) = missed_audit {
                self.audit(record);
            }
            self.notify_schedule_changed();
            return Ok(None);
        }
        let selected = cycle
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
        for consumed in cycle.consumed_slots.iter_mut().take(slot_index + 1) {
            *consumed = true;
        }
        cycle.phase = QuotaAutoContinuePhase::Running;
        let event = cycle
            .confirmed_event
            .as_ref()
            .expect("due attempt must have a confirmed reset event")
            .clone();
        persisted.last_automatic_result = Some(QuotaAutoContinueResult {
            attempted_at: Some(now),
            success_at: None,
            error_code: None,
            model: None,
            slot_index: Some(slot_index as u8),
        });
        let generation_id = cycle.generation_id.clone();
        let expected_reset_at = cycle.expected_reset_at;
        persisted.active_cycle = Some(cycle);
        save_runtime_state(&self.path, &persisted)
            .map_err(|_| QuotaAutoContinueErrorCode::Persistence)?;
        drop(persisted);
        if let Some(record) = reset_audit {
            self.audit(record);
        }
        let mut claimed = QuotaAuditRecord::info(
            now,
            if slot_index == 0 {
                QuotaAuditAction::Slot0Claimed
            } else {
                QuotaAuditAction::RetrySlotClaimed
            },
        )
        .with_event(
            event.event_id.clone(),
            event.generation_id.clone(),
            event.reason,
        );
        claimed.slot_index = Some(slot_index as u8);
        self.audit(claimed);
        self.notify_schedule_changed();
        Ok(Some(ClaimedAttempt {
            target_reset_at: expected_reset_at,
            account_fingerprint: account,
            window_fingerprint: window,
            slot_index,
            generation_id,
        }))
    }

    pub fn preflight_decision(&self, attempt: &ClaimedAttempt) -> PreflightDecision {
        let observation = self.observation().clone();
        let Some(observation) = observation else {
            return PreflightDecision::AccountChanged;
        };
        if observation.account_fingerprint != attempt.account_fingerprint
            || observation.window_fingerprint.as_deref() != Some(&attempt.window_fingerprint)
        {
            return PreflightDecision::AccountChanged;
        }
        let same = {
            let persisted = self.persisted();
            same_cycle(&persisted, attempt)
        };
        if !same {
            return PreflightDecision::AlreadyAdvanced;
        }
        // reset_at 提前推进只进入 pending；不能再据此抑制已确认事件的发送。
        PreflightDecision::Proceed
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
        if let Some(result) = persisted.last_automatic_result.as_mut() {
            result.error_code = Some(error);
            result.model = model.clone();
        }
        let cycle = persisted.active_cycle.as_mut().expect("same cycle exists");
        let exhausted = cycle.consumed_slots[3];
        if exhausted {
            // +30 分钟末槽失败即为最终结案；否则 scheduler 会再次唤醒并误改成 Missed。
            cycle.request_completed = true;
        }
        cycle.phase = if matches!(
            error,
            QuotaAutoContinueErrorCode::AuthMissing | QuotaAutoContinueErrorCode::AuthInvalid
        ) {
            QuotaAutoContinuePhase::AuthenticationRequired
        } else if exhausted {
            QuotaAutoContinuePhase::Failed
        } else {
            QuotaAutoContinuePhase::WaitingForRetry
        };
        let audit = cycle.confirmed_event.as_ref().map(|event| {
            let mut record =
                QuotaAuditRecord::warning(Utc::now(), QuotaAuditAction::AutomaticFailed)
                    .with_event(
                        event.event_id.clone(),
                        event.generation_id.clone(),
                        event.reason,
                    );
            record.slot_index = Some(attempt.slot_index as u8);
            record.error_code = Some(error_code_name(error).to_owned());
            record
        });
        if save_runtime_state(&self.path, &persisted).is_err() {
            log::warn!("额度自动接续失败状态未能持久化：类别=storage。");
        }
        drop(persisted);
        *self.selected_model() = model;
        if let Some(record) = audit {
            self.audit(record);
        }
        self.notify_schedule_changed();
    }

    pub fn finish_sent(&self, attempt: &ClaimedAttempt, model: String, now: DateTime<Utc>) {
        let mut persisted = self.persisted();
        if !same_cycle(&persisted, attempt) {
            return;
        }
        if let Some(result) = persisted.last_automatic_result.as_mut() {
            result.success_at = Some(now);
            result.error_code = None;
            result.model = Some(model.clone());
        }
        let event = {
            let cycle = persisted.active_cycle.as_mut().expect("same cycle exists");
            cycle.request_completed = true;
            cycle.phase = QuotaAutoContinuePhase::SentAwaitingConfirmation;
            cycle.confirmed_event.clone()
        };
        let audit = event.as_ref().map(|event| {
            let mut record = QuotaAuditRecord::info(now, QuotaAuditAction::ResponseCompleted)
                .with_event(
                    event.event_id.clone(),
                    event.generation_id.clone(),
                    event.reason,
                );
            record.slot_index = Some(attempt.slot_index as u8);
            record
        });
        if save_runtime_state(&self.path, &persisted).is_err() {
            log::warn!("额度自动接续成功状态未能持久化：类别=storage。");
        }
        drop(persisted);
        *self.selected_model() = Some(model);
        if let Some(record) = audit {
            self.audit(record);
        }
        self.notify_schedule_changed();
    }

    pub fn mark_already_advanced(&self, attempt: &ClaimedAttempt, _now: DateTime<Utc>) {
        let mut persisted = self.persisted();
        if same_cycle(&persisted, attempt) {
            let cycle = persisted.active_cycle.as_mut().expect("same cycle exists");
            cycle.request_completed = true;
            cycle.phase = QuotaAutoContinuePhase::Succeeded;
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
        let guard = self.manual_account_guard();
        self.client
            .send_greeting(
                guard
                    .as_ref()
                    .map(|(fingerprint, salt)| (fingerprint.as_str(), salt.as_str())),
            )
            .await
    }

    fn manual_account_guard(&self) -> Option<(String, String)> {
        // 两次复制之间不持有任何 guard，避免与 observe_dashboard 形成 observation/state ABBA。
        let observation = self.observation().clone();
        let salt = self.persisted().salt.clone();
        observation.map(|observation| (observation.account_fingerprint, salt))
    }

    pub fn record_manual_success(&self, model: String, now: DateTime<Utc>) {
        let mut persisted = self.persisted();
        persisted.last_manual_result = Some(QuotaAutoContinueResult {
            attempted_at: Some(now),
            success_at: Some(now),
            error_code: None,
            model: Some(model.clone()),
            slot_index: None,
        });
        let audit = if let Some(cycle) = persisted
            .active_cycle
            .as_mut()
            .filter(|cycle| cycle.confirmed_event.is_some())
        {
            cycle.request_completed = true;
            cycle.phase = QuotaAutoContinuePhase::Succeeded;
            cycle.confirmed_event.as_ref().map(|event| {
                QuotaAuditRecord::info(now, QuotaAuditAction::ManualSatisfied).with_event(
                    event.event_id.clone(),
                    event.generation_id.clone(),
                    event.reason,
                )
            })
        } else {
            None
        };
        if let Some(record) = audit.as_ref() {
            let _ = record;
        }
        if save_runtime_state(&self.path, &persisted).is_err() {
            log::warn!("额度自动接续手动测试结果未能持久化：类别=storage。");
        }
        drop(persisted);
        *self.selected_model() = Some(model);
        if let Some(record) = audit {
            self.audit(record);
        }
        self.notify_schedule_changed();
    }

    pub fn record_manual_failure(&self, failure: &SendFailure) {
        let mut persisted = self.persisted();
        persisted.last_manual_result = Some(QuotaAutoContinueResult {
            attempted_at: Some(Utc::now()),
            success_at: None,
            error_code: Some(failure.code),
            model: failure.model.clone(),
            slot_index: None,
        });
        if save_runtime_state(&self.path, &persisted).is_err() {
            log::warn!("额度自动接续手动测试失败状态未能持久化：类别=storage。");
        }
        drop(persisted);
        *self.selected_model() = failure.model.clone();
        self.notify_schedule_changed();
    }
}

fn same_cycle(state: &PersistedRuntimeState, attempt: &ClaimedAttempt) -> bool {
    state.account_fingerprint.as_deref() == Some(&attempt.account_fingerprint)
        && state.window_fingerprint.as_deref() == Some(&attempt.window_fingerprint)
        && state.active_cycle.as_ref().is_some_and(|cycle| {
            cycle.generation_id == attempt.generation_id
                && cycle.expected_reset_at == attempt.target_reset_at
        })
}

fn attempt_anchor(cycle: &PersistedCycleState) -> DateTime<Utc> {
    match cycle.confirmed_event.as_ref() {
        Some(event) if event.reason == QuotaResetReason::QuotaRecovered => event.detected_at,
        _ => cycle.expected_reset_at,
    }
}

fn promote_resolved_cycle(
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
fn confirm_event_in_state(
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

fn reset_confirmed_audit(event: &QuotaResetEvent, now: DateTime<Utc>) -> QuotaAuditRecord {
    let mut record = QuotaAuditRecord::info(now, QuotaAuditAction::ResetConfirmed).with_event(
        event.event_id.clone(),
        event.generation_id.clone(),
        event.reason,
    );
    record.old_remaining_percent = Some(event.previous_remaining_percent);
    record.new_remaining_percent = Some(event.current_remaining_percent);
    record.old_reset_at = Some(event.expected_reset_at);
    record.new_reset_at = event.next_reset_at;
    record
}

fn latest_compat_result(state: &PersistedRuntimeState) -> Option<QuotaAutoContinueResult> {
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

fn error_code_name(error: QuotaAutoContinueErrorCode) -> &'static str {
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

fn load_runtime_state(path: &Path) -> (PersistedRuntimeState, bool) {
    let Ok(contents) = fs::read(path) else {
        return (PersistedRuntimeState::fresh(), false);
    };
    let version = serde_json::from_slice::<Value>(&contents)
        .ok()
        .and_then(|value| value.get("schemaVersion").and_then(Value::as_u64));
    if version == Some(u64::from(STATE_SCHEMA_VERSION)) {
        if let Some(mut state) = serde_json::from_slice::<PersistedRuntimeState>(&contents)
            .ok()
            .filter(PersistedRuntimeState::valid)
        {
            // 兼容 v0.4.1 开发期早期 schema v2：从仍保留的事件补回独立锁。
            if state.last_event_lock.is_none() {
                state.last_event_lock = state
                    .active_cycle
                    .as_ref()
                    .and_then(|cycle| cycle.confirmed_event.as_ref())
                    .map(|event| PersistedEventLock {
                        event_id: event.event_id.clone(),
                        generation_id: event.generation_id.clone(),
                        lock_expires_at: event.lock_expires_at,
                    });
            }
            return (state, false);
        }
    } else if version == Some(1) {
        if let Some(v1) = serde_json::from_slice::<PersistedRuntimeStateV1>(&contents)
            .ok()
            .filter(|state| {
                state.schema_version == 1
                    && account_fingerprint(&state.salt, AccountIdentity::Token("validation"))
                        .is_ok()
            })
        {
            return (migrate_v1(v1), true);
        }
    }
    log::warn!("额度自动接续状态文件无效，已安全重建。");
    (PersistedRuntimeState::fresh(), false)
}

fn migrate_v1(v1: PersistedRuntimeStateV1) -> PersistedRuntimeState {
    let mut state = PersistedRuntimeState {
        schema_version: STATE_SCHEMA_VERSION,
        salt: v1.salt,
        account_fingerprint: v1.account_fingerprint,
        window_fingerprint: v1.window_fingerprint,
        window_seconds: Some(7 * 24 * 60 * 60),
        generation_sequence: 0,
        active_cycle: None,
        pending_reset_at: None,
        last_event_lock: None,
        migration_baseline_pending: true,
        last_observation: None,
        last_automatic_result: v1
            .last_attempt_at
            .map(|attempted_at| QuotaAutoContinueResult {
                attempted_at: Some(attempted_at),
                success_at: v1.last_success_at,
                error_code: v1.last_error_code,
                model: None,
                slot_index: v1
                    .consumed_slots
                    .iter()
                    .rposition(|consumed| *consumed)
                    .map(|index| index as u8),
            }),
        last_manual_result: if v1.last_attempt_at.is_none() {
            v1.last_success_at
                .map(|success_at| QuotaAutoContinueResult {
                    attempted_at: None,
                    success_at: Some(success_at),
                    error_code: v1.last_error_code,
                    model: None,
                    slot_index: None,
                })
        } else {
            None
        },
        last_trigger_reason: None,
        last_reset_detected_at: None,
    };
    if let (Some(expected_reset_at), Some(account), Some(window)) = (
        v1.target_reset_at,
        state.account_fingerprint.as_deref(),
        state.window_fingerprint.as_deref(),
    ) {
        state.generation_sequence = 1;
        state.active_cycle = Some(PersistedCycleState {
            generation_id: generation_id(
                &state.salt,
                account,
                window,
                expected_reset_at,
                state.generation_sequence,
            ),
            expected_reset_at,
            confirmed_event: None,
            consumed_slots: v1.consumed_slots,
            request_completed: v1.request_completed,
            phase: v1.phase,
        });
    }
    state
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
        sync::{
            mpsc::{self, Receiver},
            Arc, Barrier,
        },
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
        let directory = env::temp_dir().join(format!("codex-auto-continue-{name}-{nonce}"));
        fs::create_dir_all(&directory).unwrap();
        directory.join(STATE_FILE_NAME)
    }

    fn cleanup(path: &Path) {
        if let Some(directory) = path.parent() {
            let _ = fs::remove_dir_all(directory);
        }
    }

    fn dashboard(reset_at: DateTime<Utc>) -> DashboardSnapshot {
        dashboard_with_remaining(reset_at, 100)
    }

    fn dashboard_with_remaining(
        reset_at: DateTime<Utc>,
        remaining_percent: u8,
    ) -> DashboardSnapshot {
        dashboard_with_window(reset_at, remaining_percent, 7 * 24 * 60 * 60)
    }

    fn dashboard_with_window(
        reset_at: DateTime<Utc>,
        remaining_percent: u8,
        window_seconds: i64,
    ) -> DashboardSnapshot {
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
                window_seconds,
                used_percent: 100_u8.saturating_sub(remaining_percent),
                remaining_percent,
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

    fn establish_recovered_reset(
        runtime: &QuotaAutoContinueRuntime,
        enabled: bool,
        old_reset_at: DateTime<Utc>,
        detected_at: DateTime<Utc>,
    ) -> QuotaResetEvent {
        let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
        runtime.observe_dashboard(
            enabled,
            &identity,
            &dashboard_with_remaining(old_reset_at, 99),
            detected_at - ChronoDuration::seconds(100),
        );
        runtime.observe_dashboard(
            enabled,
            &identity,
            &dashboard_with_remaining(
                old_reset_at + ChronoDuration::seconds(7 * 24 * 60 * 60),
                100,
            ),
            detected_at,
        );
        runtime.pending_reset_event(detected_at).unwrap()
    }

    #[test]
    fn schedules_immediate_and_three_retry_slots_without_back_to_back_catchup() {
        let path = temp_path("schedule");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let event = establish_recovered_reset(&runtime, true, at(1_000), at(1_000));
        assert_eq!(event.reason, QuotaResetReason::QuotaRecovered);

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
        cleanup(&path);
    }

    #[test]
    fn unchanged_deadline_recovery_confirms_event_and_claims_slot_zero() {
        let path = temp_path("unchanged-deadline-recovery");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
        runtime.observe_dashboard(
            true,
            &identity,
            &dashboard_with_remaining(at(1_000), 98),
            at(900),
        );
        runtime.observe_dashboard(
            true,
            &identity,
            &dashboard_with_remaining(at(1_000), 100),
            at(950),
        );

        let event = runtime.pending_reset_event(at(950)).unwrap();
        assert_eq!(event.reason, QuotaResetReason::QuotaRecovered);
        assert_eq!(event.expected_reset_at, at(1_000));
        assert_eq!(event.next_reset_at, Some(at(1_000)));
        let attempt = runtime.claim_due_attempt(true, at(950)).unwrap().unwrap();
        assert_eq!(attempt.slot_index, 0);
        cleanup(&path);
    }

    #[test]
    fn any_early_deadline_advance_stays_pending_without_overwriting_active_target() {
        let path = temp_path("small-deadline-correction");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
        runtime.observe_dashboard(
            true,
            &identity,
            &dashboard_with_remaining(at(1_000), 80),
            at(900),
        );
        runtime.observe_dashboard(
            true,
            &identity,
            &dashboard_with_remaining(at(1_100), 80),
            at(950),
        );

        assert_eq!(
            runtime.status(true, at(950)).target_reset_at,
            Some(at(1_000))
        );
        assert!(runtime.pending_reset_event(at(950)).is_none());
        let (state, _) = load_runtime_state(&path);
        assert_eq!(state.pending_reset_at, Some(at(1_100)));
        assert!(runtime.claim_due_attempt(true, at(950)).unwrap().is_none());
        cleanup(&path);
    }

    #[test]
    fn claims_all_four_retry_slots_at_exact_offsets() {
        let path = temp_path("all-retry-slots");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        establish_recovered_reset(&runtime, true, at(1_000), at(1_000));

        for (slot_index, attempted_at) in [1_000, 1_060, 1_300, 2_800].into_iter().enumerate() {
            let attempt = runtime
                .claim_due_attempt(true, at(attempted_at))
                .unwrap()
                .unwrap();
            assert_eq!(attempt.slot_index, slot_index);
            runtime.finish_failure(&attempt, QuotaAutoContinueErrorCode::Network, None);
        }
        let status = runtime.status(true, at(2_800));
        assert_eq!(status.attempted_count, 4);
        assert_eq!(status.phase, QuotaAutoContinuePhase::Failed);
        assert!(status.next_attempt_at.is_none());
        let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
        runtime.observe_dashboard(
            true,
            &identity,
            &dashboard(at(1_000 + 7 * 24 * 60 * 60)),
            at(2_801),
        );
        assert_eq!(
            runtime.status(true, at(2_801)).phase,
            QuotaAutoContinuePhase::Failed
        );
        let audit = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("quota-audit-")
            })
            .filter_map(|entry| fs::read_to_string(entry.path()).ok())
            .collect::<String>();
        assert!(!audit.contains("cycleMissed"));
        cleanup(&path);
    }

    #[test]
    fn completed_send_does_not_rearm_on_repeated_full_observations() {
        let path = temp_path("completed-no-rearm");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let first = establish_recovered_reset(&runtime, true, at(1_000), at(1_000));
        let attempt = runtime.claim_due_attempt(true, at(1_000)).unwrap().unwrap();
        runtime.finish_sent(&attempt, "gpt-5.4".to_owned(), at(1_001));

        let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
        let next_reset = at(1_000 + 7 * 24 * 60 * 60);
        runtime.observe_dashboard(true, &identity, &dashboard(next_reset), at(1_002));
        let still_pending = runtime.pending_reset_event(at(1_002)).unwrap();
        assert_eq!(still_pending.event_id, first.event_id);
        assert_eq!(still_pending.generation_id, first.generation_id);
        assert!(runtime
            .claim_due_attempt(true, at(1_060))
            .unwrap()
            .is_none());
        assert_eq!(runtime.status(true, at(1_060)).attempted_count, 1);
        cleanup(&path);
    }

    #[test]
    fn promoted_cycle_keeps_event_lock_during_post_success_quota_jitter() {
        let path = temp_path("promoted-lock");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let first = establish_recovered_reset(&runtime, true, at(10_000), at(100));
        let attempt = runtime.claim_due_attempt(true, at(100)).unwrap().unwrap();
        runtime.finish_sent(&attempt, "gpt-5.4".to_owned(), at(101));
        assert!(runtime.mark_notification_claimed(&first.event_id, at(102)));
        assert!(runtime.mark_notification_queued(&first.event_id, at(102)));

        let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
        let next_reset = at(10_000 + 7 * 24 * 60 * 60);
        runtime.observe_dashboard(true, &identity, &dashboard(next_reset), at(103));
        let (promoted, _) = load_runtime_state(&path);
        assert!(promoted
            .active_cycle
            .as_ref()
            .unwrap()
            .confirmed_event
            .is_none());
        let lock = promoted.last_event_lock.unwrap();
        assert_eq!(lock.event_id, first.event_id);
        assert_eq!(lock.generation_id, first.generation_id);

        runtime.observe_dashboard(
            true,
            &identity,
            &dashboard_with_remaining(next_reset, 99),
            at(200),
        );
        runtime.observe_dashboard(
            true,
            &identity,
            &dashboard_with_remaining(next_reset, 100),
            at(201),
        );
        assert!(runtime.pending_reset_event(at(201)).is_none());
        assert_eq!(
            runtime.current_generation().unwrap().generation_id,
            first.generation_id
        );
        assert!(runtime.claim_due_attempt(true, at(201)).unwrap().is_none());
        cleanup(&path);
    }

    #[test]
    fn repeated_recovery_edges_inside_lock_keep_the_same_event() {
        let path = temp_path("same-event-inside-lock");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let first = establish_recovered_reset(&runtime, true, at(10_000), at(100));
        let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
        let next_reset = at(10_000 + 7 * 24 * 60 * 60);

        for (remaining, observed_at) in [(99, 200), (100, 201), (99, 300), (100, 301)] {
            runtime.observe_dashboard(
                true,
                &identity,
                &dashboard_with_remaining(next_reset, remaining),
                at(observed_at),
            );
            let current = runtime.pending_reset_event(at(observed_at)).unwrap();
            assert_eq!(current.event_id, first.event_id);
            assert_eq!(current.generation_id, first.generation_id);
        }
        cleanup(&path);
    }

    #[test]
    fn runtime_state_replaces_atomically_and_contains_only_redacted_v2_fields() {
        let path = temp_path("state-schema");
        let mut state = PersistedRuntimeState::fresh();
        state.start_cycle(
            "a".repeat(64),
            Some("b".repeat(64)),
            Some(7 * 24 * 60 * 60),
            Some(at(1_000)),
        );
        save_runtime_state(&path, &state).unwrap();
        let cycle = state.active_cycle.as_mut().unwrap();
        cycle.phase = QuotaAutoContinuePhase::WaitingForRetry;
        cycle.consumed_slots[0] = true;
        state.last_automatic_result = Some(QuotaAutoContinueResult {
            attempted_at: Some(at(1_000)),
            success_at: None,
            error_code: Some(QuotaAutoContinueErrorCode::Network),
            model: None,
            slot_index: Some(0),
        });
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
                "activeCycle",
                "generationSequence",
                "lastAutomaticResult",
                "lastEventLock",
                "lastManualResult",
                "lastObservation",
                "lastResetDetectedAt",
                "lastTriggerReason",
                "migrationBaselinePending",
                "pendingResetAt",
                "salt",
                "schemaVersion",
                "windowFingerprint",
                "windowSeconds",
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
        let (loaded, migrated) = load_runtime_state(&path);
        assert!(!migrated);
        assert_eq!(loaded.phase(), QuotaAutoContinuePhase::WaitingForRetry);
        assert!(!path.with_extension("json.tmp").exists());
        cleanup(&path);
    }

    #[test]
    fn explicitly_migrates_v1_without_synthesizing_a_reset_event() {
        let path = temp_path("migrate-v1");
        let v1 = PersistedRuntimeStateV1 {
            schema_version: 1,
            salt: generate_local_salt(),
            account_fingerprint: Some("a".repeat(64)),
            window_fingerprint: Some("b".repeat(64)),
            target_reset_at: Some(at(1_000)),
            consumed_slots: [true, false, false, false],
            request_completed: false,
            phase: QuotaAutoContinuePhase::WaitingForRetry,
            last_attempt_at: Some(at(950)),
            last_success_at: None,
            last_error_code: Some(QuotaAutoContinueErrorCode::Network),
        };
        fs::write(&path, serde_json::to_vec_pretty(&v1).unwrap()).unwrap();

        let (migrated, was_migrated) = load_runtime_state(&path);
        assert!(was_migrated);
        assert_eq!(migrated.schema_version, STATE_SCHEMA_VERSION);
        assert_eq!(migrated.phase(), QuotaAutoContinuePhase::WaitingForRetry);
        assert!(migrated
            .active_cycle
            .as_ref()
            .unwrap()
            .confirmed_event
            .is_none());
        assert_eq!(migrated.last_automatic_result.unwrap().slot_index, Some(0));

        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        assert!(runtime.pending_reset_event(at(1_000)).is_none());
        let value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["schemaVersion"], STATE_SCHEMA_VERSION);
        cleanup(&path);
    }

    #[test]
    fn first_snapshot_after_v1_migration_fills_actual_window_seconds_only() {
        let path = temp_path("migrate-v1-window-seconds");
        let salt = generate_local_salt();
        let window_seconds = 6 * 24 * 60 * 60;
        let account = account_fingerprint(&salt, AccountIdentity::AccountId("account-a")).unwrap();
        let window = quota_window_fingerprint(&salt, "weekly", window_seconds);
        let v1 = PersistedRuntimeStateV1 {
            schema_version: 1,
            salt,
            account_fingerprint: Some(account),
            window_fingerprint: Some(window),
            target_reset_at: Some(at(1_000)),
            consumed_slots: [true, false, false, false],
            request_completed: false,
            phase: QuotaAutoContinuePhase::WaitingForRetry,
            last_attempt_at: None,
            last_success_at: None,
            last_error_code: None,
        };
        fs::write(&path, serde_json::to_vec_pretty(&v1).unwrap()).unwrap();
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let before = runtime.current_generation().unwrap();
        assert_eq!(before.window_seconds, 7 * 24 * 60 * 60);

        runtime.observe_dashboard(
            true,
            &UsageAccountIdentity::AccountId("account-a".to_owned()),
            &dashboard_with_window(at(1_000), 80, window_seconds),
            at(900),
        );
        let after = runtime.current_generation().unwrap();
        assert_eq!(after.generation_id, before.generation_id);
        assert_eq!(after.window_seconds, window_seconds);
        assert_eq!(runtime.status(true, at(900)).attempted_count, 1);
        assert_eq!(
            runtime.status(true, at(900)).phase,
            QuotaAutoContinuePhase::WaitingForRetry
        );
        assert!(runtime.pending_reset_event(at(900)).is_none());
        cleanup(&path);
    }

    #[test]
    fn v1_expired_target_and_advanced_first_snapshot_create_baseline_only() {
        let path = temp_path("migrate-v1-expired-first-frame");
        let salt = generate_local_salt();
        let window_seconds = 7 * 24 * 60 * 60;
        let account = account_fingerprint(&salt, AccountIdentity::AccountId("account-a")).unwrap();
        let window = quota_window_fingerprint(&salt, "weekly", window_seconds);
        let v1 = PersistedRuntimeStateV1 {
            schema_version: 1,
            salt,
            account_fingerprint: Some(account),
            window_fingerprint: Some(window),
            target_reset_at: Some(at(1_000)),
            consumed_slots: [true, true, false, false],
            request_completed: false,
            phase: QuotaAutoContinuePhase::WaitingForRetry,
            last_attempt_at: Some(at(900)),
            last_success_at: None,
            last_error_code: Some(QuotaAutoContinueErrorCode::Network),
        };
        fs::write(&path, serde_json::to_vec_pretty(&v1).unwrap()).unwrap();
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let current_reset_at = at(1_000 + window_seconds);

        runtime.observe_dashboard(
            true,
            &UsageAccountIdentity::AccountId("account-a".to_owned()),
            &dashboard_with_window(current_reset_at, 100, window_seconds),
            at(2_000),
        );

        let status = runtime.status(true, at(2_000));
        assert_eq!(status.target_reset_at, Some(current_reset_at));
        assert_eq!(status.attempted_count, 0);
        assert_eq!(status.phase, QuotaAutoContinuePhase::Scheduled);
        assert!(runtime.pending_reset_event(at(2_000)).is_none());
        assert!(runtime
            .claim_due_attempt(true, at(2_000))
            .unwrap()
            .is_none());
        let (state, _) = load_runtime_state(&path);
        assert!(!state.migration_baseline_pending);
        assert_eq!(state.last_observation.unwrap().remaining_percent, 100);
        cleanup(&path);
    }

    #[test]
    fn misses_only_a_confirmed_event_after_thirty_minutes() {
        let path = temp_path("missed");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        establish_recovered_reset(&runtime, true, at(1_000), at(1_000));
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
        cleanup(&path);
    }

    #[test]
    fn account_switch_replaces_old_schedule_and_preflight_rejects_old_attempt() {
        let path = temp_path("account-switch");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        establish_recovered_reset(&runtime, true, at(1_000), at(1_000));
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
        assert!(runtime.pending_reset_event(at(1_001)).is_none());
        cleanup(&path);
    }

    #[test]
    fn deadline_advance_is_pending_until_old_deadline_then_confirms_once() {
        let path = temp_path("deadline-advanced");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
        let next_reset = at(1_000 + 7 * 24 * 60 * 60);
        runtime.observe_dashboard(true, &identity, &dashboard(at(1_000)), at(900));
        let baseline_generation = runtime.current_generation().unwrap().generation_id;
        runtime.observe_dashboard(true, &identity, &dashboard(next_reset), at(950));
        assert_eq!(
            runtime.status(true, at(950)).target_reset_at,
            Some(at(1_000))
        );
        assert!(runtime.pending_reset_event(at(950)).is_none());
        assert!(runtime.claim_due_attempt(true, at(950)).unwrap().is_none());

        runtime.observe_dashboard(true, &identity, &dashboard(next_reset), at(1_000));
        let event = runtime.pending_reset_event(at(1_000)).unwrap();
        assert_eq!(event.reason, QuotaResetReason::DeadlineReached);
        assert_ne!(event.generation_id, baseline_generation);
        let attempt = runtime.claim_due_attempt(true, at(1_000)).unwrap().unwrap();
        assert_eq!(attempt.slot_index, 0);
        assert_eq!(
            runtime.preflight_decision(&attempt),
            PreflightDecision::Proceed
        );
        cleanup(&path);
    }

    #[test]
    fn confirmed_generation_begins_on_recovery_snapshot_and_survives_promotion() {
        let path = temp_path("generation-transition");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
        runtime.observe_dashboard(
            true,
            &identity,
            &dashboard_with_remaining(at(1_000), 98),
            at(900),
        );
        let baseline = runtime.current_generation().unwrap().generation_id;
        runtime.observe_dashboard(
            true,
            &identity,
            &dashboard_with_remaining(at(1_000 + 7 * 24 * 60 * 60), 100),
            at(950),
        );
        let event = runtime.pending_reset_event(at(950)).unwrap();
        assert_ne!(event.generation_id, baseline);
        assert_eq!(
            runtime.current_generation().unwrap().generation_id,
            event.generation_id
        );

        runtime.record_manual_success("gpt-5.4".to_owned(), at(951));
        assert!(runtime.mark_notification_claimed(&event.event_id, at(952)));
        assert!(runtime.mark_notification_queued(&event.event_id, at(952)));
        runtime.observe_dashboard(
            true,
            &identity,
            &dashboard(at(1_000 + 7 * 24 * 60 * 60)),
            at(953),
        );
        assert_eq!(
            runtime.current_generation().unwrap().generation_id,
            event.generation_id
        );
        assert_eq!(
            runtime.status(true, at(953)).target_reset_at,
            Some(at(1_000 + 7 * 24 * 60 * 60))
        );
        cleanup(&path);
    }

    #[test]
    fn lock_expiry_allows_a_new_recovery_event_with_a_new_generation() {
        let path = temp_path("lock-expiry");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let first = establish_recovered_reset(&runtime, true, at(10_000), at(100));
        let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
        let next_reset = at(10_000 + 7 * 24 * 60 * 60);
        runtime.observe_dashboard(
            true,
            &identity,
            &dashboard_with_remaining(next_reset, 99),
            at(1_901),
        );
        runtime.observe_dashboard(
            true,
            &identity,
            &dashboard_with_remaining(next_reset, 100),
            at(1_902),
        );
        let second = runtime.pending_reset_event(at(1_902)).unwrap();
        assert_ne!(second.event_id, first.event_id);
        assert_ne!(second.generation_id, first.generation_id);
        assert_eq!(second.reason, QuotaResetReason::QuotaRecovered);
        cleanup(&path);
    }

    #[test]
    fn disabled_confirmation_never_back_sends_historical_hi() {
        let path = temp_path("disabled-confirmation");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
        let next_reset = at(1_000 + 7 * 24 * 60 * 60);
        runtime.observe_dashboard(false, &identity, &dashboard(at(1_000)), at(900));
        runtime.observe_dashboard(false, &identity, &dashboard(next_reset), at(1_000));
        let event = runtime.pending_reset_event(at(1_000)).unwrap();
        assert_eq!(event.reason, QuotaResetReason::DeadlineReached);
        assert!(runtime
            .claim_due_attempt(true, at(1_000))
            .unwrap()
            .is_none());
        assert!(runtime.mark_notification_claimed(&event.event_id, at(1_001)));
        assert!(runtime.mark_notification_queued(&event.event_id, at(1_001)));
        runtime.observe_dashboard(true, &identity, &dashboard(next_reset), at(1_002));
        assert!(runtime
            .claim_due_attempt(true, at(1_002))
            .unwrap()
            .is_none());
        assert_eq!(
            runtime.status(true, at(1_002)).target_reset_at,
            Some(next_reset)
        );
        cleanup(&path);
    }

    #[test]
    fn disabling_after_confirmation_then_reenabling_never_sends_old_event() {
        let path = temp_path("disable-after-confirmation");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let event = establish_recovered_reset(&runtime, true, at(1_000), at(950));
        runtime.activate_cached_observation(false, at(951));
        assert_eq!(
            runtime.pending_reset_event(at(951)).unwrap().event_id,
            event.event_id
        );
        runtime.activate_cached_observation(true, at(952));
        assert!(runtime.claim_due_attempt(true, at(952)).unwrap().is_none());
        assert_eq!(
            runtime.status(true, at(952)).phase,
            QuotaAutoContinuePhase::Succeeded
        );
        cleanup(&path);
    }

    #[test]
    fn pending_notification_survives_restart_and_claim_can_finish_failed() {
        let path = temp_path("notification-restart");
        let event_id = {
            let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
            establish_recovered_reset(&runtime, true, at(1_000), at(1_000)).event_id
        };
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        // 即使重启跨过 30 分钟去重锁，未处置通知仍必须可领取。
        assert_eq!(
            runtime.pending_reset_event(at(3_001)).unwrap().event_id,
            event_id
        );
        assert!(runtime.mark_notification_claimed(&event_id, at(3_002)));
        assert!(runtime.pending_reset_event(at(3_002)).is_none());
        assert!(runtime.mark_notification_failed(&event_id, at(3_003)));
        assert!(!runtime.mark_notification_failed(&event_id, at(3_004)));
        let (state, _) = load_runtime_state(&path);
        assert_eq!(
            state
                .active_cycle
                .unwrap()
                .confirmed_event
                .unwrap()
                .notification_disposition,
            NotificationDisposition::Failed
        );
        let audit = fs::read_to_string(runtime.audit.path_for(at(3_003))).unwrap();
        assert!(audit
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|record| record["action"] == "notificationClaimed"));
        assert!(!audit
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|record| record["action"] == "notificationQueued"));
        assert!(audit
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|record| record["action"] == "notificationFailed"
                && record["errorCode"] == "platform"));
        cleanup(&path);
    }

    #[test]
    fn notification_success_audit_is_queued_only_after_claim() {
        let path = temp_path("notification-queued-after-claim");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let event = establish_recovered_reset(&runtime, true, at(1_000), at(1_000));
        assert!(!runtime.mark_notification_queued(&event.event_id, at(1_001)));
        assert!(runtime.mark_notification_claimed(&event.event_id, at(1_002)));
        assert!(runtime.mark_notification_queued(&event.event_id, at(1_003)));
        let audit = fs::read_to_string(runtime.audit.path_for(at(1_003))).unwrap();
        let actions = audit
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter_map(|record| record["action"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let claimed = actions
            .iter()
            .position(|action| action == "notificationClaimed")
            .unwrap();
        let queued = actions
            .iter()
            .position(|action| action == "notificationQueued")
            .unwrap();
        assert!(claimed < queued);
        cleanup(&path);
    }

    #[test]
    fn interrupted_notification_claim_recovers_failed_without_redelivery() {
        let path = temp_path("notification-claim-interrupted");
        let event_id = {
            let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
            let event = establish_recovered_reset(&runtime, true, at(1_000), at(1_000));
            assert!(runtime.mark_notification_claimed(&event.event_id, at(1_001)));
            event.event_id
        };

        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        assert!(runtime.pending_reset_event(Utc::now()).is_none());
        let (state, _) = load_runtime_state(&path);
        let event = state.active_cycle.unwrap().confirmed_event.unwrap();
        assert_eq!(event.event_id, event_id);
        assert_eq!(
            event.notification_disposition,
            NotificationDisposition::Failed
        );
        let audit = fs::read_to_string(runtime.audit.path_for(Utc::now())).unwrap();
        assert!(audit
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|record| record["action"] == "notificationFailed"
                && record["errorCode"] == "claimInterrupted"));
        assert!(!audit
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|record| record["action"] == "notificationQueued"));
        cleanup(&path);
    }

    #[test]
    fn notification_suppression_audit_distinguishes_disabled_and_quiet() {
        for (index, (disposition, expected_code)) in [
            (
                NotificationDisposition::SuppressedDisabled,
                "suppressedDisabled",
            ),
            (NotificationDisposition::SuppressedQuiet, "suppressedQuiet"),
        ]
        .into_iter()
        .enumerate()
        {
            let path = temp_path(&format!("notification-suppressed-{index}"));
            let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
            let event = establish_recovered_reset(&runtime, true, at(1_000), at(1_000));
            assert!(runtime.mark_notification_suppressed(&event.event_id, disposition, at(1_001),));
            let audit = fs::read_to_string(runtime.audit.path_for(at(1_001))).unwrap();
            assert!(audit
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .any(|record| record["action"] == "notificationSuppressed"
                    && record["errorCode"] == expected_code));
            cleanup(&path);
        }
    }

    #[test]
    fn manual_success_satisfies_only_an_already_confirmed_event() {
        let path = temp_path("manual-satisfaction");
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
        let next_reset = at(1_000 + 7 * 24 * 60 * 60);
        runtime.observe_dashboard(true, &identity, &dashboard(at(1_000)), at(900));
        runtime.record_manual_success("gpt-5.4".to_owned(), at(950));
        assert_eq!(
            runtime.status(true, at(950)).phase,
            QuotaAutoContinuePhase::Scheduled
        );

        runtime.observe_dashboard(true, &identity, &dashboard(next_reset), at(1_000));
        assert!(runtime.pending_reset_event(at(1_000)).is_some());
        runtime.record_manual_success("gpt-5.4".to_owned(), at(1_001));
        let status = runtime.status(true, at(1_001));
        assert_eq!(status.phase, QuotaAutoContinuePhase::Succeeded);
        assert!(status.last_automatic_result.is_none());
        assert_eq!(
            status.last_manual_result.unwrap().success_at,
            Some(at(1_001))
        );
        assert!(runtime
            .claim_due_attempt(true, at(1_001))
            .unwrap()
            .is_none());
        cleanup(&path);
    }

    #[test]
    fn concurrent_observe_preflight_and_manual_guard_complete_without_lock_inversion() {
        let path = temp_path("lock-order");
        let runtime = Arc::new(QuotaAutoContinueRuntime::new(path.clone()).unwrap());
        establish_recovered_reset(&runtime, true, at(1_000), at(950));
        let attempt = runtime.claim_due_attempt(true, at(950)).unwrap().unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let (done_sender, done_receiver) = mpsc::channel();

        let observer = {
            let runtime = Arc::clone(&runtime);
            let barrier = Arc::clone(&barrier);
            let done_sender = done_sender.clone();
            thread::spawn(move || {
                let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
                let snapshot = dashboard(at(1_000 + 7 * 24 * 60 * 60));
                barrier.wait();
                for _ in 0..2_000 {
                    runtime.observe_dashboard(true, &identity, &snapshot, at(951));
                }
                done_sender.send("observe").unwrap();
            })
        };
        let preflight = {
            let runtime = Arc::clone(&runtime);
            let barrier = Arc::clone(&barrier);
            let done_sender = done_sender.clone();
            let attempt = attempt.clone();
            thread::spawn(move || {
                barrier.wait();
                for _ in 0..20_000 {
                    let _ = runtime.preflight_decision(&attempt);
                }
                done_sender.send("preflight").unwrap();
            })
        };
        let manual = {
            let runtime = Arc::clone(&runtime);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                for _ in 0..20_000 {
                    assert!(runtime.manual_account_guard().is_some());
                }
                done_sender.send("manual").unwrap();
            })
        };

        let mut completed = Vec::new();
        for _ in 0..3 {
            completed.push(
                done_receiver
                    .recv_timeout(Duration::from_secs(10))
                    .expect("并发锁顺序不应卡住"),
            );
        }
        completed.sort_unstable();
        assert_eq!(completed, ["manual", "observe", "preflight"]);
        observer.join().unwrap();
        preflight.join().unwrap();
        manual.join().unwrap();
        cleanup(&path);
    }
}
