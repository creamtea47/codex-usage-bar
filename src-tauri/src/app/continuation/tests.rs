use super::*;
use crate::domain::history::generate_local_salt;
use crate::domain::usage::DashboardStatus;
use crate::domain::usage::QuotaFallbackLabel;
use crate::domain::usage::QuotaWindow;
use std::fs;

use std::{
    env,
    sync::{
        mpsc::{self},
        Arc, Barrier,
    },
    thread,
    time::SystemTime,
};

use crate::test_support::http::{mock_http_server, MockResponse};

fn mock_credentials() -> AuthCredentials {
    AuthCredentials {
        access_token: "test-access-token".to_owned(),
        account_id: Some("test-account-id".to_owned()),
    }
}

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

fn temp_path(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = env::temp_dir().join(format!("codex-auto-continue-{name}-{nonce}"));
    fs::create_dir_all(&directory).unwrap();
    directory.join(STATE_FILE_NAME)
}

fn cleanup(path: &Path) {
    if let Some(directory) = path.parent() {
        let _ = fs::remove_dir_all(directory);
    }
}

fn dashboard(reset_at: DateTime<Utc>) -> DashboardSnapshot {
    dashboard_with_remaining(reset_at, 100)
}

fn dashboard_with_remaining(reset_at: DateTime<Utc>, remaining_percent: u8) -> DashboardSnapshot {
    dashboard_with_window(reset_at, remaining_percent, 7 * 24 * 60 * 60)
}

fn dashboard_with_window(
    reset_at: DateTime<Utc>,
    remaining_percent: u8,
    window_seconds: i64,
) -> DashboardSnapshot {
    DashboardSnapshot {
        account_id: None,
        status: DashboardStatus::Ready,
        account_email_masked: None,
        extra_credits: None,
        plan_label: None,
        refreshed_at: Some(at(900)),
        next_refresh_at: None,
        message: None,
        quota_windows: vec![QuotaWindow {
            id: "weekly".to_owned(),
            label: None,
            fallback_label: QuotaFallbackLabel::Weekly,
            window_seconds,
            used_percent: 100_u8.saturating_sub(remaining_percent),
            remaining_percent,
            reset_at: Some(reset_at),
            reset_after_seconds: 0,
            start_at: None,
            show_pace_marker: true,
            forecast: None,
        }],
    }
}

#[test]
fn selects_first_text_model_without_a_hardcoded_preference() {
    let preferred = serde_json::json!({"models":[
        {"slug":"gpt-image-2"},
        {"slug":"gpt-5.6-sol"},
        {"slug":"gpt-5.4"}
    ]});
    assert_eq!(
        select_preferred_model(&preferred).as_deref(),
        Some("gpt-5.6-sol")
    );
    let first = serde_json::json!({"models":[{"slug":"gpt-image-2"},{"slug":"gpt-5.6-sol"}]});
    assert_eq!(
        select_preferred_model(&first).as_deref(),
        Some("gpt-5.6-sol")
    );
}

#[test]
fn payload_is_minimal_non_stored_streaming_greeting() {
    let payload = build_greeting_payload("gpt-5.4");
    assert_eq!(payload["model"], "gpt-5.4");
    assert_eq!(payload["input"][0]["content"][0]["text"], "hi");
    assert_eq!(payload["stream"], true);
    assert_eq!(payload["store"], false);
    assert!(!payload.to_string().contains("access_token"));
}

#[test]
fn requires_response_completed_in_sse() {
    assert_eq!(
        parse_sse_line(b"data: {\"type\":\"response.completed\"}\n"),
        SseSignal::Completed
    );
    assert_eq!(
        parse_sse_line(b"data: {\"type\":\"response.failed\"}\n"),
        SseSignal::Failed
    );
    assert_eq!(parse_sse_line(b"data: [DONE]\n"), SseSignal::Continue);
}

#[tokio::test]
async fn sends_required_identity_headers_and_minimal_non_stored_payload() {
    let (base, requests, server) = mock_http_server(vec![
        MockResponse::json(r#"{"models":[{"slug":"image-1"},{"slug":"gpt-5.4"}]}"#),
        MockResponse::sse("data: {\"type\":\"response.completed\"}\n\n"),
    ]);
    let client = QuotaAutoContinueClient::with_endpoints(
        format!("{base}/models"),
        format!("{base}/responses"),
    );

    let selected = client
        .send_greeting_with_credentials(mock_credentials(), None)
        .await
        .unwrap();
    assert_eq!(selected, "gpt-5.4");

    let models_request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
    let response_request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
    server.join().unwrap();
    let models_lower = models_request.to_ascii_lowercase();
    assert!(models_lower.starts_with("get /models?client_version=0.146.0"));
    assert!(models_lower.contains("authorization: bearer test-access-token"));
    assert!(models_lower.contains("chatgpt-account-id: test-account-id"));
    assert!(models_lower.contains("originator: codex_cli_rs"));

    let response_lower = response_request.to_ascii_lowercase();
    assert!(response_lower.starts_with("post /responses"));
    assert!(response_lower.contains("accept: text/event-stream"));
    assert!(response_lower.contains("openai-beta: responses=experimental"));
    assert!(response_lower.contains("version: 0.146.0"));
    let body = response_request.split("\r\n\r\n").nth(1).unwrap();
    let payload: Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["input"][0]["content"][0]["text"], "hi");
    assert_eq!(payload["stream"], true);
    assert_eq!(payload["store"], false);
    assert_eq!(payload["model"], "gpt-5.4");
}

