use crate::app::notifications::map_notification_permission;
use crate::app::windows::compact_height;
use crate::app::windows::reliable_advice_count;
use crate::app::windows::settings_window_title;
use crate::app::windows::MAIN_ADVICE_LINE_HEIGHT;
use crate::app::windows::MAIN_ADVICE_SECTION_BASE_HEIGHT;
use crate::app::windows::MAIN_ADVICE_VISIBLE_LINE_LIMIT;
use crate::app::windows::MAIN_COMPACT_HEIGHT;
use crate::app::windows::MAIN_COMPACT_HEIGHT_PER_EXTRA_WINDOW;
use crate::app::windows::MAIN_COMPACT_MAX_HEIGHT;
use crate::domain::settings::Language;
use crate::domain::settings::NotificationPermission;
use crate::domain::usage::DashboardSnapshot;
use crate::domain::usage::ForecastStatus as ModelForecastStatus;
use crate::domain::usage::QuotaFallbackLabel;
use crate::domain::usage::QuotaForecast;
use chrono::DateTime;
use chrono::Duration as ChronoDuration;
use chrono::Utc;
use tauri::plugin::PermissionState;

#[test]
fn compact_height_grows_by_window_and_stops_at_maximum() {
    assert_eq!(compact_height(0, 0), MAIN_COMPACT_HEIGHT);
    assert_eq!(compact_height(1, 0), MAIN_COMPACT_HEIGHT);
    assert_eq!(
        compact_height(2, 0),
        MAIN_COMPACT_HEIGHT + MAIN_COMPACT_HEIGHT_PER_EXTRA_WINDOW
    );
    assert_eq!(
        compact_height(1, 1),
        MAIN_COMPACT_HEIGHT + MAIN_ADVICE_SECTION_BASE_HEIGHT + MAIN_ADVICE_LINE_HEIGHT
    );
    assert_eq!(
        compact_height(1, 4),
        MAIN_COMPACT_HEIGHT
            + MAIN_ADVICE_SECTION_BASE_HEIGHT
            + MAIN_ADVICE_LINE_HEIGHT * MAIN_ADVICE_VISIBLE_LINE_LIMIT as u32
    );
    assert_eq!(compact_height(9, 3), MAIN_COMPACT_MAX_HEIGHT);
}

#[test]
fn reliable_advice_requires_a_long_valid_period_and_supported_forecast() {
    let start_at = DateTime::parse_from_rfc3339("2030-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let reset_at = DateTime::parse_from_rfc3339("2030-01-08T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let reliable = crate::domain::usage::QuotaWindow {
        id: "weekly".to_owned(),
        label: None,
        fallback_label: QuotaFallbackLabel::Weekly,
        remaining_percent: 75,
        used_percent: 25,
        window_seconds: 7 * 24 * 60 * 60,
        reset_at: Some(reset_at),
        reset_after_seconds: 24 * 60 * 60,
        start_at: Some(start_at),
        show_pace_marker: true,
        forecast: Some(QuotaForecast {
            status: ModelForecastStatus::LastsUntilReset,
            exhausts_at: None,
            sample_count: 6,
            observed_span_seconds: 3_600,
            consumed_percent: 4.0,
        }),
    };
    let mut snapshot = DashboardSnapshot {
        quota_windows: vec![reliable.clone()],
        ..DashboardSnapshot::default()
    };
    assert_eq!(reliable_advice_count(&snapshot), 1);

    snapshot
        .quota_windows
        .push(crate::domain::usage::QuotaWindow {
            forecast: Some(QuotaForecast {
                status: ModelForecastStatus::ExhaustsBeforeReset,
                exhausts_at: Some(start_at + ChronoDuration::days(2)),
                sample_count: 6,
                observed_span_seconds: 3_600,
                consumed_percent: 4.0,
            }),
            ..reliable.clone()
        });
    assert_eq!(reliable_advice_count(&snapshot), 2);

    snapshot.quota_windows.extend([
        crate::domain::usage::QuotaWindow {
            show_pace_marker: false,
            ..reliable.clone()
        },
        crate::domain::usage::QuotaWindow {
            reset_at: Some(start_at),
            ..reliable.clone()
        },
        crate::domain::usage::QuotaWindow {
            forecast: Some(QuotaForecast {
                status: ModelForecastStatus::ExhaustsBeforeReset,
                exhausts_at: None,
                sample_count: 6,
                observed_span_seconds: 3_600,
                consumed_percent: 4.0,
            }),
            ..reliable
        },
    ]);
    assert_eq!(reliable_advice_count(&snapshot), 2);
}

#[test]
fn settings_native_title_follows_language_resolution() {
    assert_eq!(settings_window_title(Language::ZhCn), "CodexUsageBar 设置");
    assert_eq!(
        settings_window_title(Language::En),
        "CodexUsageBar Settings"
    );
    assert_eq!(
        map_notification_permission(PermissionState::Denied),
        NotificationPermission::Denied
    );
}
