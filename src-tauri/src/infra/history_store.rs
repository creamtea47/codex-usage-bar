use crate::domain::history::{
    decode_history, recovered_history, HistoryPersistenceError, HistoryStorageStatus,
    LoadedUsageHistory, UsageHistory,
};
use chrono::{DateTime, Utc};
use std::{fs, io, path::Path};
pub fn load_history(path: &Path) -> LoadedUsageHistory {
    load_history_at(path, Utc::now())
}

/// 所有文件错误都降级为空历史，不影响应用启动，也不把路径或底层错误带入返回值。
pub fn load_history_at(path: &Path, now: DateTime<Utc>) -> LoadedUsageHistory {
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return recovered_history(HistoryStorageStatus::Missing)
        }
        Err(_) => return recovered_history(HistoryStorageStatus::Unavailable),
    };
    decode_history(&contents, now)
}

pub fn save_history(path: &Path, history: &UsageHistory) -> Result<(), HistoryPersistenceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| HistoryPersistenceError::Storage)?;
    }
    let content =
        serde_json::to_vec_pretty(history).map_err(|_| HistoryPersistenceError::Serialize)?;
    let temporary_path = path.with_extension("json.tmp");
    let legacy_backup = legacy_backup_for(path)?;
    fs::write(&temporary_path, content).map_err(|_| HistoryPersistenceError::Storage)?;
    if path.exists() {
        fs::remove_file(path).map_err(|_| HistoryPersistenceError::Storage)?;
    }
    if fs::rename(&temporary_path, path).is_err() {
        let _ = fs::remove_file(&temporary_path);
        if !path.exists() {
            if let Some(backup) = legacy_backup {
                let _ = fs::copy(backup, path);
            }
        }
        return Err(HistoryPersistenceError::Storage);
    }
    Ok(())
}

/// 损坏恢复必须先丢弃未知目标；删除失败只返回稳定存储类别，不暴露路径。
pub fn clear_history_storage(path: &Path) -> Result<(), HistoryPersistenceError> {
    let temporary_path = path.with_extension("json.tmp");
    let mut failed = false;
    for candidate in [&temporary_path, path] {
        if let Err(error) = fs::remove_file(candidate) {
            if error.kind() != io::ErrorKind::NotFound {
                failed = true;
            }
        }
    }
    if failed {
        Err(HistoryPersistenceError::Storage)
    } else {
        Ok(())
    }
}

/// 清除当前账号时同步移除同账号的 v1 迁移备份，避免“永久删除”后仍可恢复旧样本。
pub fn clear_legacy_backup_for_current_account(
    path: &Path,
    history: &UsageHistory,
) -> Result<(), HistoryPersistenceError> {
    if !history.has_selected_account() {
        return Ok(());
    }
    let backup = path.with_extension("v1.backup.json");
    let contents = match fs::read(&backup) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(HistoryPersistenceError::Storage),
    };
    if history.owns_legacy_backup(&contents)? {
        fs::remove_file(backup).map_err(|_| HistoryPersistenceError::Storage)?;
    }
    Ok(())
}

/// schema v1 首次改写前保留一次原文件；后续保存失败时可恢复旧目标。
fn legacy_backup_for(path: &Path) -> Result<Option<std::path::PathBuf>, HistoryPersistenceError> {
    let Ok(contents) = fs::read(path) else {
        return Ok(None);
    };
    let is_v1 = serde_json::from_slice::<serde_json::Value>(&contents)
        .ok()
        .and_then(|value| {
            value
                .get("schemaVersion")
                .and_then(serde_json::Value::as_u64)
        })
        == Some(1);
    if !is_v1 {
        return Ok(None);
    }
    let backup = path.with_extension("v1.backup.json");
    if !backup.exists() {
        fs::write(&backup, contents).map_err(|_| HistoryPersistenceError::Storage)?;
        log::info!("本地趋势历史迁移备份已创建：schema=1。");
    }
    Ok(Some(backup))
}