#[tokio::test]
async fn manifest_failure_does_not_send_a_fallback_request() {
    let (base, requests, server) =
        mock_http_server(vec![MockResponse::status("503 Service Unavailable")]);
    let client = QuotaAutoContinueClient::with_endpoints(
        format!("{base}/models"),
        format!("{base}/responses"),
    );
    let error = client
        .send_greeting_with_credentials(mock_credentials(), None)
        .await
        .unwrap_err();
    assert_eq!(error.code, QuotaAutoContinueErrorCode::ServiceUnavailable);
    assert!(error.model.is_none());
    assert!(requests
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .starts_with("GET"));
    server.join().unwrap();
    assert!(requests.try_recv().is_err());
}

#[tokio::test]
async fn maps_auth_and_rate_limit_failures_to_stable_result_codes() {
    let (auth_base, _requests, auth_server) =
        mock_http_server(vec![MockResponse::status("401 Unauthorized")]);
    let auth_client = QuotaAutoContinueClient::with_endpoints(
        format!("{auth_base}/models"),
        format!("{auth_base}/responses"),
    );
    let auth_failure = auth_client
        .send_greeting_with_credentials(mock_credentials(), None)
        .await
        .unwrap_err();
    assert_eq!(auth_failure.code, QuotaAutoContinueErrorCode::AuthInvalid);
    assert!(auth_failure.model.is_none());
    auth_server.join().unwrap();

    let (limit_base, _requests, limit_server) = mock_http_server(vec![
        MockResponse::json(r#"{"models":[{"slug":"text-model"}]}"#),
        MockResponse::status("429 Too Many Requests"),
    ]);
    let limit_client = QuotaAutoContinueClient::with_endpoints(
        format!("{limit_base}/models"),
        format!("{limit_base}/responses"),
    );
    let limit_failure = limit_client
        .send_greeting_with_credentials(mock_credentials(), None)
        .await
        .unwrap_err();
    assert_eq!(limit_failure.code, QuotaAutoContinueErrorCode::RateLimited);
    assert_eq!(limit_failure.model.as_deref(), Some("text-model"));
    limit_server.join().unwrap();
}

#[tokio::test]
async fn rejects_failed_incomplete_and_oversized_response_streams() {
    for body in [
        "data: {\"type\":\"response.failed\"}\n\n",
        "data: {\"type\":\"response.output_text.delta\"}\n\n",
    ] {
        let (base, _requests, server) = mock_http_server(vec![
            MockResponse::json(r#"{"models":[{"slug":"text-model"}]}"#),
            MockResponse::sse(body),
        ]);
        let client = QuotaAutoContinueClient::with_endpoints(
            format!("{base}/models"),
            format!("{base}/responses"),
        );
        let failure = client
            .send_greeting_with_credentials(mock_credentials(), None)
            .await
            .unwrap_err();
        assert_eq!(failure.code, QuotaAutoContinueErrorCode::InvalidResponse);
        server.join().unwrap();
    }

    let mut oversized = MockResponse::sse("");
    oversized.declared_length = Some(MAX_RESPONSE_BYTES + 1);
    let (base, _requests, server) = mock_http_server(vec![
        MockResponse::json(r#"{"models":[{"slug":"text-model"}]}"#),
        oversized,
    ]);
    let client = QuotaAutoContinueClient::with_endpoints(
        format!("{base}/models"),
        format!("{base}/responses"),
    );
    let failure = client
        .send_greeting_with_credentials(mock_credentials(), None)
        .await
        .unwrap_err();
    assert_eq!(failure.code, QuotaAutoContinueErrorCode::InvalidResponse);
    server.join().unwrap();
}

#[tokio::test]
async fn enforces_total_request_timeout_without_retrying_inside_one_slot() {
    let mut delayed_manifest = MockResponse::json(r#"{"models":[{"slug":"text-model"}]}"#);
    delayed_manifest.delay_millis = 800;
    let (base, requests, server) = mock_http_server(vec![delayed_manifest]);
    let client = QuotaAutoContinueClient::with_endpoints(
        format!("{base}/models"),
        format!("{base}/responses"),
    )
    .with_test_timeout(Duration::from_millis(250));
    let failure = client
        .send_greeting_with_credentials(mock_credentials(), None)
        .await
        .unwrap_err();
    assert_eq!(failure.code, QuotaAutoContinueErrorCode::Network);
    assert!(requests.recv_timeout(Duration::from_secs(1)).is_ok());
    server.join().unwrap();
}

fn establish_recovered_reset(
    runtime: &QuotaAutoContinueRuntime,
    enabled: bool,
    old_reset_at: DateTime<Utc>,
    detected_at: DateTime<Utc>,
) -> QuotaResetEvent {
    let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
    runtime.observe_dashboard(
        enabled,
        &identity,
        &dashboard_with_remaining(old_reset_at, 99),
        detected_at - ChronoDuration::seconds(100),
    );
    runtime.observe_dashboard(
        enabled,
        &identity,
        &dashboard_with_remaining(
            old_reset_at + ChronoDuration::seconds(7 * 24 * 60 * 60),
            100,
        ),
        detected_at,
    );
    runtime.pending_reset_event(detected_at).unwrap()
}

#[test]
fn interrupted_or_uncertain_send_never_rearms_automatic_slots() {
    let path = temp_path("interrupted-send");
    let now = Utc::now();
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    establish_recovered_reset(&runtime, true, now + ChronoDuration::hours(1), now);
    assert!(runtime.claim_due_attempt(true, now).unwrap().is_some());
    drop(runtime);
    let recovered = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    assert!(recovered.status(true, now).next_attempt_at.is_none());
    assert!(
        recovered
            .persisted()
            .last_automatic_result
            .as_ref()
            .unwrap()
            .details
            .as_ref()
            .unwrap()
            .delivery_uncertain
    );
    // 手动请求失联同样不能让已确认事件由后台接着重发。
    *recovered.client.details.lock().unwrap() = Some(ResponseDetails {
        delivery_uncertain: true,
        ..Default::default()
    });
    recovered.record_manual_failure(&SendFailure {
        code: QuotaAutoContinueErrorCode::Network,
        model: Some("test".into()),
    });
    assert!(
        recovered
            .persisted()
            .active_cycle
            .as_ref()
            .unwrap()
            .request_completed
    );
    let _ = fs::remove_file(path);
}

#[test]
fn schedules_immediate_and_three_retry_slots_without_back_to_back_catchup() {
    let path = temp_path("schedule");
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    let event = establish_recovered_reset(&runtime, true, at(1_000), at(1_000));
    assert_eq!(event.reason, QuotaResetReason::QuotaRecovered);
    assert_eq!(
        runtime.status(true, at(1_000)).next_attempt_at,
        Some(at(1_000))
    );

    let immediate = runtime.claim_due_attempt(true, at(1_000)).unwrap().unwrap();
    assert_eq!(immediate.slot_index, 0);
    runtime.finish_failure(&immediate, QuotaAutoContinueErrorCode::Network, None);
    assert_eq!(
        runtime.status(true, at(1_001)).next_attempt_at,
        Some(at(1_060))
    );

    // 睡眠到 +6 分钟时只执行最近的 +5 分钟槽，并跳过 +1 分钟槽。
    let resumed = runtime.claim_due_attempt(true, at(1_360)).unwrap().unwrap();
    assert_eq!(resumed.slot_index, 2);
    assert_eq!(runtime.status(true, at(1_360)).attempted_count, 3);
    runtime.finish_failure(&resumed, QuotaAutoContinueErrorCode::Network, None);
    assert_eq!(
        runtime.status(true, at(1_361)).next_attempt_at,
        Some(at(2_800))
    );
    cleanup(&path);
}

#[test]
fn unchanged_deadline_recovery_confirms_event_and_claims_slot_zero() {
    let path = temp_path("unchanged-deadline-recovery");
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
    runtime.observe_dashboard(
        true,
        &identity,
        &dashboard_with_remaining(at(1_000), 98),
        at(900),
    );
    runtime.observe_dashboard(
        true,
        &identity,
        &dashboard_with_remaining(at(1_000), 100),
        at(950),
    );

    let event = runtime.pending_reset_event(at(950)).unwrap();
    assert_eq!(event.reason, QuotaResetReason::QuotaRecovered);
    assert_eq!(event.expected_reset_at, at(1_000));
    assert_eq!(event.next_reset_at, Some(at(1_000)));
    let attempt = runtime.claim_due_attempt(true, at(950)).unwrap().unwrap();
    assert_eq!(attempt.slot_index, 0);
    cleanup(&path);
}

#[test]
fn any_early_deadline_advance_stays_pending_without_overwriting_active_target() {
    let path = temp_path("small-deadline-correction");
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
    runtime.observe_dashboard(
        true,
        &identity,
        &dashboard_with_remaining(at(1_000), 80),
        at(900),
    );
    runtime.observe_dashboard(
        true,
        &identity,
        &dashboard_with_remaining(at(1_100), 80),
        at(950),
    );

    assert_eq!(
        runtime.status(true, at(950)).target_reset_at,
        Some(at(1_000))
    );
    assert!(runtime.pending_reset_event(at(950)).is_none());
    let (state, _) = load_runtime_state(&path);
    assert_eq!(state.pending_reset_at, Some(at(1_100)));
    assert!(runtime.claim_due_attempt(true, at(950)).unwrap().is_none());
    cleanup(&path);
}

#[test]
fn claims_all_four_retry_slots_at_exact_offsets() {
    let path = temp_path("all-retry-slots");
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    establish_recovered_reset(&runtime, true, at(1_000), at(1_000));

    for (slot_index, attempted_at) in [1_000, 1_060, 1_300, 2_800].into_iter().enumerate() {
        let attempt = runtime
            .claim_due_attempt(true, at(attempted_at))
            .unwrap()
            .unwrap();
        assert_eq!(attempt.slot_index, slot_index);
        runtime.finish_failure(&attempt, QuotaAutoContinueErrorCode::Network, None);
    }
    let status = runtime.status(true, at(2_800));
    assert_eq!(status.attempted_count, 4);
    assert_eq!(status.phase, QuotaAutoContinuePhase::Failed);
    assert!(status.next_attempt_at.is_none());
    let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
    runtime.observe_dashboard(
        true,
        &identity,
        &dashboard(at(1_000 + 7 * 24 * 60 * 60)),
        at(2_801),
    );
    assert_eq!(
        runtime.status(true, at(2_801)).phase,
        QuotaAutoContinuePhase::Failed
    );
    let audit = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("quota-audit-")
        })
        .filter_map(|entry| fs::read_to_string(entry.path()).ok())
        .collect::<String>();
    assert!(!audit.contains("cycleMissed"));
    cleanup(&path);
}

#[test]
fn completed_send_does_not_rearm_on_repeated_full_observations() {
    let path = temp_path("completed-no-rearm");
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    let first = establish_recovered_reset(&runtime, true, at(1_000), at(1_000));
    let attempt = runtime.claim_due_attempt(true, at(1_000)).unwrap().unwrap();
    runtime.finish_sent(&attempt, "gpt-5.4".to_owned(), at(1_001));

    let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
    let next_reset = at(1_000 + 7 * 24 * 60 * 60);
    runtime.observe_dashboard(true, &identity, &dashboard(next_reset), at(1_002));
    let still_pending = runtime.pending_reset_event(at(1_002)).unwrap();
    assert_eq!(still_pending.event_id, first.event_id);
    assert_eq!(still_pending.generation_id, first.generation_id);
    assert!(runtime
        .claim_due_attempt(true, at(1_060))
        .unwrap()
        .is_none());
    assert_eq!(runtime.status(true, at(1_060)).attempted_count, 1);
    cleanup(&path);
}

#[test]
fn promoted_cycle_keeps_event_lock_during_post_success_quota_jitter() {
    let path = temp_path("promoted-lock");
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    let first = establish_recovered_reset(&runtime, true, at(10_000), at(100));
    let attempt = runtime.claim_due_attempt(true, at(100)).unwrap().unwrap();
    runtime.finish_sent(&attempt, "gpt-5.4".to_owned(), at(101));
    assert!(runtime.mark_notification_claimed(&first.event_id, at(102)));
    assert!(runtime.mark_notification_queued(&first.event_id, at(102)));

    let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
    let next_reset = at(10_000 + 7 * 24 * 60 * 60);
    runtime.observe_dashboard(true, &identity, &dashboard(next_reset), at(103));
    let (promoted, _) = load_runtime_state(&path);
    assert!(promoted
        .active_cycle
        .as_ref()
        .unwrap()
        .confirmed_event
        .is_none());
    let lock = promoted.last_event_lock.unwrap();
    assert_eq!(lock.event_id, first.event_id);
    assert_eq!(lock.generation_id, first.generation_id);

    runtime.observe_dashboard(
        true,
        &identity,
        &dashboard_with_remaining(next_reset, 99),
        at(200),
    );
    runtime.observe_dashboard(
        true,
        &identity,
        &dashboard_with_remaining(next_reset, 100),
        at(201),
    );
    assert!(runtime.pending_reset_event(at(201)).is_none());
    assert_eq!(
        runtime.current_generation().unwrap().generation_id,
        first.generation_id
    );
    assert!(runtime.claim_due_attempt(true, at(201)).unwrap().is_none());
    cleanup(&path);
}

#[test]
fn repeated_recovery_edges_inside_lock_keep_the_same_event() {
    let path = temp_path("same-event-inside-lock");
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    let first = establish_recovered_reset(&runtime, true, at(10_000), at(100));
    let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
    let next_reset = at(10_000 + 7 * 24 * 60 * 60);

    for (remaining, observed_at) in [(99, 200), (100, 201), (99, 300), (100, 301)] {
        runtime.observe_dashboard(
            true,
            &identity,
            &dashboard_with_remaining(next_reset, remaining),
            at(observed_at),
        );
        let current = runtime.pending_reset_event(at(observed_at)).unwrap();
        assert_eq!(current.event_id, first.event_id);
        assert_eq!(current.generation_id, first.generation_id);
    }
    cleanup(&path);
}

#[test]
fn runtime_state_replaces_atomically_and_contains_only_redacted_v2_fields() {
    let path = temp_path("state-schema");
    let mut state = PersistedRuntimeState::fresh();
    state.start_cycle(
        "a".repeat(64),
        Some("b".repeat(64)),
        Some(7 * 24 * 60 * 60),
        Some(at(1_000)),
    );
    save_runtime_state(&path, &state).unwrap();
    let cycle = state.active_cycle.as_mut().unwrap();
    cycle.phase = QuotaAutoContinuePhase::WaitingForRetry;
    cycle.consumed_slots[0] = true;
    state.last_automatic_result = Some(QuotaAutoContinueResult {
        details: None,
        attempted_at: Some(at(1_000)),
        success_at: None,
        error_code: Some(QuotaAutoContinueErrorCode::Network),
        model: None,
        slot_index: Some(0),
    });
    save_runtime_state(&path, &state).unwrap();

    let bytes = fs::read(&path).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    let object = value.as_object().unwrap();
    let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "accountFingerprint",
            "activeCycle",
            "generationSequence",
            "lastAutomaticResult",
            "lastEventLock",
            "lastManualResult",
            "lastObservation",
            "lastResetDetectedAt",
            "lastTriggerReason",
            "migrationBaselinePending",
            "pendingResetAt",
            "salt",
            "schemaVersion",
            "windowFingerprint",
            "windowSeconds",
        ]
    );
    let serialized = String::from_utf8(bytes).unwrap().to_ascii_lowercase();
    for forbidden in [
        "access_token",
        "refresh_token",
        "account_id",
        "email",
        "authorization",
        "selectedmodel",
        "response.completed",
    ] {
        assert!(!serialized.contains(forbidden));
    }
    let (loaded, migrated) = load_runtime_state(&path);
    assert!(!migrated);
    assert_eq!(loaded.phase(), QuotaAutoContinuePhase::WaitingForRetry);
    assert!(!path.with_extension("json.tmp").exists());
    cleanup(&path);
}

