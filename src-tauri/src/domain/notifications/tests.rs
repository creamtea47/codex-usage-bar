use super::*;
use crate::domain::quota_reset::confirmed_event;
use crate::domain::quota_reset::generation_id;
use crate::domain::quota_reset::QuotaResetReason;
use chrono::{DateTime, Utc};

fn at(unix_seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(unix_seconds, 0).unwrap()
}

fn policy() -> NotificationPolicy {
    NotificationPolicy {
        enabled: true,
        ..NotificationPolicy::default()
    }
}

fn window(key: &str, remaining_percent: u8) -> NotificationWindow {
    NotificationWindow {
        key: key.to_owned(),
        remaining_percent,
        reset_at_unix_seconds: Some(1_000),
        start_at_unix_seconds: Some(0),
        window_seconds: 1_000,
        is_long_period: false,
    }
}

fn evaluate(
    tracker: &mut NotificationTracker,
    windows: &[NotificationWindow],
    observed_at_unix_seconds: i64,
    local_minute_of_day: u16,
    policy: &NotificationPolicy,
) -> Option<NotificationBatch> {
    tracker.evaluate(
        &NotificationSnapshot {
            observed_at_unix_seconds,
            local_minute_of_day,
            windows,
        },
        policy,
        |_| false,
    )
}

