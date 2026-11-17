use super::*;
use crate::domain::usage::DashboardStatus;

#[tokio::test]
async fn reset_credit_request_is_read_only_account_scoped_and_handles_failures() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!(
        "http://{}/backend-api/wham/rate-limit-reset-credits",
        listener.local_addr().unwrap()
    );
    let server = std::thread::spawn(move || {
        for (status, body) in [
            (
                "200 OK",
                r#"{"credits":[{"id":"private","status":"available","reset_type":"codex_rate_limits","expires_at":"2026-10-22T20:30:58Z"}]}"#,
            ),
            ("401 Unauthorized", "private server error"),
            ("200 OK", "invalid json"),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(StdDuration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap().to_lowercase();
            assert!(request.starts_with("get /backend-api/wham/rate-limit-reset-credits "));
            assert!(request.contains("authorization: bearer test-token\r\n"));
            assert!(request.contains("chatgpt-account-id: test-account\r\n"));
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    let client = UsageClient {
        client: HttpClient::fixed(
            Client::builder()
                .no_proxy()
                .timeout(StdDuration::from_secs(5))
                .build()
                .unwrap(),
        ),
    };
    let credentials = crate::infra::auth::AuthCredentials {
        access_token: "test-token".into(),
        account_id: Some("test-account".into()),
    };
    let credits = client
        .fetch_reset_credit_details_at(credentials.clone(), &endpoint)
        .await
        .unwrap();
    assert_eq!(credits.len(), 1);
    assert!(!serde_json::to_string(&credits).unwrap().contains("private"));
    assert!(matches!(
        client
            .fetch_reset_credit_details_at(credentials.clone(), &endpoint)
            .await,
        Err(UsageError::Unauthorized)
    ));
    assert!(matches!(
        client
            .fetch_reset_credit_details_at(credentials, &endpoint)
            .await,
        Err(UsageError::InvalidPayload)
    ));
    server.join().unwrap();
}

#[test]
fn parses_primary_and_secondary_windows_without_private_fields() {
    let payload = serde_json::json!({
        "email": "private@example.com",
        "plan_type": "pro",
        "rate_limit": {
            "primary_window": {"used_percent": 30, "limit_window_seconds": 18000, "reset_after_seconds": 3600, "reset_at": 1_800_000_000},
            "secondary_window": {"used_percent": 40, "limit_window_seconds": 604800, "reset_after_seconds": 7200, "reset_at": 1_800_000_000}
        }
    });
    let snapshot = parse_usage_payload(&payload).unwrap();
    assert_eq!(snapshot.status, DashboardStatus::Ready);
    assert_eq!(
        snapshot.account_email_masked.as_deref(),
        Some("p***@example.com")
    );
    assert_eq!(snapshot.quota_windows.len(), 2);
    assert_eq!(snapshot.quota_windows[0].remaining_percent, 70);
    assert!(snapshot.quota_windows[0].show_pace_marker);
    assert!(snapshot.quota_windows[1].show_pace_marker);
    let rendered = serde_json::to_string(&snapshot).unwrap();
    assert!(!rendered.contains("private@example.com"));
    assert!(!rendered.contains("private@"));
    assert!(rendered.contains("p***@example.com"));
    assert!(!rendered.contains("access_token"));
}

#[test]
fn safely_masks_nested_account_email_and_omits_invalid_email() {
    let nested = serde_json::json!({
        "account": { "email": "  jane.doe@example.com " },
        "rate_limit": {
            "primary_window": {"used_percent": 30, "limit_window_seconds": 18000, "reset_after_seconds": 3600, "reset_at": 1_800_000_000}
        }
    });
    assert_eq!(
        parse_usage_payload(&nested)
            .unwrap()
            .account_email_masked
            .as_deref(),
        Some("j***@example.com")
    );

    let invalid = serde_json::json!({
        "email": "not-an-email",
        "rate_limit": {
            "primary_window": {"used_percent": 30, "limit_window_seconds": 18000, "reset_after_seconds": 3600, "reset_at": 1_800_000_000}
        }
    });
    assert_eq!(
        parse_usage_payload(&invalid).unwrap().account_email_masked,
        None
    );
}

#[test]
fn accepts_future_windows_array() {
    let payload = serde_json::json!({
        "rate_limit": {"windows": [
            {"id":"monthly", "label":"月限额", "used_percent":10, "limit_window_seconds":2592000, "reset_after_seconds":120, "reset_at":1800000000}
        ]}
    });
    let snapshot = parse_usage_payload(&payload).unwrap();
    assert_eq!(snapshot.quota_windows[0].label.as_deref(), Some("月限额"));
    assert_eq!(snapshot.quota_windows[0].remaining_percent, 90);
}

#[test]
fn accepts_unknown_named_quota_windows() {
    let payload = serde_json::json!({
        "rate_limit": {
            "tertiary_window": {
                "id": "monthly",
                "used_percent": 55,
                "limit_window_seconds": 2_592_000,
                "reset_after_seconds": 3_600,
                "reset_at": "2027-01-01T00:00:00Z"
            }
        }
    });

    let snapshot = parse_usage_payload(&payload).unwrap();
    assert_eq!(snapshot.quota_windows.len(), 1);
    assert_eq!(snapshot.quota_windows[0].id, "monthly");
    assert_eq!(snapshot.quota_windows[0].remaining_percent, 45);
    assert!(snapshot.quota_windows[0].reset_at.is_some());
}

#[test]
fn accepts_window_without_reset_at_when_reset_after_seconds_is_present() {
    let payload = serde_json::json!({
        "rate_limit": {
            "primary_window": {
                "used_percent": 25,
                "limit_window_seconds": 18_000,
                "reset_after_seconds": 900
            }
        }
    });

    let snapshot = parse_usage_payload(&payload).unwrap();
    assert_eq!(snapshot.quota_windows.len(), 1);
    assert_eq!(snapshot.quota_windows[0].remaining_percent, 75);
    assert_eq!(snapshot.quota_windows[0].reset_after_seconds, 900);
    assert_eq!(snapshot.quota_windows[0].reset_at, None);
    assert_eq!(snapshot.quota_windows[0].start_at, None);
}

#[test]
fn rejects_non_positive_or_overflowing_window_durations_without_panicking() {
    for invalid_duration in [0, -1, i64::MAX] {
        let payload = serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 25,
                    "limit_window_seconds": invalid_duration,
                    "reset_after_seconds": 900,
                    "reset_at": 1_800_000_000
                }
            }
        });
        assert!(matches!(
            parse_usage_payload(&payload),
            Err(crate::domain::usage::InvalidUsagePayload)
        ));
    }
}