#[test]
fn explicitly_migrates_v1_without_synthesizing_a_reset_event() {
    let path = temp_path("migrate-v1");
    let v1 = PersistedRuntimeStateV1 {
        schema_version: 1,
        salt: generate_local_salt(),
        account_fingerprint: Some("a".repeat(64)),
        window_fingerprint: Some("b".repeat(64)),
        target_reset_at: Some(at(1_000)),
        consumed_slots: [true, false, false, false],
        request_completed: false,
        phase: QuotaAutoContinuePhase::WaitingForRetry,
        last_attempt_at: Some(at(950)),
        last_success_at: None,
        last_error_code: Some(QuotaAutoContinueErrorCode::Network),
    };
    fs::write(&path, serde_json::to_vec_pretty(&v1).unwrap()).unwrap();

    let (migrated, was_migrated) = load_runtime_state(&path);
    assert!(was_migrated);
    assert_eq!(migrated.schema_version, STATE_SCHEMA_VERSION);
    assert_eq!(migrated.phase(), QuotaAutoContinuePhase::WaitingForRetry);
    assert!(migrated
        .active_cycle
        .as_ref()
        .unwrap()
        .confirmed_event
        .is_none());
    assert_eq!(migrated.last_automatic_result.unwrap().slot_index, Some(0));

    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    assert!(runtime.pending_reset_event(at(1_000)).is_none());
    let value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(value["schemaVersion"], STATE_SCHEMA_VERSION);
    cleanup(&path);
}

