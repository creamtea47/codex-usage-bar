use super::*;
use crate::domain::settings::Language;
use crate::domain::settings::MainWindowSizeMode;
use crate::domain::settings::NotificationSettings;
use crate::domain::settings::Settings;
use crate::domain::settings::Theme;

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
    assert!(settings.preferences.notifications.reset_credit_enabled);
    assert!(settings.preferences.history_enabled);
    assert!(settings.preferences.minimize_to_tray_on_close);
    assert!(!settings.preferences.quota_auto_continue_enabled);
}

#[test]
fn legacy_notification_object_defaults_reset_credit_alert_to_on() {
    let mut legacy = serde_json::to_value(StoredSettings::default()).unwrap();
    legacy["preferences"]["notifications"]
        .as_object_mut()
        .unwrap()
        .remove("resetCreditEnabled");

    let loaded: StoredSettings = serde_json::from_value(legacy).unwrap();
    assert!(loaded.preferences.notifications.reset_credit_enabled);
    let mut old = serde_json::to_value(&loaded).unwrap();
    let flags = old["preferences"]["notifications"].as_object_mut().unwrap();
    flags.insert("resetCreditEnabled".into(), false.into());
    flags.remove("extraCreditEnabled");
    flags.remove("connectionEnabled");
    let old: StoredSettings = serde_json::from_value(old).unwrap();
    assert!(!old.preferences.notifications.reset_credit_enabled);
    assert!(
        old.preferences.notifications.extra_credit_enabled
            && old.preferences.notifications.connection_enabled
    );
    assert!(!old.preferences.notifications.enabled);
    assert!(loaded.preferences.notifications.reset_enabled);
    assert!(loaded.preferences.notifications.low_quota_enabled);
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
