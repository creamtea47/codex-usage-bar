use super::collection::compact_cycle_for_age;
use super::query::bound_query_points;
use super::*;
use crate::infra::history_store::*;
use std::fs;
use std::{env, time::SystemTime};

fn at(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 6, hour, minute, 0)
        .single()
        .unwrap()
}

fn salt() -> String {
    "11".repeat(SALT_BYTES)
}

fn history() -> UsageHistory {
    UsageHistory::with_salt(salt()).unwrap()
}

fn identity() -> AccountIdentity<'static> {
    AccountIdentity::AccountId("account-private")
}

fn window(
    id: &str,
    seconds: i64,
    remaining: u8,
    reset_at: Option<DateTime<Utc>>,
) -> HistoryWindowInput {
    HistoryWindowInput {
        window_id: id.to_owned(),
        window_seconds: seconds,
        cycle_reset_at: reset_at,
        remaining_percent: remaining,
    }
}

fn record(
    history: &mut UsageHistory,
    time: DateTime<Utc>,
    remaining: u8,
    reset_at: DateTime<Utc>,
) -> HistoryMutation {
    history
        .record_successful_snapshot(
            identity(),
            time,
            &[window("weekly", 604_800, remaining, Some(reset_at))],
        )
        .unwrap()
}

#[test]
fn five_hour_pace_uses_each_cycles_reset_at_even_for_partial_history() {
    let mut value = history();
    for (sample, reset, remaining) in [
        (at(3, 0), at(5, 0), 60),
        (at(5, 0), at(10, 0), 100),
        (at(6, 0), at(10, 0), 75),
    ] {
        value
            .record_successful_snapshot(
                identity(),
                sample,
                &[window("five", 18000, remaining, Some(reset))],
            )
            .unwrap();
    }
    let stream = &value.streams[0];
    let points = stream.query_points(at(0, 0), at(11, 0), None);
    assert_eq!(
        points
            .iter()
            .map(|p| (p.suggested_remaining_percent, p.break_before))
            .collect::<Vec<_>>(),
        vec![(Some(40), true), (Some(100), true), (Some(80), false)]
    );
    assert_eq!(stream.suggested_remaining(None, at(3, 0)), None);
}

fn generation(id: &str) -> HistoryGenerationInput {
    HistoryGenerationInput {
        window_id: "weekly".to_owned(),
        window_seconds: 604_800,
        generation_id: id.to_owned(),
    }
}

fn record_with_generation(
    history: &mut UsageHistory,
    time: DateTime<Utc>,
    remaining: u8,
    reset_at: DateTime<Utc>,
    generation_id: &str,
) -> HistoryMutation {
    history
        .record_successful_snapshot_with_generations(
            identity(),
            time,
            &[window("weekly", 604_800, remaining, Some(reset_at))],
            &[generation(generation_id)],
        )
        .unwrap()
}

fn temp_path(name: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    env::temp_dir().join(format!("codex-usage-history-{name}-{nonce}.json"))
}

#[test]
fn fingerprints_are_salted_domain_separated_and_redacted_in_debug() {
    let first = account_fingerprint(&salt(), AccountIdentity::AccountId("same")).unwrap();
    let second =
        account_fingerprint(&"22".repeat(SALT_BYTES), AccountIdentity::AccountId("same")).unwrap();
    let fallback = account_fingerprint(&salt(), AccountIdentity::Token("same")).unwrap();
    let selected_account = account_fingerprint(
        &salt(),
        AccountIdentity::from_parts(Some("same"), "ignored-token"),
    )
    .unwrap();
    let selected_fallback =
        account_fingerprint(&salt(), AccountIdentity::from_parts(None, "same")).unwrap();
    assert_eq!(first.len(), FINGERPRINT_BYTES * 2);
    assert_ne!(first, second);
    assert_ne!(first, fallback);
    assert_eq!(selected_account, first);
    assert_eq!(selected_fallback, fallback);
    assert_eq!(
        format!("{:?}", AccountIdentity::Token("secret-token")),
        "Token(\"[redacted]\")"
    );
    assert!(matches!(
        account_fingerprint("bad", identity()),
        Err(FingerprintError::InvalidSalt)
    ));
    assert!(matches!(
        account_fingerprint(&salt(), AccountIdentity::Token("  ")),
        Err(FingerprintError::EmptyIdentity)
    ));
}

#[test]
fn schema_round_trip_contains_only_allowed_history_fields() {
    let mut value = history();
    let reset = at(20, 0);
    value
        .record_successful_snapshot(
            identity(),
            at(8, 0),
            &[window(
                "private-account@example.com",
                604_800,
                91,
                Some(reset),
            )],
        )
        .unwrap();
    let path = temp_path("roundtrip");
    save_history(&path, &value).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("schemaVersion"));
    assert!(text.contains("accountFingerprint"));
    assert!(text.contains("windowId"));
    assert!(text.contains("remainingPercent"));
    for private in [
        "account-private",
        "private-account@example.com",
        "secret-token",
        "email",
        "label",
        "proxy",
        "auth.json",
        "rawResponse",
    ] {
        assert!(!text.contains(private));
    }
    let loaded = load_history_at(&path, at(9, 0));
    assert_eq!(loaded.status, HistoryStorageStatus::Ready);
    assert_eq!(loaded.history, value);
    let _ = fs::remove_file(path);
}

#[test]
fn corrupt_and_unknown_files_recover_empty_without_blocking() {
    let corrupt = temp_path("corrupt");
    fs::write(&corrupt, b"{not json").unwrap();
    let recovered = load_history_at(&corrupt, at(9, 0));
    assert_eq!(recovered.status, HistoryStorageStatus::RecoveredCorrupt);
    assert!(recovered.history.is_empty());
    assert!(recovered.needs_rewrite);
    let _ = fs::remove_file(corrupt);

    let unknown = temp_path("unknown");
    fs::write(
        &unknown,
        br#"{"schemaVersion":999,"token":"must-not-survive"}"#,
    )
    .unwrap();
    let recovered = load_history_at(&unknown, at(9, 0));
    assert_eq!(recovered.status, HistoryStorageStatus::RecoveredUnsupported);
    assert!(recovered.history.is_empty());
    let serialized = serde_json::to_string(&recovered.history).unwrap();
    assert!(!serialized.contains("must-not-survive"));
    let _ = fs::remove_file(unknown);
}