#[test]
fn first_snapshot_after_v1_migration_fills_actual_window_seconds_only() {
    let path = temp_path("migrate-v1-window-seconds");
    let salt = generate_local_salt();
    let window_seconds = 6 * 24 * 60 * 60;
    let account = account_fingerprint(&salt, AccountIdentity::AccountId("account-a")).unwrap();
    let window = quota_window_fingerprint(&salt, "weekly", window_seconds);
    let v1 = PersistedRuntimeStateV1 {
        schema_version: 1,
        salt,
        account_fingerprint: Some(account),
        window_fingerprint: Some(window),
        target_reset_at: Some(at(1_000)),
        consumed_slots: [true, false, false, false],
        request_completed: false,
        phase: QuotaAutoContinuePhase::WaitingForRetry,
        last_attempt_at: None,
        last_success_at: None,
        last_error_code: None,
    };
    fs::write(&path, serde_json::to_vec_pretty(&v1).unwrap()).unwrap();
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    let before = runtime.current_generation().unwrap();
    assert_eq!(before.window_seconds, 7 * 24 * 60 * 60);

    runtime.observe_dashboard(
        true,
        &UsageAccountIdentity::AccountId("account-a".to_owned()),
        &dashboard_with_window(at(1_000), 80, window_seconds),
        at(900),
    );
    let after = runtime.current_generation().unwrap();
    assert_eq!(after.generation_id, before.generation_id);
    assert_eq!(after.window_seconds, window_seconds);
    assert_eq!(runtime.status(true, at(900)).attempted_count, 1);
    assert_eq!(
        runtime.status(true, at(900)).phase,
        QuotaAutoContinuePhase::WaitingForRetry
    );
    assert!(runtime.pending_reset_event(at(900)).is_none());
    cleanup(&path);
}