fn evaluate_with_resets<F>(
    tracker: &mut NotificationTracker,
    windows: &[NotificationWindow],
    observed_at_unix_seconds: i64,
    local_minute_of_day: u16,
    policy: &NotificationPolicy,
    confirmed_resets: &[MatchedQuotaResetEvent<'_>],
    reset_event_is_quiet: F,
) -> Option<NotificationBatch>
where
    F: Fn(i64) -> bool,
{
    tracker.evaluate_with_confirmed_resets(
        &NotificationSnapshot {
            observed_at_unix_seconds,
            local_minute_of_day,
            windows,
        },
        policy,
        confirmed_resets,
        reset_event_is_quiet,
    )
}

fn reset_event(
    sequence: u64,
    detected_at: i64,
    reason: QuotaResetReason,
    previous_remaining_percent: u8,
    current_remaining_percent: u8,
) -> QuotaResetEvent {
    let generation_id = generation_id(
        "test-salt",
        "test-account-fingerprint",
        "test-window-fingerprint",
        at(1_000),
        sequence,
    );
    confirmed_event(
        "test-salt",
        generation_id,
        "test-window-fingerprint".to_owned(),
        1_000,
        reason,
        at(detected_at),
        at(1_000),
        Some(at(2_000)),
        previous_remaining_percent,
        current_remaining_percent,
    )
}

#[test]
fn first_success_only_builds_a_baseline() {
    let mut tracker = NotificationTracker::new();
    let initial = [window("weekly", 10)];

    assert_eq!(
        evaluate(&mut tracker, &initial, 100, 12 * 60, &policy()),
        None
    );
    assert_eq!(
        evaluate(&mut tracker, &initial, 200, 12 * 60, &policy()),
        None
    );
}

#[test]
fn first_success_still_notifies_a_reset_confirmed_on_that_refresh() {
    let mut tracker = NotificationTracker::new();
    let current = [window("weekly", 100)];
    let event = reset_event(1, 200, QuotaResetReason::QuotaRecovered, 99, 100);
    let matched = [MatchedQuotaResetEvent {
        window_index: 0,
        event: &event,
    }];

    let notification = evaluate_with_resets(
        &mut tracker,
        &current,
        200,
        12 * 60,
        &policy(),
        &matched,
        |_| false,
    )
    .expect("a reset newly confirmed after restart must not be swallowed by baseline");
    assert_eq!(
        notification.items[0].reasons,
        vec![NotificationReason::Reset]
    );

    // 识别器即使在下一刷新重复携带同一事件，通知侧也只能消费一次。
    assert_eq!(
        evaluate_with_resets(
            &mut tracker,
            &current,
            201,
            12 * 60,
            &policy(),
            &matched,
            |_| false,
        ),
        None
    );
}

#[test]
fn low_remaining_fires_only_once_per_cycle_and_honors_boundaries() {
    let mut tracker = NotificationTracker::new();
    let mut current = [window("weekly", 21)];
    let mut rules = policy();
    rules.low_remaining_threshold = 20;
    evaluate(&mut tracker, &current, 100, 0, &rules);

    current[0].remaining_percent = 20;
    let notification = evaluate(&mut tracker, &current, 200, 0, &rules).unwrap();
    assert_eq!(
        notification.items[0].reasons,
        vec![NotificationReason::LowRemaining {
            remaining_percent: 20
        }]
    );

    current[0].remaining_percent = 0;
    assert_eq!(evaluate(&mut tracker, &current, 300, 0, &rules), None);

    let mut zero_tracker = NotificationTracker::new();
    let mut zero_window = [window("short", 1)];
    rules.low_remaining_threshold = 0;
    evaluate(&mut zero_tracker, &zero_window, 100, 0, &rules);
    zero_window[0].remaining_percent = 0;
    assert!(evaluate(&mut zero_tracker, &zero_window, 200, 0, &rules).is_some());
}

#[test]
fn pace_deficit_fires_once_when_a_long_window_falls_behind() {
    let mut tracker = NotificationTracker::new();
    let mut current = [window("weekly", 80)];
    current[0].is_long_period = true;
    let rules = policy();
    evaluate(&mut tracker, &current, 200, 0, &rules); // suggested: 80

    current[0].remaining_percent = 49;
    let notification = evaluate(&mut tracker, &current, 400, 0, &rules).unwrap();
    assert_eq!(
        notification.items[0].reasons,
        vec![NotificationReason::PaceDeficit { deficit_points: 11 }]
    );

    current[0].remaining_percent = 25;
    assert_eq!(evaluate(&mut tracker, &current, 500, 0, &rules), None);
}

#[test]
fn confirmed_reset_fires_once_and_rearms_thresholds_for_the_new_cycle() {
    let mut tracker = NotificationTracker::new();
    let mut current = [window("weekly", 80)];
    let rules = policy();
    evaluate(&mut tracker, &current, 100, 0, &rules);

    current[0].reset_at_unix_seconds = Some(2_000);
    current[0].start_at_unix_seconds = Some(1_000);
    current[0].remaining_percent = 100;
    let event = reset_event(1, 1_100, QuotaResetReason::DeadlineReached, 80, 100);
    let matched = [MatchedQuotaResetEvent {
        window_index: 0,
        event: &event,
    }];
    let notification =
        evaluate_with_resets(&mut tracker, &current, 1_100, 0, &rules, &matched, |_| {
            false
        })
        .unwrap();
    assert_eq!(
        notification.items[0].reasons,
        vec![NotificationReason::Reset]
    );
    assert_eq!(
        evaluate_with_resets(&mut tracker, &current, 1_200, 0, &rules, &matched, |_| {
            false
        },),
        None
    );

    current[0].remaining_percent = 20;
    assert!(evaluate(&mut tracker, &current, 1_300, 0, &rules).is_some());
}

#[test]
fn reset_at_changes_alone_never_bypass_the_unified_recognizer() {
    let mut tracker = NotificationTracker::new();
    let mut current = [window("weekly", 80)];
    let rules = policy();
    evaluate(&mut tracker, &current, 100, 0, &rules);

    current[0].reset_at_unix_seconds = Some(1_100);
    current[0].start_at_unix_seconds = Some(100);
    current[0].remaining_percent = 100;
    assert_eq!(evaluate(&mut tracker, &current, 900, 0, &rules), None);

    // 即使截止时间真正过去，通知模块也等待统一识别器给出确认事件。
    current[0].reset_at_unix_seconds = Some(2_100);
    current[0].start_at_unix_seconds = Some(1_100);
    assert_eq!(evaluate(&mut tracker, &current, 1_100, 0, &rules), None);

    let event = reset_event(1, 1_100, QuotaResetReason::DeadlineReached, 80, 100);
    let matched = [MatchedQuotaResetEvent {
        window_index: 0,
        event: &event,
    }];
    assert_eq!(
        evaluate_with_resets(&mut tracker, &current, 1_100, 0, &rules, &matched, |_| {
            false
        },)
        .unwrap()
        .items[0]
            .reasons,
        vec![NotificationReason::Reset]
    );
}

#[test]
fn same_reset_at_quota_recovery_from_98_or_99_to_100_is_notified() {
    for (sequence, previous) in [(1, 98), (2, 99)] {
        let mut tracker = NotificationTracker::new();
        let mut current = [window("weekly", previous)];
        let rules = policy();
        evaluate(&mut tracker, &current, 100, 0, &rules);

        // reset_at 保持不变；是否重置完全由统一事件决定。
        current[0].remaining_percent = 100;
        let event = reset_event(
            sequence,
            200,
            QuotaResetReason::QuotaRecovered,
            previous,
            100,
        );
        let matched = [MatchedQuotaResetEvent {
            window_index: 0,
            event: &event,
        }];
        let notification =
            evaluate_with_resets(&mut tracker, &current, 200, 0, &rules, &matched, |_| false)
                .unwrap();
        assert_eq!(
            notification.items[0].reasons,
            vec![NotificationReason::Reset]
        );
    }
}

#[test]
fn generation_dedupe_blocks_a_second_event_id_for_the_same_cycle() {
    let mut tracker = NotificationTracker::new();
    let mut current = [window("weekly", 99)];
    let rules = policy();
    evaluate(&mut tracker, &current, 100, 0, &rules);
    current[0].remaining_percent = 100;

    let first = reset_event(1, 200, QuotaResetReason::QuotaRecovered, 99, 100);
    let mut duplicate_generation = first.clone();
    duplicate_generation.event_id = "different-event-id".to_owned();
    duplicate_generation.detected_at = at(201);
    let first_match = [MatchedQuotaResetEvent {
        window_index: 0,
        event: &first,
    }];
    assert!(
        evaluate_with_resets(&mut tracker, &current, 200, 0, &rules, &first_match, |_| {
            false
        },)
        .is_some()
    );

    let duplicate_match = [MatchedQuotaResetEvent {
        window_index: 0,
        event: &duplicate_generation,
    }];
    assert_eq!(
        evaluate_with_resets(
            &mut tracker,
            &current,
            201,
            0,
            &rules,
            &duplicate_match,
            |_| false,
        ),
        None
    );
}

#[test]
fn settings_baseline_does_not_forget_consumed_reset_identity() {
    let mut tracker = NotificationTracker::new();
    let current = [window("weekly", 100)];
    let first = reset_event(1, 200, QuotaResetReason::QuotaRecovered, 99, 100);
    let first_match = [MatchedQuotaResetEvent {
        window_index: 0,
        event: &first,
    }];
    assert!(evaluate_with_resets(
        &mut tracker,
        &current,
        200,
        0,
        &policy(),
        &first_match,
        |_| false,
    )
    .is_some());

    tracker.reset_baseline();
    assert_eq!(
        evaluate_with_resets(
            &mut tracker,
            &current,
            201,
            0,
            &policy(),
            &first_match,
            |_| false,
        ),
        None
    );

    // 新一轮基线刷新中的新事件仍必须透传，不能把所有 reset 都当成历史。
    tracker.reset_baseline();
    let next = reset_event(2, 202, QuotaResetReason::QuotaRecovered, 99, 100);
    let next_match = [MatchedQuotaResetEvent {
        window_index: 0,
        event: &next,
    }];
    assert!(evaluate_with_resets(
        &mut tracker,
        &current,
        202,
        0,
        &policy(),
        &next_match,
        |_| false,
    )
    .is_some());
}

#[test]
fn multiple_windows_are_merged_and_limited_to_three_items() {
    let mut tracker = NotificationTracker::new();
    let mut current = [
        window("one", 21),
        window("two", 21),
        window("three", 21),
        window("four", 21),
    ];
    let rules = policy();
    evaluate(&mut tracker, &current, 100, 0, &rules);
    for item in &mut current {
        item.remaining_percent = 20;
    }

    let notification = evaluate(&mut tracker, &current, 200, 0, &rules).unwrap();
    assert_eq!(
        notification
            .items
            .iter()
            .map(|item| item.window_index)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(notification.omitted_window_count, 1);
}

#[test]
fn cross_midnight_quiet_hours_consume_events_without_replay() {
    let quiet_hours = QuietHours {
        enabled: true,
        start_minute: 22 * 60,
        end_minute: 8 * 60,
    };
    assert!(quiet_hours.contains(22 * 60));
    assert!(quiet_hours.contains(23 * 60));
    assert!(quiet_hours.contains(7 * 60 + 59));
    assert!(!quiet_hours.contains(8 * 60));
    assert!(!quiet_hours.contains(12 * 60));

    let mut rules = policy();
    rules.quiet_hours = quiet_hours;
    let mut tracker = NotificationTracker::new();
    let mut current = [window("weekly", 21)];
    evaluate(&mut tracker, &current, 100, 21 * 60, &rules);
    current[0].remaining_percent = 20;

    assert_eq!(evaluate(&mut tracker, &current, 200, 23 * 60, &rules), None);
    assert_eq!(evaluate(&mut tracker, &current, 300, 9 * 60, &rules), None);
}

#[test]
fn reset_that_occurred_during_quiet_hours_is_not_replayed_after_outage() {
    let mut tracker = NotificationTracker::new();
    let mut current = [window("weekly", 80)];
    let rules = policy();
    evaluate(&mut tracker, &current, 100, 12 * 60, &rules);

    current[0].reset_at_unix_seconds = Some(2_000);
    current[0].start_at_unix_seconds = Some(1_000);
    current[0].remaining_percent = 100;
    let event = reset_event(1, 1_000, QuotaResetReason::DeadlineReached, 80, 100);
    let matched = [MatchedQuotaResetEvent {
        window_index: 0,
        event: &event,
    }];
    assert_eq!(
        evaluate_with_resets(
            &mut tracker,
            &current,
            1_100,
            8 * 60 + 10,
            &rules,
            &matched,
            |event_at| event_at == 1_000,
        ),
        None
    );
    assert_eq!(
        evaluate_with_resets(
            &mut tracker,
            &current,
            1_101,
            8 * 60 + 11,
            &rules,
            &matched,
            |_| false,
        ),
        None
    );
}

#[test]
fn reset_observed_while_current_quiet_hours_are_active_is_consumed() {
    let mut rules = policy();
    rules.quiet_hours = QuietHours {
        enabled: true,
        start_minute: 22 * 60,
        end_minute: 8 * 60,
    };
    let mut tracker = NotificationTracker::new();
    let mut current = [window("weekly", 99)];
    evaluate(&mut tracker, &current, 100, 21 * 60, &rules);
    current[0].remaining_percent = 100;
    let event = reset_event(1, 200, QuotaResetReason::QuotaRecovered, 99, 100);
    let matched = [MatchedQuotaResetEvent {
        window_index: 0,
        event: &event,
    }];

    assert_eq!(
        evaluate_with_resets(
            &mut tracker,
            &current,
            200,
            23 * 60,
            &rules,
            &matched,
            |_| false,
        ),
        None
    );
    assert_eq!(
        evaluate_with_resets(
            &mut tracker,
            &current,
            201,
            9 * 60,
            &rules,
            &matched,
            |_| false,
        ),
        None
    );
}

#[test]
fn equal_quiet_hour_endpoints_mean_an_empty_interval() {
    let mut rules = policy();
    rules.quiet_hours = QuietHours {
        enabled: true,
        start_minute: 8 * 60,
        end_minute: 8 * 60,
    };
    assert!(!rules.quiet_hours.contains(8 * 60));

    let mut tracker = NotificationTracker::new();
    let mut current = [window("weekly", 21)];
    evaluate(&mut tracker, &current, 100, 8 * 60, &rules);
    current[0].remaining_percent = 20;
    assert!(evaluate(&mut tracker, &current, 200, 8 * 60, &rules).is_some());
}

#[test]
fn reset_baseline_suppresses_events_already_reached_under_new_settings() {
    let mut tracker = NotificationTracker::new();
    let mut current = [window("weekly", 30)];
    let mut rules = policy();
    rules.low_remaining_threshold = 20;
    evaluate(&mut tracker, &current, 100, 0, &rules);

    current[0].remaining_percent = 25;
    rules.low_remaining_threshold = 30;
    tracker.reset_baseline();
    assert_eq!(evaluate(&mut tracker, &current, 200, 0, &rules), None);
    current[0].remaining_percent = 20;
    assert_eq!(evaluate(&mut tracker, &current, 300, 0, &rules), None);
}

#[test]
fn disabled_policy_always_requires_a_fresh_baseline_when_reenabled() {
    let mut tracker = NotificationTracker::new();
    let mut current = [window("weekly", 30)];
    let enabled = policy();
    evaluate(&mut tracker, &current, 100, 0, &enabled);
    current[0].remaining_percent = 20;

    let disabled = NotificationPolicy::default();
    assert_eq!(evaluate(&mut tracker, &current, 200, 0, &disabled), None);
    assert_eq!(evaluate(&mut tracker, &current, 300, 0, &enabled), None);
}
