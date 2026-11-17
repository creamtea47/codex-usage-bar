use crate::app::state::normalize_general_settings;
use crate::domain::settings::Language;
use crate::domain::settings::NotificationSettings;
use crate::domain::settings::Settings;

#[test]
fn general_settings_cannot_bypass_side_effect_commands() {
    let current = Settings {
        notifications: NotificationSettings {
            enabled: false,
            ..NotificationSettings::default()
        },
        history_enabled: true,
        ..Settings::default()
    };
    let requested = Settings {
        notifications: NotificationSettings {
            enabled: true,
            low_quota_threshold_percent: 42,
            ..NotificationSettings::default()
        },
        history_enabled: false,
        language: Language::En,
        ..Settings::default()
    };

    let normalized = normalize_general_settings(requested, &current);
    assert!(!normalized.notifications.enabled);
    assert!(normalized.history_enabled);
    assert_eq!(normalized.notifications.low_quota_threshold_percent, 42);
    assert_eq!(normalized.language, Language::En);
}