#[test]
fn v1_expired_target_and_advanced_first_snapshot_create_baseline_only() {
    let path = temp_path("migrate-v1-expired-first-frame");
    let salt = generate_local_salt();
    let window_seconds = 7 * 24 * 60 * 60;
    let account = account_fingerprint(&salt, AccountIdentity::AccountId("account-a")).unwrap();
    let window = quota_window_fingerprint(&salt, "weekly", window_seconds);
    let v1 = PersistedRuntimeStateV1 {
        schema_version: 1,
        salt,
        account_fingerprint: Some(account),
        window_fingerprint: Some(window),
        target_reset_at: Some(at(1_000)),
        consumed_slots: [true, true, false, false],
        request_completed: false,
        phase: QuotaAutoContinuePhase::WaitingForRetry,
        last_attempt_at: Some(at(900)),
        last_success_at: None,
        last_error_code: Some(QuotaAutoContinueErrorCode::Network),
    };
    fs::write(&path, serde_json::to_vec_pretty(&v1).unwrap()).unwrap();
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    let current_reset_at = at(1_000 + window_seconds);

    runtime.observe_dashboard(
        true,
        &UsageAccountIdentity::AccountId("account-a".to_owned()),
        &dashboard_with_window(current_reset_at, 100, window_seconds),
        at(2_000),
    );

    let status = runtime.status(true, at(2_000));
    assert_eq!(status.target_reset_at, Some(current_reset_at));
    assert_eq!(status.attempted_count, 0);
    assert_eq!(status.phase, QuotaAutoContinuePhase::Scheduled);
    assert!(runtime.pending_reset_event(at(2_000)).is_none());
    assert!(runtime
        .claim_due_attempt(true, at(2_000))
        .unwrap()
        .is_none());
    let (state, _) = load_runtime_state(&path);
    assert!(!state.migration_baseline_pending);
    assert_eq!(state.last_observation.unwrap().remaining_percent, 100);
    cleanup(&path);
}

