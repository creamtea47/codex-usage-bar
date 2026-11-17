mod delivery;
mod observation;
pub(crate) use crate::domain::continuation::*;
use crate::domain::history::account_fingerprint;
use crate::domain::history::AccountIdentity;
use crate::domain::quota_reset::NotificationDisposition;
use crate::domain::quota_reset::QuotaResetEvent;
use crate::domain::quota_reset::QuotaResetReason;
use crate::domain::usage::DashboardSnapshot;
use crate::infra::auth::read_auth_credentials;
use crate::infra::auth::AuthCredentials;
use crate::infra::auth::AuthError;
use crate::infra::continuation_store::{load_runtime_state, save_runtime_state};
use crate::infra::quota_audit::QuotaAuditAction;
use crate::infra::quota_audit::QuotaAuditLog;
use crate::infra::quota_audit::QuotaAuditRecord;
use crate::infra::usage::UsageAccountIdentity;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
#[cfg(test)]
use reqwest::Client;
use reqwest::StatusCode;
use serde::Serialize;
use serde_json::Value;
#[cfg(test)]
use std::time::Duration;
use std::{
    path::{Path, PathBuf},
    sync::{Mutex as StdMutex, MutexGuard as StdMutexGuard},
};
use tokio::sync::{watch, Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};

pub const STATE_FILE_NAME: &str = "quota-auto-continue.json";
const MODEL_MANIFEST_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/models";
const RESPONSES_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/responses";
const CODEX_CLIENT_VERSION: &str = "0.146.0";
const CODEX_USER_AGENT: &str = "codex_cli_rs/0.146.0 (Ubuntu 22.4.0; x86_64) xterm-256color";

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
mod protocol;
pub use protocol::ModelOption;
use protocol::QuotaAutoContinueClient;
#[cfg(test)]
use protocol::{build_greeting_payload, parse_sse_line, select_preferred_model, SseSignal};

#[derive(Debug)]
pub struct SendFailure {
    pub code: QuotaAutoContinueErrorCode,
    pub model: Option<String>,
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
    pub fn bind_account(
        &mut self,
        account: std::sync::Arc<crate::infra::accounts::ManagedAccount>,
    ) {
        self.client.account = Some(account);
    }

    pub async fn models(&self) -> Result<Vec<ModelOption>, QuotaAutoContinueErrorCode> {
        self.client.models().await
    }

    pub fn new(path: PathBuf) -> Result<Self, QuotaAutoContinueErrorCode> {
        Self::new_with_client(path, QuotaAutoContinueClient::new()?)
    }

    fn new_with_client(
        path: PathBuf,
        client: QuotaAutoContinueClient,
    ) -> Result<Self, QuotaAutoContinueErrorCode> {
        let (mut persisted, migrated) = load_runtime_state(&path);
        // 崩溃可能发生在 POST 发出后、结果落盘前；不能把 Running 当作可安全重试。
        let interrupted_send = persisted.active_cycle.as_mut().is_some_and(|cycle| {
            if cycle.phase != QuotaAutoContinuePhase::Running {
                return false;
            }
            cycle.request_completed = true;
            cycle.phase = QuotaAutoContinuePhase::Failed;
            true
        });
        if interrupted_send {
            if let Some(result) = persisted.last_automatic_result.as_mut() {
                result.error_code = Some(QuotaAutoContinueErrorCode::Network);
                result
                    .details
                    .get_or_insert_with(ResponseDetails::default)
                    .delivery_uncertain = true;
            }
        }
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
        if (migrated || interrupted_claim.is_some() || interrupted_send)
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

    pub fn status(&self, enabled: bool, now: DateTime<Utc>) -> QuotaAutoContinueStatus {
        let selected_model = self.selected_model().clone();
        let persisted = self.persisted();
        let latest = latest_compat_result(&persisted);
        QuotaAutoContinueStatus {
            account_id: self.client.account.as_ref().map(|a| a.id.clone()),
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

#[cfg(test)]
mod tests;
