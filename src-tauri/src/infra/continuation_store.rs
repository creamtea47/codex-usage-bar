use crate::domain::history::{account_fingerprint, AccountIdentity};
use crate::domain::{continuation::*, quota_reset::*};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
pub(crate) fn load_runtime_state(path: &Path) -> (PersistedRuntimeState, bool) {
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

pub(crate) fn migrate_v1(v1: PersistedRuntimeStateV1) -> PersistedRuntimeState {
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
                details: None,
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
                    details: None,
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

pub(crate) fn save_runtime_state(path: &Path, state: &PersistedRuntimeState) -> io::Result<()> {
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