#[test]
fn misses_only_a_confirmed_event_after_thirty_minutes() {
    let path = temp_path("missed");
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    establish_recovered_reset(&runtime, true, at(1_000), at(1_000));
    assert!(runtime
        .claim_due_attempt(true, at(2_801))
        .unwrap()
        .is_none());
    assert_eq!(
        runtime.status(true, at(2_801)).phase,
        QuotaAutoContinuePhase::Missed
    );
    let reloaded = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    assert_eq!(
        reloaded.status(true, at(2_801)).phase,
        QuotaAutoContinuePhase::Missed
    );
    cleanup(&path);
}

#[test]
fn account_switch_replaces_old_schedule_and_preflight_rejects_old_attempt() {
    let path = temp_path("account-switch");
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    establish_recovered_reset(&runtime, true, at(1_000), at(1_000));
    let attempt = runtime.claim_due_attempt(true, at(1_000)).unwrap().unwrap();
    runtime.observe_dashboard(
        true,
        &UsageAccountIdentity::AccountId("account-b".to_owned()),
        &dashboard(at(2_000)),
        at(1_001),
    );
    assert_eq!(
        runtime.preflight_decision(&attempt),
        PreflightDecision::AccountChanged
    );
    assert_eq!(
        runtime.status(true, at(1_001)).target_reset_at,
        Some(at(2_000))
    );
    assert!(runtime.pending_reset_event(at(1_001)).is_none());
    cleanup(&path);
}

#[test]
fn deadline_advance_is_pending_until_old_deadline_then_confirms_once() {
    let path = temp_path("deadline-advanced");
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
    let next_reset = at(1_000 + 7 * 24 * 60 * 60);
    runtime.observe_dashboard(true, &identity, &dashboard(at(1_000)), at(900));
    let baseline_generation = runtime.current_generation().unwrap().generation_id;
    runtime.observe_dashboard(true, &identity, &dashboard(next_reset), at(950));
    assert_eq!(
        runtime.status(true, at(950)).target_reset_at,
        Some(at(1_000))
    );
    assert!(runtime.pending_reset_event(at(950)).is_none());
    assert!(runtime.claim_due_attempt(true, at(950)).unwrap().is_none());

    runtime.observe_dashboard(true, &identity, &dashboard(next_reset), at(1_000));
    let event = runtime.pending_reset_event(at(1_000)).unwrap();
    assert_eq!(event.reason, QuotaResetReason::DeadlineReached);
    assert_ne!(event.generation_id, baseline_generation);
    let attempt = runtime.claim_due_attempt(true, at(1_000)).unwrap().unwrap();
    assert_eq!(attempt.slot_index, 0);
    assert_eq!(
        runtime.preflight_decision(&attempt),
        PreflightDecision::Proceed
    );
    cleanup(&path);
}

#[test]
fn confirmed_generation_begins_on_recovery_snapshot_and_survives_promotion() {
    let path = temp_path("generation-transition");
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
    runtime.observe_dashboard(
        true,
        &identity,
        &dashboard_with_remaining(at(1_000), 98),
        at(900),
    );
    let baseline = runtime.current_generation().unwrap().generation_id;
    runtime.observe_dashboard(
        true,
        &identity,
        &dashboard_with_remaining(at(1_000 + 7 * 24 * 60 * 60), 100),
        at(950),
    );
    let event = runtime.pending_reset_event(at(950)).unwrap();
    assert_ne!(event.generation_id, baseline);
    assert_eq!(
        runtime.current_generation().unwrap().generation_id,
        event.generation_id
    );

    runtime.record_manual_success("gpt-5.4".to_owned(), at(951));
    assert!(runtime.mark_notification_claimed(&event.event_id, at(952)));
    assert!(runtime.mark_notification_queued(&event.event_id, at(952)));
    runtime.observe_dashboard(
        true,
        &identity,
        &dashboard(at(1_000 + 7 * 24 * 60 * 60)),
        at(953),
    );
    assert_eq!(
        runtime.current_generation().unwrap().generation_id,
        event.generation_id
    );
    assert_eq!(
        runtime.status(true, at(953)).target_reset_at,
        Some(at(1_000 + 7 * 24 * 60 * 60))
    );
    cleanup(&path);
}

