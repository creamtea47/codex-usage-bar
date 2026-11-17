use super::*;
pub(crate) fn decode_history(contents: &[u8], now: DateTime<Utc>) -> LoadedUsageHistory {
    let value: serde_json::Value = match serde_json::from_slice(contents) {
        Ok(value) => value,
        Err(_) => return recovered_history(HistoryStorageStatus::RecoveredCorrupt),
    };
    let Some(version) = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
    else {
        return recovered_history(HistoryStorageStatus::RecoveredCorrupt);
    };
    let (mut history, migrated) = match version {
        1 => {
            let legacy = match serde_json::from_value::<UsageHistoryV1>(value) {
                Ok(legacy) if legacy.schema_version == 1 => legacy,
                _ => return recovered_history(HistoryStorageStatus::RecoveredCorrupt),
            };
            let history = UsageHistory {
                schema_version: USAGE_HISTORY_SCHEMA_VERSION,
                salt: legacy.salt,
                account_fingerprint: legacy.account_fingerprint,
                streams: legacy.streams,
                accounts: Vec::new(),
            };
            if !history.validate() {
                return recovered_history(HistoryStorageStatus::RecoveredCorrupt);
            }
            log::info!(
                "本地趋势历史已加载旧版待迁移数据：from_schema=1，to_schema=2，samples={}。",
                history.sample_count()
            );
            (history, true)
        }
        version if version == USAGE_HISTORY_SCHEMA_VERSION as u64 => {
            let history = match serde_json::from_value::<UsageHistory>(value) {
                Ok(history) if history.validate() => history,
                _ => return recovered_history(HistoryStorageStatus::RecoveredCorrupt),
            };
            (history, false)
        }
        _ => return recovered_history(HistoryStorageStatus::RecoveredUnsupported),
    };
    let removed = history.compact_at(now);
    LoadedUsageHistory {
        history,
        status: HistoryStorageStatus::Ready,
        needs_rewrite: migrated || removed > 0,
    }
}

pub(crate) fn recovered_history(status: HistoryStorageStatus) -> LoadedUsageHistory {
    LoadedUsageHistory {
        history: UsageHistory::new_random(),
        status,
        needs_rewrite: status != HistoryStorageStatus::Missing,
    }
}

impl UsageHistory {
    pub(crate) fn has_selected_account(&self) -> bool {
        self.account_fingerprint.is_some()
    }
    pub(crate) fn owns_legacy_backup(&self, bytes: &[u8]) -> Result<bool, HistoryPersistenceError> {
        let legacy: UsageHistoryV1 =
            serde_json::from_slice(bytes).map_err(|_| HistoryPersistenceError::Storage)?;
        Ok(self.account_fingerprint.is_some()
            && legacy.account_fingerprint == self.account_fingerprint)
    }
}