#[test]
fn explicit_storage_clear_removes_target_and_temporary_files() {
    let path = temp_path("clear-storage");
    let temporary_path = path.with_extension("json.tmp");
    fs::write(&path, b"old-account-history").unwrap();
    fs::write(&temporary_path, b"stale-temporary-history").unwrap();

    clear_history_storage(&path).unwrap();
    assert!(!path.exists());
    assert!(!temporary_path.exists());
    // Missing files are already in the desired privacy-safe state.
    clear_history_storage(&path).unwrap();
}

#[test]
fn missing_file_starts_with_a_valid_random_salt() {
    let path = temp_path("missing");
    let first = load_history_at(&path, at(9, 0));
    let second = load_history_at(&path, at(9, 0));
    assert_eq!(first.status, HistoryStorageStatus::Missing);
    assert!(!first.needs_rewrite);
    assert_ne!(first.history.salt, second.history.salt);
    assert!(decode_hex_exact(&first.history.salt, SALT_BYTES).is_some());
}

#[test]
fn account_switch_restores_each_anonymous_partition() {
    let mut value = history();
    record(&mut value, at(8, 0), 80, at(20, 0));
    assert_eq!(value.sample_count(), 1);
    let selection = value
        .select_account(AccountIdentity::AccountId("another-account"))
        .unwrap();
    assert_eq!(selection, AccountSelection::Switched { created: true });
    assert!(value.is_empty());
    value
        .record_successful_snapshot(
            AccountIdentity::AccountId("another-account"),
            at(9, 0),
            &[window("weekly", 604_800, 60, Some(at(20, 0)))],
        )
        .unwrap();

    assert_eq!(
        value.select_account(identity()).unwrap(),
        AccountSelection::Switched { created: false }
    );
    assert_eq!(value.sample_count(), 1);
    assert_eq!(value.summary().latest_sample_at, Some(at(8, 0)));

    let serialized = serde_json::to_string(&value).unwrap();
    assert!(!serialized.contains("account-private"));
    assert!(!serialized.contains("another-account"));
    let mut restored: UsageHistory = serde_json::from_str(&serialized).unwrap();
    restored
        .select_account(AccountIdentity::AccountId("another-account"))
        .unwrap();
    assert_eq!(restored.summary().latest_sample_at, Some(at(9, 0)));
}

#[test]
fn samples_on_interval_change_new_window_and_new_cycle() {
    let mut value = history();
    let reset = at(20, 0);
    assert_eq!(record(&mut value, at(8, 0), 80, reset).samples_recorded, 1);
    // 未变化且不足五分钟。
    assert_eq!(record(&mut value, at(8, 4), 80, reset).samples_recorded, 0);
    // 百分比变化绕过节流。
    assert_eq!(record(&mut value, at(8, 4), 79, reset).samples_recorded, 1);
    // 未变化但满五分钟。
    assert_eq!(record(&mut value, at(8, 9), 79, reset).samples_recorded, 1);

    let second = window("short", 18_000, 60, Some(at(12, 0)));
    assert_eq!(
        value
            .record_successful_snapshot(identity(), at(8, 10), &[second])
            .unwrap()
            .samples_recorded,
        1
    );

    let next_reset = reset + Duration::days(7);
    assert_eq!(
        record(&mut value, reset + Duration::minutes(1), 100, next_reset).samples_recorded,
        1
    );
    let weekly_key = window_stream_key(&value.salt, "weekly", 604_800).unwrap();
    let weekly = value
        .streams
        .iter()
        .find(|stream| stream.window_id == weekly_key)
        .unwrap();
    assert_eq!(weekly.cycles.len(), 2);
}

#[test]
fn generation_change_splits_same_reset_at_recovery_from_98_or_99_to_100() {
    for (index, previous) in [98, 99].into_iter().enumerate() {
        let mut value = history();
        let reset = at(20, 0);
        let previous_generation = "aa".repeat(GENERATION_ID_BYTES);
        let next_generation = if index == 0 {
            "bb".repeat(GENERATION_ID_BYTES)
        } else {
            "cc".repeat(GENERATION_ID_BYTES)
        };
        record_with_generation(&mut value, at(8, 0), previous, reset, &previous_generation);
        record_with_generation(&mut value, at(8, 1), 100, reset, &next_generation);

        assert_eq!(value.streams[0].cycles.len(), 2);
        let query = value
            .query_request(
                UsageHistoryRequest::Preset {
                    preset: UsageHistoryPreset::Hours24,
                },
                at(8, 2),
            )
            .unwrap();
        assert_eq!(query.series[0].points.len(), 2);
        assert!(query.series[0].points[0].break_before);
        assert!(query.series[0].points[1].break_before);
    }
}

#[test]
fn same_generation_ignores_percent_and_reset_at_noise_for_segmentation() {
    let mut value = history();
    let generation_id = "aa".repeat(GENERATION_ID_BYTES);
    let reset = at(20, 0);
    record_with_generation(&mut value, at(8, 0), 99, reset, &generation_id);

    // 同代次中的 99→100 抖动与截止时间校正都只能增加采样点，不能伪造新周期。
    record_with_generation(
        &mut value,
        at(8, 1),
        100,
        reset + Duration::hours(2),
        &generation_id,
    );
    record_with_generation(&mut value, at(8, 2), 99, reset, &generation_id);
    assert_eq!(value.streams[0].cycles.len(), 1);
    assert_eq!(value.streams[0].cycles[0].samples.len(), 3);
}

#[test]
fn first_generation_adopts_legacy_cycle_without_an_upgrade_breakpoint() {
    let mut value = history();
    let reset = at(20, 0);
    record(&mut value, at(8, 0), 80, reset);
    let generation_id = "aa".repeat(GENERATION_ID_BYTES);
    let mutation = record_with_generation(&mut value, at(8, 1), 80, reset, &generation_id);

    assert_eq!(value.streams[0].cycles.len(), 1);
    assert_eq!(value.streams[0].cycles[0].samples.len(), 1);
    assert_eq!(mutation.samples_recorded, 0);
    assert!(mutation.generation_metadata_updated);
    assert!(mutation.changed());
    assert_eq!(
        value.streams[0].cycles[0].generation_id.as_deref(),
        Some(generation_id.as_str())
    );
}