#[test]
fn lock_expiry_allows_a_new_recovery_event_with_a_new_generation() {
    let path = temp_path("lock-expiry");
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    let first = establish_recovered_reset(&runtime, true, at(10_000), at(100));
    let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
    let next_reset = at(10_000 + 7 * 24 * 60 * 60);
    runtime.observe_dashboard(
        true,
        &identity,
        &dashboard_with_remaining(next_reset, 99),
        at(1_901),
    );
    runtime.observe_dashboard(
        true,
        &identity,
        &dashboard_with_remaining(next_reset, 100),
        at(1_902),
    );
    let second = runtime.pending_reset_event(at(1_902)).unwrap();
    assert_ne!(second.event_id, first.event_id);
    assert_ne!(second.generation_id, first.generation_id);
    assert_eq!(second.reason, QuotaResetReason::QuotaRecovered);
    cleanup(&path);
}

#[test]
fn disabled_confirmation_never_back_sends_historical_hi() {
    let path = temp_path("disabled-confirmation");
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
    let next_reset = at(1_000 + 7 * 24 * 60 * 60);
    runtime.observe_dashboard(false, &identity, &dashboard(at(1_000)), at(900));
    runtime.observe_dashboard(false, &identity, &dashboard(next_reset), at(1_000));
    let event = runtime.pending_reset_event(at(1_000)).unwrap();
    assert_eq!(event.reason, QuotaResetReason::DeadlineReached);
    assert!(runtime
        .claim_due_attempt(true, at(1_000))
        .unwrap()
        .is_none());
    assert!(runtime.mark_notification_claimed(&event.event_id, at(1_001)));
    assert!(runtime.mark_notification_queued(&event.event_id, at(1_001)));
    runtime.observe_dashboard(true, &identity, &dashboard(next_reset), at(1_002));
    assert!(runtime
        .claim_due_attempt(true, at(1_002))
        .unwrap()
        .is_none());
    assert_eq!(
        runtime.status(true, at(1_002)).target_reset_at,
        Some(next_reset)
    );
    cleanup(&path);
}

#[test]
fn disabling_after_confirmation_then_reenabling_never_sends_old_event() {
    let path = temp_path("disable-after-confirmation");
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    let event = establish_recovered_reset(&runtime, true, at(1_000), at(950));
    runtime.activate_cached_observation(false, at(951));
    assert_eq!(
        runtime.pending_reset_event(at(951)).unwrap().event_id,
        event.event_id
    );
    runtime.activate_cached_observation(true, at(952));
    assert!(runtime.claim_due_attempt(true, at(952)).unwrap().is_none());
    assert_eq!(
        runtime.status(true, at(952)).phase,
        QuotaAutoContinuePhase::Succeeded
    );
    cleanup(&path);
}

#[test]
fn pending_notification_survives_restart_and_claim_can_finish_failed() {
    let path = temp_path("notification-restart");
    let event_id = {
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        establish_recovered_reset(&runtime, true, at(1_000), at(1_000)).event_id
    };
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    // 即使重启跨过 30 分钟去重锁，未处置通知仍必须可领取。
    assert_eq!(
        runtime.pending_reset_event(at(3_001)).unwrap().event_id,
        event_id
    );
    assert!(runtime.mark_notification_claimed(&event_id, at(3_002)));
    assert!(runtime.pending_reset_event(at(3_002)).is_none());
    assert!(runtime.mark_notification_failed(&event_id, at(3_003)));
    assert!(!runtime.mark_notification_failed(&event_id, at(3_004)));
    let (state, _) = load_runtime_state(&path);
    assert_eq!(
        state
            .active_cycle
            .unwrap()
            .confirmed_event
            .unwrap()
            .notification_disposition,
        NotificationDisposition::Failed
    );
    let audit = fs::read_to_string(runtime.audit.path_for(at(3_003))).unwrap();
    assert!(audit
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .any(|record| record["action"] == "notificationClaimed"));
    assert!(!audit
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .any(|record| record["action"] == "notificationQueued"));
    assert!(audit
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .any(
            |record| record["action"] == "notificationFailed" && record["errorCode"] == "platform"
        ));
    cleanup(&path);
}

