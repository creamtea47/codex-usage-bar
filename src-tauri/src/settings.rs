use crate::models::{MainWindowSizeMode, Settings, StoredSettings, WindowPlacement};
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
mod tests {
    use super::*;
    use crate::models::{Language, MainWindowSizeMode, NotificationSettings, Settings, Theme};
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
            settings_window_placement: Some(WindowPlacement {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }),
            compact_layout_migration_completed: false,
            main_window_size_mode: MainWindowSizeMode::Auto,
        });
        assert_eq!(result.preferences.refresh_interval_seconds, 60);
        assert_eq!(result.window_placement, None);
        assert_eq!(result.settings_window_placement, None);
        assert_eq!(result.preferences.theme, Theme::System);
        assert!(result.preferences.auto_check_updates);
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
                settings_window_placement: None,
                compact_layout_migration_completed: false,
                main_window_size_mode: MainWindowSizeMode::Auto,
            });
            assert_eq!(result.preferences.refresh_interval_seconds, interval);
        }
    }

    #[test]
    fn migrates_auto_main_window_without_losing_placement() {
        let mut settings = StoredSettings {
            preferences: Settings::default(),
            window_placement: Some(WindowPlacement {
                x: 120,
                y: 240,
                width: 720,
                height: 420,
            }),
            settings_window_placement: None,
            compact_layout_migration_completed: false,
            main_window_size_mode: MainWindowSizeMode::Auto,
        };

        assert!(apply_compact_layout_migration(&mut settings, 260));
        assert_eq!(settings.window_placement.as_ref().unwrap().height, 260);
        assert_eq!(settings.main_window_size_mode, MainWindowSizeMode::Auto);
        assert!(settings.compact_layout_migration_completed);
        assert!(!apply_compact_layout_migration(&mut settings, 260));
    }

    #[test]
    fn updates_an_automatic_card_when_the_compact_height_changes() {
        let mut settings = StoredSettings {
            preferences: Settings::default(),
            window_placement: Some(WindowPlacement {
                x: 120,
                y: 240,
                width: 460,
                height: 330,
            }),
            settings_window_placement: None,
            compact_layout_migration_completed: true,
            main_window_size_mode: MainWindowSizeMode::Auto,
        };

        assert!(apply_compact_layout_migration(&mut settings, 260));
        assert_eq!(settings.window_placement.as_ref().unwrap().height, 260);

        settings.main_window_size_mode = MainWindowSizeMode::Manual;
        settings.window_placement.as_mut().unwrap().height = 480;
        assert!(!apply_compact_layout_migration(&mut settings, 260));
        assert_eq!(settings.window_placement.as_ref().unwrap().height, 480);
    }

    #[test]
    fn reads_legacy_window_placement_without_settings_window_field() {
        let legacy = r#"{
          "preferences": {"alwaysOnTop": false, "lockPosition": false, "refreshIntervalSeconds": 60, "theme": "system"},
          "windowPlacement": {"x": 120, "y": 240, "width": 460, "height": 420}
        }"#;
        let settings = normalize_stored_settings(serde_json::from_str(legacy).unwrap());
        assert_eq!(settings.window_placement.as_ref().unwrap().x, 120);
        assert_eq!(settings.settings_window_placement, None);
        assert!(!settings.compact_layout_migration_completed);
        // 旧设置文件没有此字段时保持新版默认，避免升级后意外关闭自动检查。
        assert!(settings.preferences.auto_check_updates);
        assert_eq!(settings.preferences.language, Language::System);
        assert_eq!(
            settings.preferences.notifications,
            NotificationSettings::default()
        );
        assert!(settings.preferences.history_enabled);
        assert!(settings.preferences.minimize_to_tray_on_close);
        assert!(!settings.preferences.quota_auto_continue_enabled);
    }

    #[test]
    fn settings_replace_atomically_without_leaving_a_temporary_file() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("codex-usage-bar-settings-{nonce}.json"));
        let mut stored = StoredSettings::default();
        save_settings(&path, &stored).unwrap();
        stored.preferences.minimize_to_tray_on_close = false;
        stored.preferences.quota_auto_continue_enabled = true;
        save_settings(&path, &stored).unwrap();

        let loaded = load_settings(&path);
        assert!(!loaded.preferences.minimize_to_tray_on_close);
        assert!(loaded.preferences.quota_auto_continue_enabled);
        assert!(!path.with_extension("json.tmp").exists());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn normalizes_notification_thresholds_and_quiet_hours() {
        let result = Settings {
            notifications: NotificationSettings {
                low_quota_threshold_percent: 101,
                pace_deficit_threshold_percent: 255,
                quiet_hours_start: "24:00".to_owned(),
                quiet_hours_end: "8:00".to_owned(),
                ..NotificationSettings::default()
            },
            ..Settings::default()
        }
        .normalized();

        assert_eq!(result.notifications.low_quota_threshold_percent, 100);
        assert_eq!(result.notifications.pace_deficit_threshold_percent, 100);
        assert_eq!(result.notifications.quiet_hours_start, "22:00");
        assert_eq!(result.notifications.quiet_hours_end, "08:00");
        assert!(!result.notifications.enabled);
    }

    #[test]
    fn keeps_main_and_settings_window_geometry_independent() {
        let mut settings = StoredSettings::default();
        update_main_window_placement(
            &mut settings,
            WindowPlacement {
                x: 10,
                y: 20,
                width: 460,
                height: 330,
            },
        );
        update_settings_window_placement(
            &mut settings,
            WindowPlacement {
                x: 30,
                y: 40,
                width: 760,
                height: 560,
            },
        );
        assert_eq!(settings.window_placement.as_ref().unwrap().width, 460);
        assert_eq!(
            settings.settings_window_placement.as_ref().unwrap().width,
            760
        );
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