#[test]
fn generation_is_persisted_but_never_exposed_by_trend_query() {
    let mut value = history();
    let generation_id = "aa".repeat(GENERATION_ID_BYTES);
    record_with_generation(&mut value, at(8, 0), 99, at(20, 0), &generation_id);
    let path = temp_path("generation-roundtrip");
    save_history(&path, &value).unwrap();
    let stored_json = fs::read_to_string(&path).unwrap();
    assert!(stored_json.contains("generationId"));
    assert!(stored_json.contains(&generation_id));

    let loaded = load_history_at(&path, at(9, 0));
    assert_eq!(loaded.status, HistoryStorageStatus::Ready);
    assert_eq!(loaded.history, value);
    let query_json = serde_json::to_string(
        &loaded
            .history
            .query_request(
                UsageHistoryRequest::Preset {
                    preset: UsageHistoryPreset::Hours24,
                },
                at(9, 0),
            )
            .unwrap(),
    )
    .unwrap();
    assert!(!query_json.contains("generationId"));
    assert!(!query_json.contains(&generation_id));
    let _ = fs::remove_file(path);
}

#[test]
fn forward_reset_correction_before_expiry_does_not_split_a_cycle() {
    let mut value = history();
    let original_reset = at(20, 0);
    record(&mut value, at(8, 0), 80, original_reset);

    let corrected_reset = original_reset + Duration::hours(2);
    record(&mut value, at(9, 0), 79, corrected_reset);
    assert_eq!(value.streams[0].cycles.len(), 1);
    assert_eq!(value.streams[0].cycles[0].reset_at, Some(corrected_reset));

    let next_reset = corrected_reset + Duration::days(7);
    record(
        &mut value,
        corrected_reset + Duration::minutes(1),
        100,
        next_reset,
    );
    assert_eq!(value.streams[0].cycles.len(), 2);
}

#[test]
fn percentage_rise_segments_a_cycle_when_reset_timestamp_is_missing() {
    let mut value = history();
    value
        .record_successful_snapshot(identity(), at(8, 0), &[window("unknown", 86_400, 10, None)])
        .unwrap();
    value
        .record_successful_snapshot(identity(), at(8, 1), &[window("unknown", 86_400, 99, None)])
        .unwrap();
    assert_eq!(value.streams[0].cycles.len(), 2);
    let query = value
        .query_request(
            UsageHistoryRequest::Preset {
                preset: UsageHistoryPreset::Hours24,
            },
            at(8, 2),
        )
        .unwrap();
    assert!(query.series[0].points[0].break_before);
    assert!(query.series[0].points[1].break_before);
}

#[test]
fn duplicate_windows_invalid_values_and_clock_rollback_are_ignored() {
    let mut value = history();
    let reset = at(20, 0);
    record(&mut value, at(8, 0), 80, reset);
    let duplicate = window("weekly", 604_800, 79, Some(reset));
    let invalid = window("", 0, 101, None);
    let mutation = value
        .record_successful_snapshot(
            identity(),
            at(8, 1),
            &[duplicate.clone(), duplicate, invalid],
        )
        .unwrap();
    assert_eq!(mutation.samples_recorded, 1);
    assert_eq!(mutation.ignored_windows, 2);
    assert_eq!(record(&mut value, at(7, 59), 70, reset).samples_recorded, 0);
}
#[test]
fn permanent_history_keeps_old_samples_and_unbounded_streams() {
    let now = at(8, 0);
    let mut value = history();
    let reset = now + Duration::days(7);
    record(&mut value, now - Duration::days(400), 80, reset);
    for index in 0..20 {
        value
            .record_successful_snapshot(
                identity(),
                now + Duration::minutes(index),
                &[window(
                    &format!("window-{index}"),
                    60 + index,
                    90,
                    Some(reset),
                )],
            )
            .unwrap();
    }

    value.compact_at(now);

    assert_eq!(value.stream_count(), 21);
    assert!(value
        .all_samples()
        .any(|sample| sample.sampled_at == now - Duration::days(400)));
    let all = value
        .query_request(
            UsageHistoryRequest::Preset {
                preset: UsageHistoryPreset::All,
            },
            now,
        )
        .unwrap();
    assert_eq!(all.applied_start_at, now - Duration::days(400));
    assert_eq!(all.bucket_seconds, Some(DAILY_BUCKET_SECONDS));
}
#[test]
fn seven_day_query_downsamples_and_never_exposes_internal_metadata() {
    let mut value = history();
    let reset = at(20, 0) + Duration::days(7);
    for minute in 0..180 {
        record(
            &mut value,
            at(8, 0) + Duration::minutes(minute),
            100 - (minute / 10) as u8,
            reset,
        );
    }
    let full = value
        .query_request(
            UsageHistoryRequest::Preset {
                preset: UsageHistoryPreset::Hours24,
            },
            at(11, 0),
        )
        .unwrap();
    let sampled = value
        .query_request(
            UsageHistoryRequest::Preset {
                preset: UsageHistoryPreset::Days7,
            },
            at(11, 0),
        )
        .unwrap();
    assert!(sampled.series[0].points.len() < full.series[0].points.len());
    assert!(sampled.series[0].points[0].break_before);
    let json = serde_json::to_string(&sampled).unwrap();
    assert!(!json.contains("accountFingerprint"));
    assert!(!json.contains("salt"));
    assert!(!json.contains("resetAt"));
    assert!(!json.contains("label"));
}

#[test]
fn query_output_is_bounded_and_preserves_every_included_cycle_boundary() {
    let now = at(12, 0);
    let start = now - Duration::hours(20);
    let mut cycles = Vec::new();
    let mut expected_boundaries = Vec::new();
    for cycle_index in 0..4 {
        let cycle_start = start + Duration::minutes((cycle_index * 300) as i64);
        let samples = (0..300)
            .map(|sample_index| StoredUsageSample {
                sampled_at: cycle_start + Duration::minutes(sample_index as i64),
                remaining_percent: (sample_index % 101) as u8,
            })
            .collect::<Vec<_>>();
        expected_boundaries.push((
            samples.first().unwrap().sampled_at,
            samples.last().unwrap().sampled_at,
        ));
        cycles.push(StoredUsageCycle {
            first_exhausted_at: None,
            generation_id: None,
            reset_at: None,
            samples,
        });
    }
    let mut value = history();
    value.select_account(identity()).unwrap();
    value.streams.push(StoredUsageStream {
        window_id: window_stream_key(&value.salt, "weekly", 604_800).unwrap(),
        window_seconds: 604_800,
        cycles,
    });

    let query = value
        .query_request(
            UsageHistoryRequest::Preset {
                preset: UsageHistoryPreset::Hours24,
            },
            now,
        )
        .unwrap();
    let points = &query.series[0].points;
    assert!(points.len() <= MAX_QUERY_POINTS_PER_SERIES);
    for (first_at, last_at) in expected_boundaries {
        let first = points
            .iter()
            .find(|point| point.sampled_at == first_at)
            .expect("cycle first point must survive query bounding");
        assert!(first.break_before);
        assert!(points.iter().any(|point| point.sampled_at == last_at));
    }
}

