use crate::quota_reset::QuotaResetReason;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub const AUDIT_FILE_PREFIX: &str = "quota-audit-";
pub const AUDIT_FILE_SUFFIX: &str = ".jsonl";
pub const AUDIT_RETENTION_DAYS: i64 = 14;

/// 审计动作与验收链名称保持一致，禁止写入网络响应正文或认证信息。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum QuotaAuditAction {
    StateMigrated,
    ResetConfirmed,
    NotificationClaimed,
    NotificationQueued,
    NotificationFailed,
    NotificationSuppressed,
    Slot0Claimed,
    RetrySlotClaimed,
    #[serde(rename = "response.completed")]
    ResponseCompleted,
    AutomaticFailed,
    ManualSatisfied,
    CycleMissed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum QuotaAuditLevel {
    Info,
    Warning,
}

/// 单条 JSONL 审计记录。上下文均为额度整数、UTC 时间、脱敏 ID 或稳定枚举代码。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaAuditRecord {
    pub timestamp: DateTime<Utc>,
    pub level: QuotaAuditLevel,
    pub action: QuotaAuditAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_reason: Option<QuotaResetReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot_index: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_remaining_percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_remaining_percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_reset_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_reset_at: Option<DateTime<Utc>>,
}

impl QuotaAuditRecord {
    pub fn info(timestamp: DateTime<Utc>, action: QuotaAuditAction) -> Self {
        Self {
            timestamp,
            level: QuotaAuditLevel::Info,
            action,
            event_id: None,
            generation_id: None,
            trigger_reason: None,
            slot_index: None,
            error_code: None,
            old_remaining_percent: None,
            new_remaining_percent: None,
            old_reset_at: None,
            new_reset_at: None,
        }
    }

    pub fn warning(timestamp: DateTime<Utc>, action: QuotaAuditAction) -> Self {
        Self {
            level: QuotaAuditLevel::Warning,
            ..Self::info(timestamp, action)
        }
    }

    pub fn with_event(
        mut self,
        event_id: impl Into<String>,
        generation_id: impl Into<String>,
        reason: QuotaResetReason,
    ) -> Self {
        self.event_id = Some(event_id.into());
        self.generation_id = Some(generation_id.into());
        self.trigger_reason = Some(reason);
        self
    }
}

/// 按 UTC 日滚动的 JSONL 写入器。审计失败不得阻断额度排期。
#[derive(Debug, Clone)]
pub struct QuotaAuditLog {
    directory: PathBuf,
    io_guard: Arc<Mutex<()>>,
}

impl QuotaAuditLog {
    pub fn new(directory: PathBuf, now: DateTime<Utc>) -> Self {
        let value = Self {
            directory,
            io_guard: Arc::new(Mutex::new(())),
        };
        let _ = value.prune(now);
        value
    }

    pub fn path_for(&self, timestamp: DateTime<Utc>) -> PathBuf {
        self.directory.join(format!(
            "{AUDIT_FILE_PREFIX}{}{AUDIT_FILE_SUFFIX}",
            timestamp.format("%Y-%m-%d")
        ))
    }

    pub fn append(&self, record: &QuotaAuditRecord) -> io::Result<()> {
        // 通知线程与自动接续线程可能同时落同一日文件；独立 I/O 锁避免 JSON 与换行交错，
        // 且调用方在进入这里前已释放 scheduler state 锁。
        let _guard = self
            .io_guard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        fs::create_dir_all(&self.directory)?;
        // 长期不重启也必须持续满足 14 天留存；清理失败不应阻断当天审计写入。
        let _ = self.prune_unlocked(record.timestamp);
        let path = self.path_for(record.timestamp);
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        let mut line = serde_json::to_vec(record).map_err(io::Error::other)?;
        line.push(b'\n');
        file.write_all(&line)?;
        file.flush()?;
        file.sync_data()
    }

    /// 以 UTC 日为单位保留今天及之前共 14 个日期文件，不解析或改写在保留期内的记录。
    pub fn prune(&self, now: DateTime<Utc>) -> io::Result<usize> {
        let _guard = self
            .io_guard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.prune_unlocked(now)
    }

    fn prune_unlocked(&self, now: DateTime<Utc>) -> io::Result<usize> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error),
        };
        let oldest_retained = now.date_naive() - Duration::days(AUDIT_RETENTION_DAYS - 1);
        let mut removed = 0_usize;
        for entry in entries {
            let entry = entry?;
            let Some(date) = audit_file_date(&entry.file_name().to_string_lossy()) else {
                continue;
            };
            if date < oldest_retained {
                fs::remove_file(entry.path())?;
                removed = removed.saturating_add(1);
            }
        }
        Ok(removed)
    }
}