#[test]
fn maps_expired_or_forbidden_authentication_to_safe_messages() {
    for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
        let error = status_to_error(status);
        assert!(matches!(error, UsageError::Unauthorized));
        let rendered = error.to_string();
        assert!(rendered.contains("重新登录"));
        assert!(!rendered.to_ascii_lowercase().contains("authorization"));
        assert!(!rendered.to_ascii_lowercase().contains("token"));
    }
}

#[test]
fn rejects_payloads_without_valid_quota_windows() {
    let payload = serde_json::json!({"rate_limit": {"primary_window": null}});
    assert!(matches!(
        parse_usage_payload(&payload),
        Err(crate::domain::usage::InvalidUsagePayload)
    ));
}

#[test]
fn parses_reset_credit_counts_without_exposing_them_in_dashboard_json() {
    let payload = serde_json::json!({
        "rate_limit": {
            "primary_window": {"used_percent": 30, "limit_window_seconds": 18000, "reset_after_seconds": 3600}
        },
        "rate_limit_reset_credits": {
            "available_count": 2,
            "applicable_available_count": 1
        }
    });

    assert_eq!(
        parse_reset_credit_availability(&payload),
        Some(ResetCreditAvailability {
            available_count: 2,
            applicable_available_count: Some(1),
        })
    );
    let rendered = serde_json::to_string(&parse_usage_payload(&payload).unwrap()).unwrap();
    assert!(!rendered.contains("availableCount"));
    assert!(!rendered.contains("resetCredit"));
}

#[test]
fn ignores_missing_null_negative_or_mistyped_reset_credit_counts() {
    for invalid in [
        serde_json::json!(null),
        serde_json::json!({}),
        serde_json::json!({"available_count": null}),
        serde_json::json!({"available_count": -1, "applicable_available_count": 0}),
        serde_json::json!({"available_count": "1", "applicable_available_count": 0}),
        serde_json::json!({"available_count": 1, "applicable_available_count": -1}),
        serde_json::json!({"available_count": 1, "applicable_available_count": "0"}),
    ] {
        let payload = serde_json::json!({"rate_limit_reset_credits": invalid});
        assert_eq!(parse_reset_credit_availability(&payload), None);
    }
}

#[test]
fn accepts_missing_applicable_count_and_u64_max_without_overflow() {
    let without_applicable = serde_json::json!({
        "rate_limit_reset_credits": {"available_count": 1}
    });
    assert_eq!(
        parse_reset_credit_availability(&without_applicable),
        Some(ResetCreditAvailability {
            available_count: 1,
            applicable_available_count: None,
        })
    );

    let maximum = serde_json::json!({
        "rate_limit_reset_credits": {
            "available_count": u64::MAX,
            "applicable_available_count": u64::MAX
        }
    });
    assert_eq!(
        parse_reset_credit_availability(&maximum),
        Some(ResetCreditAvailability {
            available_count: u64::MAX,
            applicable_available_count: Some(u64::MAX),
        })
    );
}
