use crate::domain::settings::MainWindowSizeMode;
use crate::domain::settings::Settings;
use crate::domain::settings::StoredSettings;
use crate::domain::settings::WindowPlacement;
use std::{
    fs,
    io::{self, Write},
    path::Path,
    time::{Duration, SystemTime},
};

pub fn load_settings(path: &Path) -> StoredSettings {
    let Ok(contents) = fs::read_to_string(path) else {
        return StoredSettings::default();
    };
    serde_json::from_str::<StoredSettings>(&contents)
        .map(normalize_stored_settings)
        .unwrap_or_default()
}

pub fn save_settings(path: &Path, settings: &StoredSettings) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary_path = path.with_extension("json.tmp");
    let content = serde_json::to_vec_pretty(settings).map_err(io::Error::other)?;
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

pub fn normalize_stored_settings(mut settings: StoredSettings) -> StoredSettings {
    settings.preferences = settings.preferences.normalized();
    settings.window_placement = settings.window_placement.filter(valid_main_placement);
    settings.settings_window_placement = settings
        .settings_window_placement
        .filter(valid_settings_placement);
    settings
}

pub fn update_preferences(stored: &mut StoredSettings, preferences: Settings) {
    stored.preferences = preferences.normalized();
}

/// 旧 JSON 的 `windowPlacement` 继续代表主悬浮卡，保证升级后原位置可恢复。
pub fn update_main_window_placement(stored: &mut StoredSettings, placement: WindowPlacement) {
    stored.window_placement = Some(placement);
}

pub fn update_settings_window_placement(stored: &mut StoredSettings, placement: WindowPlacement) {
    stored.settings_window_placement = Some(placement);
}

/// 自动尺寸模式会跟随当前紧凑基准高度，只修改高度，保留用户的位置和宽度。
/// 这样更新紧凑基准时可消除无效留白；用户手动调整过的尺寸则绝不被覆盖。
pub fn apply_compact_layout_migration(stored: &mut StoredSettings, compact_height: u32) -> bool {
    if stored.main_window_size_mode == MainWindowSizeMode::Manual {
        if stored.compact_layout_migration_completed {
            return false;
        }
        // 记录已检查即可，避免每次启动都重试；手动尺寸始终优先。
        stored.compact_layout_migration_completed = true;
        return true;
    }

    let mut changed = !stored.compact_layout_migration_completed;
    if let Some(placement) = stored.window_placement.as_mut() {
        if placement.height != compact_height {
            placement.height = compact_height;
            changed = true;
        }
    }
    stored.compact_layout_migration_completed = true;
    changed
}

fn valid_main_placement(placement: &WindowPlacement) -> bool {
    (340..=2400).contains(&placement.width) && (260..=1800).contains(&placement.height)
}

fn valid_settings_placement(placement: &WindowPlacement) -> bool {
    (620..=2400).contains(&placement.width) && (420..=1800).contains(&placement.height)
}

/// 日志可用于排障，但保留时间有限，避免无关运行信息无限积累。
pub fn cleanup_logs(log_directory: &Path, max_age: Duration) {
    let Ok(entries) = fs::read_dir(log_directory) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if now.duration_since(modified).is_ok_and(|age| age > max_age) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests;