fn audit_file_date(file_name: &str) -> Option<NaiveDate> {
    let value = file_name
        .strip_prefix(AUDIT_FILE_PREFIX)?
        .strip_suffix(AUDIT_FILE_SUFFIX)?;
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, thread, time::SystemTime};

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, 0).unwrap()
    }

    fn temp_directory(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("codex-quota-audit-{name}-{nonce}"))
    }

    #[test]
    fn appends_redacted_records_to_utc_daily_file() {
        let directory = temp_directory("append");
        let timestamp = DateTime::parse_from_rfc3339("2026-08-09T23:59:59Z")
            .unwrap()
            .with_timezone(&Utc);
        let audit = QuotaAuditLog::new(directory.clone(), timestamp);
        let mut record = QuotaAuditRecord::info(timestamp, QuotaAuditAction::ResetConfirmed)
            .with_event("event", "generation", QuotaResetReason::QuotaRecovered);
        record.old_remaining_percent = Some(98);
        record.new_remaining_percent = Some(100);
        audit.append(&record).unwrap();

        let path = directory.join("quota-audit-2026-08-09.jsonl");
        let content = fs::read_to_string(path).unwrap();
        assert_eq!(content.lines().count(), 1);
        assert_eq!(
            serde_json::from_str::<QuotaAuditRecord>(content.trim()).unwrap(),
            record
        );
        for forbidden in ["access_token", "authorization", "account_id", "model"] {
            assert!(!content.to_ascii_lowercase().contains(forbidden));
        }
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn removes_only_whole_daily_files_older_than_fourteen_days() {
        let directory = temp_directory("retention");
        fs::create_dir_all(&directory).unwrap();
        for name in [
            "quota-audit-2026-07-26.jsonl",
            "quota-audit-2026-07-27.jsonl",
            "quota-audit-2026-08-09.jsonl",
            "unrelated.jsonl",
        ] {
            fs::write(directory.join(name), b"{}\n").unwrap();
        }
        let now = DateTime::parse_from_rfc3339("2026-08-09T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let audit = QuotaAuditLog::new(directory.clone(), now);
        assert!(!directory.join("quota-audit-2026-07-26.jsonl").exists());
        assert!(directory.join("quota-audit-2026-07-27.jsonl").exists());
        assert!(directory.join("quota-audit-2026-08-09.jsonl").exists());
        assert!(directory.join("unrelated.jsonl").exists());
        assert_eq!(audit.prune(at(2_000)).unwrap(), 0);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn append_prunes_during_long_running_process() {
        let directory = temp_directory("long-running");
        let started_at = DateTime::parse_from_rfc3339("2026-07-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let audit = QuotaAuditLog::new(directory.clone(), started_at);
        audit
            .append(&QuotaAuditRecord::info(
                started_at,
                QuotaAuditAction::StateMigrated,
            ))
            .unwrap();
        let later = DateTime::parse_from_rfc3339("2026-08-09T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        audit
            .append(&QuotaAuditRecord::info(
                later,
                QuotaAuditAction::ResetConfirmed,
            ))
            .unwrap();

        assert!(!directory.join("quota-audit-2026-07-01.jsonl").exists());
        assert!(directory.join("quota-audit-2026-08-09.jsonl").exists());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn concurrent_appends_keep_every_jsonl_record_parseable() {
        let directory = temp_directory("concurrent");
        let timestamp = DateTime::parse_from_rfc3339("2026-08-09T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let audit = QuotaAuditLog::new(directory.clone(), timestamp);
        let workers = (0..8)
            .map(|worker| {
                let audit = audit.clone();
                thread::spawn(move || {
                    for index in 0..10 {
                        let record =
                            QuotaAuditRecord::info(timestamp, QuotaAuditAction::NotificationQueued)
                                .with_event(
                                    format!("event-{worker}-{index}"),
                                    "generation",
                                    QuotaResetReason::QuotaRecovered,
                                );
                        audit.append(&record).unwrap();
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }

        let content = fs::read_to_string(audit.path_for(timestamp)).unwrap();
        let records = content
            .lines()
            .map(|line| serde_json::from_str::<QuotaAuditRecord>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 80);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn response_completed_action_matches_sse_acceptance_literal() {
        assert_eq!(
            serde_json::to_value(QuotaAuditAction::ResponseCompleted).unwrap(),
            "response.completed"
        );
        assert_eq!(
            serde_json::to_value(QuotaAuditAction::NotificationClaimed).unwrap(),
            "notificationClaimed"
        );
    }
}
