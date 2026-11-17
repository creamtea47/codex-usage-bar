use super::*;
use chrono::Duration;
#[test]
fn parses_dates_without_ids_and_keeps_unknown_expirations() {
    let value = serde_json::json!({"credits":[
        {"id":"private-card","reset_type":"codex_rate_limits","status":"available","expires_at":"2026-10-22T20:30:58.60046Z","is_supported_by_plan":true},
        {"reset_type":"codex_rate_limits","status":"redeemed","expires_at":"2026-09-01T00:00:00Z"},
        {"reset_type":"other","status":"available"},
        {"reset_type":"codex_rate_limits","status":"available","expires_at":"invalid"},
        {"reset_type":"codex_rate_limits","status":"available","expires_at":"2026-10-01T00:00:00Z","is_supported_by_plan":false}
    ]});
    let rows = parse_credit_details(&value).unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].supported_by_plan, Some(false));
    assert!(rows[2].expires_at.is_none());
    assert_eq!(
        rows[1]
            .expires_at
            .unwrap()
            .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
        "2026-10-23 04:30:58"
    );
    assert!(!serde_json::to_string(&rows)
        .unwrap()
        .contains("private-card"));
    assert!(parse_credit_details(&serde_json::json!({})).is_err());
}
#[test]
fn cache_expires_retains_failures_and_rejects_an_old_generation() {
    let now = Utc::now();
    let mut cache = ResetCreditCache::default();
    cache.observe_count(Some(1));
    let old = cache.generation();
    cache.finish(
        old,
        "one",
        Ok(vec![ResetCreditItem {
            expires_at: Some(now + Duration::days(2)),
            status: "available",
            supported_by_plan: Some(true),
        }]),
        now,
    );
    assert!(cache.get(now + Duration::seconds(299)).is_some());
    assert!(cache.get(now + Duration::seconds(300)).is_none());
    let failed = cache.finish(
        old,
        "one",
        Err(DashboardErrorCode::Network),
        now + Duration::seconds(301),
    );
    assert_eq!(failed.status, "stale");
    assert_eq!(failed.credits.len(), 1);
    assert_eq!(failed.fetched_at, Some(now));
    cache.observe_count(Some(0));
    cache.observe_count(Some(1));
    assert!(cache.get(now + Duration::seconds(302)).is_none());
    let stale = cache.finish(old, "one", Ok(vec![]), now + Duration::seconds(303));
    assert_eq!(stale.status, "stale");
    assert!(cache.get(now + Duration::seconds(304)).is_none());
    assert!(ResetCreditCache::default().get(now).is_none());
}