#[test]
fn notification_success_audit_is_queued_only_after_claim() {
    let path = temp_path("notification-queued-after-claim");
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    let event = establish_recovered_reset(&runtime, true, at(1_000), at(1_000));
    assert!(!runtime.mark_notification_queued(&event.event_id, at(1_001)));
    assert!(runtime.mark_notification_claimed(&event.event_id, at(1_002)));
    assert!(runtime.mark_notification_queued(&event.event_id, at(1_003)));
    let audit = fs::read_to_string(runtime.audit.path_for(at(1_003))).unwrap();
    let actions = audit
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|record| record["action"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let claimed = actions
        .iter()
        .position(|action| action == "notificationClaimed")
        .unwrap();
    let queued = actions
        .iter()
        .position(|action| action == "notificationQueued")
        .unwrap();
    assert!(claimed < queued);
    cleanup(&path);
}

#[test]
fn interrupted_notification_claim_recovers_failed_without_redelivery() {
    let path = temp_path("notification-claim-interrupted");
    let event_id = {
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let event = establish_recovered_reset(&runtime, true, at(1_000), at(1_000));
        assert!(runtime.mark_notification_claimed(&event.event_id, at(1_001)));
        event.event_id
    };

    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    assert!(runtime.pending_reset_event(Utc::now()).is_none());
    let (state, _) = load_runtime_state(&path);
    let event = state.active_cycle.unwrap().confirmed_event.unwrap();
    assert_eq!(event.event_id, event_id);
    assert_eq!(
        event.notification_disposition,
        NotificationDisposition::Failed
    );
    let audit = fs::read_to_string(runtime.audit.path_for(Utc::now())).unwrap();
    assert!(audit
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .any(|record| record["action"] == "notificationFailed"
            && record["errorCode"] == "claimInterrupted"));
    assert!(!audit
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .any(|record| record["action"] == "notificationQueued"));
    cleanup(&path);
}

#[test]
fn notification_suppression_audit_distinguishes_disabled_and_quiet() {
    for (index, (disposition, expected_code)) in [
        (
            NotificationDisposition::SuppressedDisabled,
            "suppressedDisabled",
        ),
        (NotificationDisposition::SuppressedQuiet, "suppressedQuiet"),
    ]
    .into_iter()
    .enumerate()
    {
        let path = temp_path(&format!("notification-suppressed-{index}"));
        let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
        let event = establish_recovered_reset(&runtime, true, at(1_000), at(1_000));
        assert!(runtime.mark_notification_suppressed(&event.event_id, disposition, at(1_001),));
        let audit = fs::read_to_string(runtime.audit.path_for(at(1_001))).unwrap();
        assert!(audit
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|record| record["action"] == "notificationSuppressed"
                && record["errorCode"] == expected_code));
        cleanup(&path);
    }
}

#[test]
fn manual_success_satisfies_only_an_already_confirmed_event() {
    let path = temp_path("manual-satisfaction");
    let runtime = QuotaAutoContinueRuntime::new(path.clone()).unwrap();
    let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
    let next_reset = at(1_000 + 7 * 24 * 60 * 60);
    runtime.observe_dashboard(true, &identity, &dashboard(at(1_000)), at(900));
    runtime.record_manual_success("gpt-5.4".to_owned(), at(950));
    assert_eq!(
        runtime.status(true, at(950)).phase,
        QuotaAutoContinuePhase::Scheduled
    );

    runtime.observe_dashboard(true, &identity, &dashboard(next_reset), at(1_000));
    assert!(runtime.pending_reset_event(at(1_000)).is_some());
    runtime.record_manual_success("gpt-5.4".to_owned(), at(1_001));
    let status = runtime.status(true, at(1_001));
    assert_eq!(status.phase, QuotaAutoContinuePhase::Succeeded);
    assert!(status.last_automatic_result.is_none());
    assert_eq!(
        status.last_manual_result.unwrap().success_at,
        Some(at(1_001))
    );
    assert!(runtime
        .claim_due_attempt(true, at(1_001))
        .unwrap()
        .is_none());
    cleanup(&path);
}

#[test]
fn concurrent_observe_preflight_and_manual_guard_complete_without_lock_inversion() {
    let path = temp_path("lock-order");
    let runtime = Arc::new(QuotaAutoContinueRuntime::new(path.clone()).unwrap());
    establish_recovered_reset(&runtime, true, at(1_000), at(950));
    let attempt = runtime.claim_due_attempt(true, at(950)).unwrap().unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let (done_sender, done_receiver) = mpsc::channel();

    let observer = {
        let runtime = Arc::clone(&runtime);
        let barrier = Arc::clone(&barrier);
        let done_sender = done_sender.clone();
        thread::spawn(move || {
            let identity = UsageAccountIdentity::AccountId("account-a".to_owned());
            let snapshot = dashboard(at(1_000 + 7 * 24 * 60 * 60));
            barrier.wait();
            for _ in 0..2_000 {
                runtime.observe_dashboard(true, &identity, &snapshot, at(951));
            }
            done_sender.send("observe").unwrap();
        })
    };
    let preflight = {
        let runtime = Arc::clone(&runtime);
        let barrier = Arc::clone(&barrier);
        let done_sender = done_sender.clone();
        let attempt = attempt.clone();
        thread::spawn(move || {
            barrier.wait();
            for _ in 0..20_000 {
                let _ = runtime.preflight_decision(&attempt);
            }
            done_sender.send("preflight").unwrap();
        })
    };
    let manual = {
        let runtime = Arc::clone(&runtime);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            for _ in 0..20_000 {
                assert!(runtime.manual_account_guard().is_some());
            }
            done_sender.send("manual").unwrap();
        })
    };

    let mut completed = Vec::new();
    for _ in 0..3 {
        completed.push(
            done_receiver
                .recv_timeout(Duration::from_secs(10))
                .expect("并发锁顺序不应卡住"),
        );
    }
    completed.sort_unstable();
    assert_eq!(completed, ["manual", "observe", "preflight"]);
    observer.join().unwrap();
    preflight.join().unwrap();
    manual.join().unwrap();
    cleanup(&path);
}
