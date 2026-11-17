use crate::domain::settings::Settings;
use crate::domain::settings::Theme;

#[test]
fn only_supported_refresh_intervals_are_accepted() {
    let result = Settings {
        always_on_top: true,
        lock_position: false,
        refresh_interval_seconds: 99,
        theme: Theme::Dark,
        auto_check_updates: false,
        ..Settings::default()
    }
    .normalized();
    assert_eq!(result.refresh_interval_seconds, 60);
    assert!(result.always_on_top);
    assert!(!result.auto_check_updates);
}
