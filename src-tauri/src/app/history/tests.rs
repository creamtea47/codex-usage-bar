use crate::app::history::HistoryResponseStorageStatus;
use crate::app::history::UsageHistoryResponse;
use crate::app::history::UsageHistorySeriesResponse;
use crate::domain::history::UsageHistoryRequest;
use crate::domain::usage::ForecastStatus as ModelForecastStatus;
use crate::domain::usage::QuotaFallbackLabel;
use crate::domain::usage::QuotaForecast;
use chrono::Duration as ChronoDuration;
use chrono::Utc;

#[test]
fn history_response_serializes_structured_request_and_coverage_within_ipc_budget() {
    let now = Utc::now();
    let request = UsageHistoryRequest::Preset {
        preset: crate::domain::history::UsageHistoryPreset::Days30,
    };
    // Mirror the production structural caps so the size assertion measures
    // UTF-8 bytes for the largest response shape that can cross Tauri IPC.
    let series = (0..16)
        .map(|series_index| UsageHistorySeriesResponse {
            current_reset_at: None,
            window_id: format!("{series_index:064x}"),
            window_seconds: 7 * 24 * 60 * 60,
            fallback_label: QuotaFallbackLabel::Weekly,
            current_remaining_percent: Some(75),
            today_consumed_percent: 25,
            points: (0..1_000)
                .map(|point_index| crate::domain::history::UsageHistoryPoint {
                    suggested_remaining_percent: None,
                    sampled_at: now
                        - ChronoDuration::seconds(i64::from(series_index * 1_000 + point_index)),
                    remaining_percent: 75,
                    break_before: point_index == 0,
                })
                .collect(),
            forecast: QuotaForecast {
                status: ModelForecastStatus::Collecting,
                exhausts_at: None,
                sample_count: 1,
                observed_span_seconds: 0,
                consumed_percent: 0.0,
            },
        })
        .collect();
    let response = UsageHistoryResponse {
        account_id: None,
        request,
        applied_start_at: now - ChronoDuration::days(30),
        applied_end_at_exclusive: now,
        available_start_at: Some(now - ChronoDuration::days(7)),
        available_end_at: Some(now - ChronoDuration::minutes(1)),
        truncated_by_retention: false,
        bucket_seconds: Some(60 * 60),
        history_enabled: true,
        storage_status: HistoryResponseStorageStatus::Ready,
        sample_count: 16_000,
        earliest_sample_at: Some(now - ChronoDuration::days(7)),
        latest_sample_at: Some(now - ChronoDuration::minutes(1)),
        series,
    };

    let bytes = serde_json::to_vec(&response).unwrap();
    assert!(bytes.len() <= 2 * 1024 * 1024);
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        json["request"],
        serde_json::json!({"kind": "preset", "preset": "30d"})
    );
    assert_eq!(json["bucketSeconds"], 3_600);
    assert_eq!(json["truncatedByRetention"], false);
    assert!(json.get("range").is_none());
}