#[test]
fn query_bound_never_returns_empty_when_required_boundaries_exceed_the_cap() {
    let start = at(8, 0) - Duration::hours(1);
    let points = (0..MAX_QUERY_POINTS_PER_SERIES + 1)
        .flat_map(|cycle_index| {
            let first_at = start + Duration::seconds((cycle_index * 2) as i64);
            [
                UsageHistoryPoint {
                    suggested_remaining_percent: None,
                    sampled_at: first_at,
                    remaining_percent: 100,
                    break_before: true,
                },
                UsageHistoryPoint {
                    suggested_remaining_percent: None,
                    sampled_at: first_at + Duration::seconds(1),
                    remaining_percent: 99,
                    break_before: false,
                },
            ]
        })
        .collect::<Vec<_>>();
    let latest_at = points.last().unwrap().sampled_at;

    let bounded = bound_query_points(points, MAX_QUERY_POINTS_PER_SERIES);

    assert!(!bounded.is_empty());
    assert!(bounded.len() <= MAX_QUERY_POINTS_PER_SERIES);
    assert!(bounded[0].break_before);
    assert_eq!(bounded.last().unwrap().sampled_at, latest_at);
}

#[test]
fn query_bound_safely_samples_a_latest_cycle_larger_than_the_cap() {
    let start = at(8, 0) - Duration::hours(1);
    let points = (0..MAX_QUERY_POINTS_PER_SERIES + 500)
        .map(|index| UsageHistoryPoint {
            suggested_remaining_percent: None,
            sampled_at: start + Duration::seconds(index as i64),
            remaining_percent: (index % 101) as u8,
            break_before: index == 0,
        })
        .collect::<Vec<_>>();
    let first_at = points.first().unwrap().sampled_at;
    let latest_at = points.last().unwrap().sampled_at;

    let bounded = bound_query_points(points, MAX_QUERY_POINTS_PER_SERIES);

    assert_eq!(bounded.len(), MAX_QUERY_POINTS_PER_SERIES);
    assert!(bounded[0].break_before);
    assert_eq!(bounded[0].sampled_at, first_at);
    assert_eq!(bounded.last().unwrap().sampled_at, latest_at);
}

#[test]
fn query_bound_samples_latest_large_cycle_when_old_boundaries_reach_the_cap() {
    let start = at(8, 0) - Duration::hours(1);
    let mut points = (0..500)
        .flat_map(|cycle_index| {
            let first_at = start + Duration::seconds((cycle_index * 2) as i64);
            [
                UsageHistoryPoint {
                    suggested_remaining_percent: None,
                    sampled_at: first_at,
                    remaining_percent: 1,
                    break_before: true,
                },
                UsageHistoryPoint {
                    suggested_remaining_percent: None,
                    sampled_at: first_at + Duration::seconds(1),
                    remaining_percent: 0,
                    break_before: false,
                },
            ]
        })
        .collect::<Vec<_>>();
    let latest_cycle_start = start + Duration::seconds(1_000);
    points.extend((0..1_500).map(|index| UsageHistoryPoint {
        suggested_remaining_percent: None,
        sampled_at: latest_cycle_start + Duration::seconds(index as i64),
        remaining_percent: (index % 101) as u8,
        break_before: index == 0,
    }));
    let latest_at = points.last().unwrap().sampled_at;

    let bounded = bound_query_points(points, MAX_QUERY_POINTS_PER_SERIES);

    assert_eq!(bounded.len(), MAX_QUERY_POINTS_PER_SERIES);
    assert!(bounded[0].break_before);
    assert_eq!(bounded[0].sampled_at, latest_cycle_start);
    assert_eq!(bounded.last().unwrap().sampled_at, latest_at);
    assert!(bounded
        .windows(2)
        .all(|window| window[0].sampled_at < window[1].sampled_at));
}

