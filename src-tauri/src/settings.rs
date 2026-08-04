use crate::models::{Settings, StoredSettings, WindowPlacement};
use std::{
    fs, io,
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
    fs::write(&temporary_path, content)?;
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(temporary_path, path)
}

pub fn normalize_stored_settings(mut settings: StoredSettings) -> StoredSettings {
    settings.preferences = settings.preferences.normalized();
    settings.window_placement = settings.window_placement.filter(|placement| {
        (340..=2400).contains(&placement.width) && (260..=1800).contains(&placement.height)
    });
    settings
}

pub fn update_preferences(stored: &mut StoredSettings, preferences: Settings) {
    stored.preferences = preferences.normalized();
}

pub fn update_window_placement(stored: &mut StoredSettings, placement: WindowPlacement) {
    stored.window_placement = Some(placement);
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
mod tests {
    use super::*;
    use crate::models::{Settings, Theme};
    use std::{env, thread, time::SystemTime};

    #[test]
    fn normalizes_invalid_refresh_interval_and_geometry() {
        let result = normalize_stored_settings(StoredSettings {
            preferences: Settings {
                refresh_interval_seconds: 2,
                ..Settings::default()
            },
            window_placement: Some(WindowPlacement {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }),
        });
        assert_eq!(result.preferences.refresh_interval_seconds, 60);
        assert_eq!(result.window_placement, None);
        assert_eq!(result.preferences.theme, Theme::System);
    }

    #[test]
    fn preserves_every_supported_refresh_interval() {
        for interval in [60, 180, 300, 600, 1800] {
            let result = normalize_stored_settings(StoredSettings {
                preferences: Settings {
                    refresh_interval_seconds: interval,
                    ..Settings::default()
                },
                window_placement: None,
            });
            assert_eq!(result.preferences.refresh_interval_seconds, interval);
        }
    }

    #[test]
    fn removes_files_older_than_the_requested_log_retention() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!("codex-usage-bar-log-cleanup-{nonce}"));
        fs::create_dir_all(&directory).unwrap();
        let old_log = directory.join("codex-usage-bar_2000-01-01.log");
        fs::write(&old_log, "safe test log").unwrap();

        // 标准库不提供设置修改时间的跨平台接口；短暂等待后用零保留期验证清理分支。
        thread::sleep(Duration::from_millis(10));
        cleanup_logs(&directory, Duration::ZERO);
        assert!(!old_log.exists());
        fs::remove_dir_all(directory).unwrap();
    }
}
