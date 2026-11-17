use crate::app::refresh::refresh_completed_since;
use crate::app::refresh::refresh_result_is_current;
use crate::app::refresh::scheduled_deadline_is_current;
use crate::app::state::AppState;
use crate::app::state::HistoryRefreshEffect;
use crate::app::state::RefreshResult;
use crate::domain::settings::StoredSettings;
use crate::domain::usage::DashboardSnapshot;
use crate::domain::usage::DashboardStatus;
use crate::infra::usage::UsageClient;
use chrono::Duration as ChronoDuration;
use chrono::Utc;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[test]
fn stale_refresh_result_cannot_publish_after_a_newer_snapshot_commit() {
    let snapshot = DashboardSnapshot::default();
    let current_result = RefreshResult {
        started: std::time::Instant::now(),
        snapshot: snapshot.clone(),
        was_successful: true,
        should_emit: true,
        history_effect: HistoryRefreshEffect::default(),
        quota_auto_continue_changed: false,
        reset_credit_observation: None,
    };
    assert!(refresh_result_is_current(&current_result, &snapshot));

    let mut newer_snapshot = snapshot.clone();
    newer_snapshot.status = DashboardStatus::Stale;
    assert!(!refresh_result_is_current(&current_result, &newer_snapshot));

    let reused_result = RefreshResult {
        should_emit: false,
        ..current_result
    };
    assert!(!refresh_result_is_current(&reused_result, &snapshot));
}

#[test]
fn canceled_schedule_revisions_and_deadlines_are_never_current() {
    let first = Some(Utc::now() + ChronoDuration::minutes(1));
    let replacement = Some(Utc::now() + ChronoDuration::minutes(5));
    assert!(scheduled_deadline_is_current(4, 4, first, first));
    assert!(!scheduled_deadline_is_current(4, 5, first, first));
    assert!(!scheduled_deadline_is_current(4, 4, first, replacement));

    assert!(!refresh_completed_since(9, 9, 1, 1));
    assert!(refresh_completed_since(9, 10, 1, 1));
    assert!(!refresh_completed_since(9, 10, 1, 2));
}

#[tokio::test]
async fn rescheduling_uses_current_fixed_interval_and_failure_backoff() {
    let state = AppState::new(
        UsageClient::new().unwrap(),
        StoredSettings::default(),
        PathBuf::from("unused-schedule-test-settings.json"),
    )
    .unwrap();
    state.stored_settings().preferences.refresh_interval_seconds = 1_800;
    let schedule_guard = state.schedule_guard.lock().await;
    let before = Utc::now();
    let fixed = state.reschedule_from_now_locked().await;
    let after = Utc::now();
    let fixed_deadline = fixed.next_refresh_at.unwrap();
    assert!(fixed_deadline >= before + ChronoDuration::seconds(1_800));
    assert!(fixed_deadline <= after + ChronoDuration::seconds(1_800));

    state.consecutive_failures.store(2, Ordering::Relaxed);
    let before = Utc::now();
    let retry = state.reschedule_from_now_locked().await;
    let after = Utc::now();
    let retry_deadline = retry.next_refresh_at.unwrap();
    assert!(retry_deadline >= before + ChronoDuration::seconds(180));
    assert!(retry_deadline <= after + ChronoDuration::seconds(180));
    drop(schedule_guard);
}

#[tokio::test]
async fn concurrent_refresh_waits_for_the_single_flight_result_without_rebroadcasting_cache() {
    let state = Arc::new(
        AppState::new(
            UsageClient::new().unwrap(),
            StoredSettings::default(),
            PathBuf::from("unused-test-settings.json"),
        )
        .unwrap(),
    );
    let expected = DashboardSnapshot {
        account_id: None,
        status: DashboardStatus::Ready,
        account_email_masked: Some("p***@example.com".to_owned()),
        extra_credits: None,
        plan_label: Some("Pro".to_owned()),
        refreshed_at: None,
        next_refresh_at: None,
        message: None,
        quota_windows: Vec::new(),
    };
    let first_refresh = state.refresh_guard.lock().await;
    let contender = {
        let state = state.clone();
        tokio::spawn(async move { state.refresh_dashboard().await })
    };
    tokio::task::yield_now().await;
    *state.snapshot.lock().await = expected.clone();
    state.refresh_generation.fetch_add(1, Ordering::Release);
    drop(first_refresh);

    let result = contender.await.unwrap();
    assert_eq!(result.snapshot, expected);
    assert!(!result.should_emit);
    assert!(!result.was_successful);
}