#[test]
fn structured_requests_deserialize_exactly_and_presets_use_a_half_open_snapshot() {
    let now = at(12, 0);
    let preset: UsageHistoryRequest =
        serde_json::from_str(r#"{"kind":"preset","preset":"30d"}"#).unwrap();
    assert_eq!(
        preset,
        UsageHistoryRequest::Preset {
            preset: UsageHistoryPreset::Days30,
        }
    );
    let resolved = preset.resolve(now, None).unwrap();
    assert_eq!(resolved.start_at, now - Duration::days(30));
    assert_eq!(resolved.end_at_exclusive, now);
    assert_eq!(resolved.bucket_seconds, Some(60 * 60));
    assert!(!resolved.truncated_by_retention);

    let all: UsageHistoryRequest =
        serde_json::from_str(r#"{"kind":"preset","preset":"all"}"#).unwrap();
    let earliest = now - Duration::days(400);
    let resolved_all = all.resolve(now, Some(earliest)).unwrap();
    assert_eq!(resolved_all.start_at, earliest);
    assert_eq!(resolved_all.end_at_exclusive, now);
    assert_eq!(resolved_all.bucket_seconds, Some(DAILY_BUCKET_SECONDS));

    let custom: UsageHistoryRequest = serde_json::from_str(
        r#"{"kind":"custom","startAt":"2026-08-06T08:00:00Z","endAtExclusive":"2026-08-06T09:00:00Z"}"#,
    )
    .unwrap();
    assert_eq!(
        custom,
        UsageHistoryRequest::Custom {
            start_at: at(8, 0),
            end_at_exclusive: at(9, 0),
        }
    );

    let mut value = history();
    let reset = now + Duration::days(7);
    record(&mut value, now - Duration::minutes(1), 80, reset);
    record(&mut value, now, 79, reset);
    let query = value
        .query_request(
            UsageHistoryRequest::Preset {
                preset: UsageHistoryPreset::Hours24,
            },
            now,
        )
        .unwrap();
    assert_eq!(query.series[0].points.len(), 1);
    assert_eq!(
        query.series[0].points[0].sampled_at,
        now - Duration::minutes(1)
    );
    assert_eq!(query.series[0].current_remaining_percent, 79);
    assert_eq!(query.applied_end_at_exclusive, now);
}

#[test]
fn custom_bucket_selection_changes_only_after_exact_tier_boundaries() {
    let now = at(12, 0);
    let bucket_for = |span| {
        UsageHistoryRequest::Custom {
            start_at: now - span,
            end_at_exclusive: now,
        }
        .resolve(now, None)
        .unwrap()
        .bucket_seconds
    };

    assert_eq!(bucket_for(Duration::hours(24)), None);
    assert_eq!(
        bucket_for(Duration::hours(24) + Duration::nanoseconds(1)),
        Some(SEVEN_DAY_BUCKET_SECONDS)
    );
    assert_eq!(
        bucket_for(Duration::days(7)),
        Some(SEVEN_DAY_BUCKET_SECONDS)
    );
    assert_eq!(
        bucket_for(Duration::days(7) + Duration::nanoseconds(1)),
        Some(MONTH_BUCKET_SECONDS)
    );
    assert_eq!(
        bucket_for(Duration::days(32) + Duration::nanoseconds(1)),
        Some(DAILY_BUCKET_SECONDS)
    );
}

#[test]
fn custom_request_validation_is_defensive_but_allows_a_dst_safe_span() {
    let now = at(12, 0);
    let resolve = |start_at, end_at_exclusive| {
        UsageHistoryRequest::Custom {
            start_at,
            end_at_exclusive,
        }
        .resolve(now, None)
    };

    assert_eq!(
        resolve(now, now),
        Err(UsageHistoryQueryError::EmptyOrReversedRange)
    );
    assert_eq!(
        resolve(now, now - Duration::seconds(1)),
        Err(UsageHistoryQueryError::EmptyOrReversedRange)
    );
    assert_eq!(
        resolve(now - Duration::hours(1), now + Duration::nanoseconds(1)),
        Err(UsageHistoryQueryError::FutureEnd)
    );
    assert!(resolve(now - Duration::days(400), now).is_ok());
}

#[test]
fn old_custom_range_is_not_truncated_by_retention() {
    let now = at(12, 0);
    let end_at_exclusive = now - Duration::days(32) - Duration::hours(1);
    let request = UsageHistoryRequest::Custom {
        start_at: end_at_exclusive - Duration::days(1),
        end_at_exclusive,
    };
    let requested_start = end_at_exclusive - Duration::days(1);
    let resolved = request.resolve(now, None).unwrap();
    assert_eq!(resolved.start_at, requested_start);
    assert_eq!(resolved.end_at_exclusive, end_at_exclusive);
    assert!(!resolved.truncated_by_retention);

    let query = history().query_request(request, now).unwrap();
    assert_eq!(query.applied_start_at, requested_start);
    assert_eq!(query.sample_count, 0);
    assert!(query.series.is_empty());
    assert!(!query.truncated_by_retention);
}

#[test]
fn custom_query_uses_half_open_bounds_and_keeps_current_value_global() {
    let mut value = history();
    let now = at(12, 0);
    let reset = now + Duration::days(7);
    record(&mut value, now - Duration::hours(3), 90, reset);
    record(&mut value, now - Duration::hours(2), 80, reset);
    record(&mut value, now - Duration::hours(1), 70, reset);

    let query = value
        .query_request(
            UsageHistoryRequest::Custom {
                start_at: now - Duration::hours(3),
                end_at_exclusive: now - Duration::hours(1),
            },
            now,
        )
        .unwrap();
    assert_eq!(query.series[0].points.len(), 2);
    assert_eq!(
        query.series[0].points[0].sampled_at,
        now - Duration::hours(3)
    );
    assert_eq!(
        query.series[0].points[1].sampled_at,
        now - Duration::hours(2)
    );
    assert_eq!(query.series[0].current_remaining_percent, 70);
    assert_eq!(query.sample_count, 2);
    assert_eq!(query.earliest_sample_at, Some(now - Duration::hours(3)));
    assert_eq!(query.latest_sample_at, Some(now - Duration::hours(2)));
    assert_eq!(query.available_start_at, Some(now - Duration::hours(3)));
    assert_eq!(query.available_end_at, Some(now - Duration::hours(1)));
}

#[test]
fn available_summary_includes_now_but_excludes_future_samples() {
    let mut value = history();
    let now = at(12, 0);
    let reset = now + Duration::days(7);
    record(&mut value, now - Duration::minutes(1), 80, reset);
    record(&mut value, now, 79, reset);
    record(&mut value, now + Duration::minutes(1), 78, reset);

    let query = value
        .query_request(
            UsageHistoryRequest::Preset {
                preset: UsageHistoryPreset::Hours24,
            },
            now,
        )
        .unwrap();

    assert_eq!(query.available_start_at, Some(now - Duration::minutes(1)));
    assert_eq!(query.available_end_at, Some(now));
    assert_eq!(query.series[0].current_remaining_percent, 79);
    assert_eq!(query.latest_sample_at, Some(now - Duration::minutes(1)));
}

#[test]
fn available_summary_includes_samples_older_than_thirty_two_days() {
    let sampled_at = at(12, 0);
    let now = sampled_at + Duration::days(33);
    let mut value = history();
    record(&mut value, sampled_at, 80, sampled_at + Duration::days(7));

    let query = value
        .query_request(
            UsageHistoryRequest::Preset {
                preset: UsageHistoryPreset::Hours24,
            },
            now,
        )
        .unwrap();

    assert_eq!(query.sample_count, 0);
    assert!(query.series.is_empty());
    assert_eq!(query.available_start_at, Some(sampled_at));
    assert_eq!(query.available_end_at, Some(sampled_at));
}

#[test]
fn historical_custom_query_keeps_forecast_based_on_the_global_latest_cycle() {
    let mut value = history();
    let now = at(12, 0);
    let old_reset = now - Duration::hours(1);
    for (hours_ago, remaining) in [(10, 90), (9, 80), (8, 70), (7, 60)] {
        record(
            &mut value,
            now - Duration::hours(hours_ago),
            remaining,
            old_reset,
        );
    }
    let current_reset = now + Duration::hours(12);
    for (minutes_ago, remaining) in [(60, 80), (40, 70), (20, 60), (0, 50)] {
        record(
            &mut value,
            now - Duration::minutes(minutes_ago),
            remaining,
            current_reset,
        );
    }
    let expected_forecast = value.forecast_for("weekly", 604_800, now).unwrap();

    let query = value
        .query_request(
            UsageHistoryRequest::Custom {
                start_at: now - Duration::hours(11),
                end_at_exclusive: now - Duration::hours(6),
            },
            now,
        )
        .unwrap();

    assert_eq!(query.series[0].current_remaining_percent, 50);
    assert_eq!(query.series[0].forecast, expected_forecast);
    assert!(query.series[0]
        .points
        .iter()
        .all(|point| point.sampled_at < now - Duration::hours(6)));
}

#[test]
fn compacts_four_age_tiers_without_deleting_old_cycle_boundaries() {
    let now = at(12, 0);
    let daily_first = now - Duration::days(400) + Duration::hours(1);
    let hourly_first = now - Duration::days(10) + Duration::minutes(1);
    let medium_first = now - Duration::days(2) + Duration::minutes(1);
    let recent_first = now - Duration::hours(2);
    let samples = |first: DateTime<Utc>, step: Duration| {
        vec![
            StoredUsageSample {
                sampled_at: first,
                remaining_percent: 100,
            },
            StoredUsageSample {
                sampled_at: first + step,
                remaining_percent: 99,
            },
            StoredUsageSample {
                sampled_at: first + step + step,
                remaining_percent: 98,
            },
        ]
    };

    let mut value = history();
    value.select_account(identity()).unwrap();
    value.streams.push(StoredUsageStream {
        window_id: window_stream_key(&value.salt, "weekly", 604_800).unwrap(),
        window_seconds: 604_800,
        cycles: vec![
            StoredUsageCycle {
                first_exhausted_at: None,
                generation_id: None,
                reset_at: None,
                samples: samples(daily_first, Duration::hours(1)),
            },
            StoredUsageCycle {
                first_exhausted_at: None,
                generation_id: None,
                reset_at: None,
                samples: samples(hourly_first, Duration::minutes(10)),
            },
            StoredUsageCycle {
                first_exhausted_at: None,
                generation_id: None,
                reset_at: None,
                samples: samples(medium_first, Duration::minutes(3)),
            },
            StoredUsageCycle {
                first_exhausted_at: None,
                generation_id: None,
                reset_at: None,
                samples: samples(recent_first, Duration::minutes(1)),
            },
        ],
    });

    assert_eq!(value.compact_at(now), 3);
    let cycles = &value.streams[0].cycles;
    assert_eq!(cycles[0].samples.len(), 2);
    assert_eq!(cycles[0].samples[0].sampled_at, daily_first);
    assert_eq!(
        cycles[0].samples[1].sampled_at,
        daily_first + Duration::hours(2)
    );
    assert_eq!(cycles[1].samples.len(), 2);
    assert_eq!(cycles[2].samples.len(), 2);
    assert_eq!(cycles[3].samples.len(), 3);
}

#[test]
fn age_compaction_is_idempotent_at_exact_tier_boundaries() {
    let now = at(12, 0);
    let daily_cutoff = now - Duration::days(32);
    let medium_cutoff = now - Duration::days(7);
    let recent_cutoff = now - Duration::hours(24);
    let sample_at = |sampled_at, remaining_percent| StoredUsageSample {
        sampled_at,
        remaining_percent,
    };
    let samples = vec![
        sample_at(medium_cutoff - Duration::minutes(50), 100),
        sample_at(medium_cutoff - Duration::minutes(40), 99),
        sample_at(medium_cutoff - Duration::minutes(30), 98),
        sample_at(medium_cutoff, 97),
        sample_at(medium_cutoff + Duration::minutes(1), 96),
        sample_at(medium_cutoff + Duration::minutes(2), 95),
        sample_at(recent_cutoff - Duration::minutes(10), 94),
        sample_at(recent_cutoff - Duration::minutes(9), 93),
        sample_at(recent_cutoff - Duration::minutes(8), 92),
        sample_at(recent_cutoff, 91),
        sample_at(recent_cutoff + Duration::minutes(1), 90),
        sample_at(recent_cutoff + Duration::minutes(2), 89),
    ];

    let once = compact_cycle_for_age(&samples, daily_cutoff, medium_cutoff, recent_cutoff);
    let twice = compact_cycle_for_age(&once, daily_cutoff, medium_cutoff, recent_cutoff);

    assert!(once.len() < samples.len());
    assert_eq!(once, twice);
    assert!(once.iter().any(|sample| sample.sampled_at == medium_cutoff));
    assert!(once.iter().any(|sample| sample.sampled_at == recent_cutoff));
}

#[test]
fn age_compaction_keeps_reset_cycles_separate_inside_one_bucket() {
    let now = at(12, 0);
    let first_start = now - Duration::days(10) + Duration::minutes(1);
    let second_start = first_start + Duration::minutes(5);
    let mut stream = StoredUsageStream {
        window_id: "bounded".to_owned(),
        window_seconds: 604_800,
        cycles: vec![
            StoredUsageCycle {
                first_exhausted_at: None,
                generation_id: None,
                reset_at: None,
                samples: vec![
                    StoredUsageSample {
                        sampled_at: first_start,
                        remaining_percent: 10,
                    },
                    StoredUsageSample {
                        sampled_at: first_start + Duration::minutes(1),
                        remaining_percent: 9,
                    },
                    StoredUsageSample {
                        sampled_at: first_start + Duration::minutes(2),
                        remaining_percent: 0,
                    },
                ],
            },
            StoredUsageCycle {
                first_exhausted_at: None,
                generation_id: None,
                reset_at: None,
                samples: vec![
                    StoredUsageSample {
                        sampled_at: second_start,
                        remaining_percent: 100,
                    },
                    StoredUsageSample {
                        sampled_at: second_start + Duration::minutes(1),
                        remaining_percent: 99,
                    },
                    StoredUsageSample {
                        sampled_at: second_start + Duration::minutes(2),
                        remaining_percent: 98,
                    },
                ],
            },
        ],
    };

    stream.compact_for_age(now);
    let points = stream.query_points(now - Duration::days(30), now, Some(MONTH_BUCKET_SECONDS));

    assert_eq!(stream.cycles.len(), 2);
    assert_eq!(points.len(), 4);
    assert!(points[0].break_before);
    assert!(points[2].break_before);
    assert_eq!(stream.cycles[0].samples.len(), 2);
    assert_eq!(stream.cycles[1].samples.len(), 2);
}

#[test]
fn recent_compaction_consumes_a_max_utc_sample_without_a_sentinel_loop() {
    let now = at(12, 0);
    let recent_cutoff = now - Duration::hours(24);
    let samples = vec![
        StoredUsageSample {
            sampled_at: recent_cutoff,
            remaining_percent: 100,
        },
        StoredUsageSample {
            sampled_at: recent_cutoff + Duration::seconds(1),
            remaining_percent: 99,
        },
        StoredUsageSample {
            sampled_at: DateTime::<Utc>::MAX_UTC,
            remaining_percent: 98,
        },
    ];

    let compacted = compact_cycle_for_age(
        &samples,
        now - Duration::days(32),
        now - Duration::days(7),
        recent_cutoff,
    );

    assert_eq!(compacted, samples);
}

#[test]
fn schema_v1_migrates_without_data_loss_and_keeps_a_backup() {
    let now = at(12, 0);
    let mut value = history();
    let reset = now + Duration::days(7);
    record(&mut value, now - Duration::days(6), 90, reset);
    record(&mut value, now - Duration::days(1), 80, reset);
    let path = temp_path("schema-v1-upgrade");
    let mut legacy = serde_json::to_value(&value).unwrap();
    let object = legacy.as_object_mut().unwrap();
    object.insert("schemaVersion".to_owned(), serde_json::Value::from(1));
    object.remove("accounts");
    fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();

    let loaded = load_history_at(&path, now);
    assert_eq!(loaded.status, HistoryStorageStatus::Ready);
    assert_eq!(loaded.history.schema_version, USAGE_HISTORY_SCHEMA_VERSION);
    assert_eq!(loaded.history.sample_count(), 2);
    assert!(loaded.needs_rewrite);

    save_history(&path, &loaded.history).unwrap();
    let backup = path.with_extension("v1.backup.json");
    assert!(backup.exists());
    assert_eq!(load_history_at(&path, now).history.sample_count(), 2);
    clear_legacy_backup_for_current_account(&path, &loaded.history).unwrap();
    assert!(!backup.exists());
    let _ = fs::remove_file(path);
}

#[test]
fn range_summary_counts_only_raw_samples_inside_requested_window() {
    let mut value = history();
    let now = at(12, 0);
    let reset = now + Duration::days(14);
    let older = now - Duration::days(2);
    let boundary = now - Duration::hours(24);
    let recent = now - Duration::hours(1);
    let future = now + Duration::hours(1);
    for (sampled_at, remaining) in [(older, 90), (boundary, 80), (recent, 70), (future, 60)] {
        record(&mut value, sampled_at, remaining, reset);
    }

    let hours_24 = value
        .query_request(
            UsageHistoryRequest::Preset {
                preset: UsageHistoryPreset::Hours24,
            },
            now,
        )
        .unwrap();
    assert_eq!(hours_24.sample_count, 2);
    assert_eq!(hours_24.earliest_sample_at, Some(boundary));
    assert_eq!(hours_24.latest_sample_at, Some(recent));

    let days_7 = value
        .query_request(
            UsageHistoryRequest::Preset {
                preset: UsageHistoryPreset::Days7,
            },
            now,
        )
        .unwrap();
    assert_eq!(days_7.sample_count, 3);
    assert_eq!(days_7.earliest_sample_at, Some(older));
    assert_eq!(days_7.latest_sample_at, Some(recent));
}

#[test]
fn today_consumption_uses_day_boundary_and_sums_across_resets_for_every_range() {
    let mut value = history();
    let today_started_at = at(0, 0);
    let first_reset = at(2, 0);
    record(
        &mut value,
        today_started_at - Duration::minutes(5),
        90,
        first_reset,
    );
    record(&mut value, at(1, 0), 70, first_reset);

    let next_reset = first_reset + Duration::hours(5);
    record(&mut value, first_reset, 100, next_reset);
    record(&mut value, at(4, 0), 80, next_reset);
    record(&mut value, at(6, 0), 40, next_reset);

    let now = at(6, 0);
    let requests = [
        UsageHistoryRequest::Preset {
            preset: UsageHistoryPreset::Hours24,
        },
        UsageHistoryRequest::Preset {
            preset: UsageHistoryPreset::Days7,
        },
        UsageHistoryRequest::Preset {
            preset: UsageHistoryPreset::Days30,
        },
        UsageHistoryRequest::Custom {
            start_at: today_started_at - Duration::minutes(5),
            end_at_exclusive: at(1, 30),
        },
    ];
    for request in requests {
        let query = value
            .query_request_with_day_start(request, now, today_started_at)
            .unwrap();
        assert_eq!(query.series[0].today_consumed_percent, 80);
    }
}

#[test]
fn today_consumption_ignores_samples_outside_the_natural_day_and_future() {
    let mut value = history();
    let today_started_at = at(0, 0);
    let reset = at(20, 0);
    record(&mut value, today_started_at - Duration::hours(2), 90, reset);
    record(&mut value, at(1, 0), 80, reset);
    record(&mut value, at(6, 0), 70, reset);
    record(&mut value, at(7, 0), 60, reset);

    let query = value
        .query_request_with_day_start(
            UsageHistoryRequest::Preset {
                preset: UsageHistoryPreset::Hours24,
            },
            at(6, 0),
            today_started_at,
        )
        .unwrap();
    assert_eq!(query.series[0].today_consumed_percent, 20);

    let invalid_boundary = value
        .query_request_with_day_start(
            UsageHistoryRequest::Preset {
                preset: UsageHistoryPreset::Hours24,
            },
            at(6, 0),
            at(7, 0),
        )
        .unwrap();
    assert_eq!(invalid_boundary.series[0].today_consumed_percent, 0);
}

#[test]
fn forecast_collects_until_sample_count_and_span_are_sufficient() {
    let mut value = history();
    let reset = at(20, 0);
    for (minute, remaining) in [(0, 100), (10, 99), (20, 98)] {
        record(&mut value, at(8, minute), remaining, reset);
    }
    let forecast = value.forecast_for("weekly", 604_800, at(8, 20)).unwrap();
    assert_eq!(forecast.status, ForecastStatus::Collecting);
    assert_eq!(forecast.sample_count, 3);

    record(&mut value, at(8, 29), 97, reset);
    let forecast = value.forecast_for("weekly", 604_800, at(8, 29)).unwrap();
    assert_eq!(forecast.status, ForecastStatus::Collecting);
    assert_eq!(forecast.observed_span_seconds, 29 * 60);
}

#[test]
fn forecast_reports_stable_when_observed_decline_is_below_two_points() {
    let mut value = history();
    let reset = at(20, 0);
    for (minute, remaining) in [(0, 80), (10, 80), (20, 79), (30, 79)] {
        record(&mut value, at(8, minute), remaining, reset);
    }
    let forecast = value.forecast_for("weekly", 604_800, at(8, 30)).unwrap();
    assert_eq!(forecast.status, ForecastStatus::Stable);
    assert_eq!(forecast.consumed_percent, 1);
    assert_eq!(forecast.exhausts_at, None);
}

#[test]
fn forecast_reports_exhaustion_before_reset_rounded_to_quarter_hour() {
    let mut value = history();
    let reset = at(20, 0);
    for (minute, remaining) in [(0, 80), (10, 70), (20, 60), (30, 50)] {
        record(&mut value, at(8, minute), remaining, reset);
    }
    let forecast = value.forecast_for("weekly", 604_800, at(8, 30)).unwrap();
    assert_eq!(forecast.status, ForecastStatus::ExhaustsBeforeReset);
    assert_eq!(forecast.exhausts_at, Some(at(9, 15)));
    assert_eq!(forecast.sample_count, 4);
    assert_eq!(forecast.observed_span_seconds, 30 * 60);
    assert_eq!(forecast.consumed_percent, 30);
}

#[test]
fn forecast_does_not_extrapolate_past_reset() {
    let mut value = history();
    let reset = at(10, 0);
    for (minute, remaining) in [(0, 90), (10, 88), (20, 86), (30, 84)] {
        record(&mut value, at(8, minute), remaining, reset);
    }
    let forecast = value.forecast_for("weekly", 604_800, at(8, 30)).unwrap();
    assert_eq!(forecast.status, ForecastStatus::LastsUntilReset);
    assert_eq!(forecast.exhausts_at, None);
}

#[test]
fn forecast_uses_only_latest_cycle_and_last_six_hours() {
    let mut value = history();
    let old_reset = at(10, 0);
    for (minute, remaining) in [(0, 100), (10, 70), (20, 40), (30, 10)] {
        record(&mut value, at(8, minute), remaining, old_reset);
    }
    let new_reset = old_reset + Duration::days(7);
    // 在旧截止之后进入新周期，并放入一个超过六小时回看范围的旧点。
    record(&mut value, old_reset, 80, new_reset);
    for (minutes, remaining) in [(390, 100), (400, 100), (410, 99), (420, 99)] {
        record(
            &mut value,
            old_reset + Duration::minutes(minutes),
            remaining,
            new_reset,
        );
    }
    let forecast = value
        .forecast_for("weekly", 604_800, old_reset + Duration::minutes(420))
        .unwrap();
    assert_eq!(forecast.status, ForecastStatus::Stable);
    assert_eq!(forecast.sample_count, 4);
    assert_eq!(forecast.consumed_percent, 1);
}

#[test]
fn zero_percent_can_produce_an_immediate_reliable_forecast() {
    let mut value = history();
    let reset = at(20, 0);
    for (minute, remaining) in [(0, 9), (10, 6), (20, 3), (30, 0)] {
        record(&mut value, at(8, minute), remaining, reset);
    }
    let forecast = value.forecast_for("weekly", 604_800, at(8, 30)).unwrap();
    assert_eq!(forecast.status, ForecastStatus::ExhaustsBeforeReset);
    assert_eq!(forecast.exhausts_at, Some(at(8, 30)));
}

#[test]
fn zero_percent_at_non_quarter_hour_never_rounds_before_latest_sample() {
    let mut value = history();
    let reset = at(20, 0);
    for (minute, remaining) in [(7, 9), (17, 6), (27, 3), (37, 0)] {
        record(&mut value, at(8, minute), remaining, reset);
    }

    let latest_sample_at = at(8, 37);
    let forecast = value
        .forecast_for("weekly", 604_800, latest_sample_at)
        .unwrap();
    assert_eq!(forecast.status, ForecastStatus::ExhaustsBeforeReset);
    assert_eq!(forecast.exhausts_at, Some(at(8, 45)));
    assert!(forecast.exhausts_at.unwrap() >= latest_sample_at);
}

#[test]
fn zero_percent_prefers_latest_sample_when_next_quarter_crosses_reset() {
    let mut value = history();
    let reset = at(8, 40);
    for (minute, remaining) in [(7, 9), (17, 6), (27, 3), (37, 0)] {
        record(&mut value, at(8, minute), remaining, reset);
    }

    let latest_sample_at = at(8, 37);
    let forecast = value
        .forecast_for("weekly", 604_800, latest_sample_at)
        .unwrap();
    assert_eq!(forecast.status, ForecastStatus::ExhaustsBeforeReset);
    assert_eq!(forecast.exhausts_at, Some(latest_sample_at));
    assert!(forecast.exhausts_at.unwrap() < reset);
}

#[test]
fn system_clock_rollback_never_uses_future_samples() {
    let mut value = history();
    let reset = at(20, 0);
    for (minute, remaining) in [(0, 80), (10, 70), (20, 60), (30, 50)] {
        record(&mut value, at(8, minute), remaining, reset);
    }
    let forecast = value.forecast_for("weekly", 604_800, at(7, 59)).unwrap();
    assert_eq!(forecast.status, ForecastStatus::Collecting);
    assert_eq!(forecast.sample_count, 0);
    assert_eq!(forecast.exhausts_at, None);
}

#[test]
fn clear_keeps_account_partition_but_removes_all_points() {
    let mut value = history();
    record(&mut value, at(8, 0), 80, at(20, 0));
    let fingerprint = value.account_fingerprint.clone();
    value
        .select_account(AccountIdentity::AccountId("another-account"))
        .unwrap();
    value
        .record_successful_snapshot(
            AccountIdentity::AccountId("another-account"),
            at(9, 0),
            &[window("weekly", 604_800, 60, Some(at(20, 0)))],
        )
        .unwrap();
    assert_eq!(value.clear_samples(), 1);
    assert!(value.is_empty());
    assert_eq!(value.clear_samples(), 0);
    value.select_account(identity()).unwrap();
    assert_eq!(value.account_fingerprint, fingerprint);
    assert_eq!(value.sample_count(), 1);
}
