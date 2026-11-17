use super::*;
use std::{env, time::SystemTime};

fn temp_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    env::temp_dir().join(format!("codex-reset-credit-{label}-{nonce}.json"))
}

fn identity(value: &str) -> UsageAccountIdentity {
    UsageAccountIdentity::AccountId(value.to_owned())
}

fn availability(available_count: u64, applicable: Option<u64>) -> ResetCreditAvailability {
    ResetCreditAvailability {
        available_count,
        applicable_available_count: applicable,
    }
}

fn cleanup(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(path.with_extension("json.tmp"));
}

#[test]
fn credits_survive_restart_missing_data_and_preserve_decimal_precision() {
    use crate::domain::usage::ExtraCredits;
    let path = temp_path("extra");
    let id = identity("account-a");
    let credit = |amount: Option<&str>, has: bool| ExtraCredits {
        balance: amount.map(str::to_owned),
        has_credits: Some(has),
        unlimited: Some(false),
    };
    let first = ResetCreditNotificationRuntime::new(path.clone());
    assert_eq!(
        first.observe_extra(&id, Some(&credit(Some("2.195"), true))),
        None
    );
    let next = ResetCreditNotificationRuntime::new(path.clone());
    assert_eq!(next.observe_extra(&id, None), None);
    assert_eq!(next.observe_extra(&id, Some(&credit(None, true))), None);
    let after = credit(Some("2.205"), true);
    assert_eq!(
        next.observe_extra(&id, Some(&after)),
        Some(ExtraIncrease::Balance("0.01".into(), "2.205".into()))
    );
    assert_eq!(next.observe_extra(&id, Some(&after)), None);
    assert_eq!(next.observe_extra(&id, Some(&credit(None, false))), None);
    assert_eq!(
        next.observe_extra(&id, Some(&credit(None, true))),
        Some(ExtraIncrease::Available)
    );
    assert_eq!(
        next.observe_extra(&identity("account-b"), Some(&after)),
        None
    );
    cleanup(&path);
}

#[test]
fn first_observation_and_account_switch_only_establish_baselines() {
    let path = temp_path("baseline");
    let runtime = ResetCreditNotificationRuntime::new(path.clone());
    assert_eq!(
        runtime.observe(&identity("account-a"), Some(availability(1, Some(0)))),
        None
    );
    assert_eq!(
        runtime.observe(&identity("account-b"), Some(availability(4, Some(2)))),
        None
    );
    cleanup(&path);
}

#[test]
fn reports_only_available_count_increases() {
    let path = temp_path("increase");
    let runtime = ResetCreditNotificationRuntime::new(path.clone());
    assert_eq!(
        runtime.observe(&identity("account"), Some(availability(0, Some(0)))),
        None
    );
    assert_eq!(
        runtime.observe(&identity("account"), Some(availability(1, Some(0)))),
        Some(ResetCreditIncrease {
            gained_count: 1,
            available_count: 1,
            applicable_available_count: Some(0),
        })
    );
    assert_eq!(
        runtime.observe(&identity("account"), Some(availability(3, Some(1)))),
        Some(ResetCreditIncrease {
            gained_count: 2,
            available_count: 3,
            applicable_available_count: Some(1),
        })
    );
    assert_eq!(
        runtime.observe(&identity("account"), Some(availability(3, Some(2)))),
        None
    );
    cleanup(&path);
}

#[test]
fn decreases_advance_the_baseline_without_alerting() {
    let path = temp_path("decrease");
    let runtime = ResetCreditNotificationRuntime::new(path.clone());
    assert_eq!(
        runtime.observe(&identity("account"), Some(availability(2, None))),
        None
    );
    assert_eq!(
        runtime.observe(&identity("account"), Some(availability(0, None))),
        None
    );
    assert_eq!(
        runtime.observe(&identity("account"), Some(availability(1, None))),
        Some(ResetCreditIncrease {
            gained_count: 1,
            available_count: 1,
            applicable_available_count: None,
        })
    );
    cleanup(&path);
}

#[test]
fn missing_observations_do_not_replace_the_previous_baseline() {
    let path = temp_path("missing");
    let runtime = ResetCreditNotificationRuntime::new(path.clone());
    assert_eq!(
        runtime.observe(&identity("account"), Some(availability(0, None))),
        None
    );
    assert_eq!(runtime.observe(&identity("account"), None), None);
    assert_eq!(
        runtime.observe(&identity("account"), Some(availability(1, None))),
        Some(ResetCreditIncrease {
            gained_count: 1,
            available_count: 1,
            applicable_available_count: None,
        })
    );
    cleanup(&path);
}

#[test]
fn persisted_baseline_detects_an_increase_after_restart() {
    let path = temp_path("restart");
    {
        let runtime = ResetCreditNotificationRuntime::new(path.clone());
        assert_eq!(
            runtime.observe(&identity("account"), Some(availability(0, None))),
            None
        );
    }
    let restarted = ResetCreditNotificationRuntime::new(path.clone());
    assert_eq!(
        restarted.observe(&identity("account"), Some(availability(1, Some(1)))),
        Some(ResetCreditIncrease {
            gained_count: 1,
            available_count: 1,
            applicable_available_count: Some(1),
        })
    );
    cleanup(&path);
}

#[test]
fn corrupt_state_rebuilds_without_reporting_existing_cards() {
    let path = temp_path("corrupt");
    fs::write(&path, b"not-json").unwrap();
    let runtime = ResetCreditNotificationRuntime::new(path.clone());
    assert_eq!(
        runtime.observe(&identity("account"), Some(availability(5, Some(0)))),
        None
    );
    cleanup(&path);
}

#[test]
fn persisted_state_contains_only_redacted_baseline_fields() {
    let path = temp_path("redacted");
    let runtime = ResetCreditNotificationRuntime::new(path.clone());
    assert_eq!(
        runtime.observe(
            &identity("sensitive-account-id"),
            Some(availability(2, Some(1)))
        ),
        None
    );

    let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut keys = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "accountFingerprint",
            "availableCount",
            "salt",
            "schemaVersion"
        ]
    );
    let rendered = value.to_string();
    assert!(!rendered.contains("sensitive-account-id"));
    assert!(!rendered.contains("applicableAvailableCount"));
    assert!(!path.with_extension("json.tmp").exists());
    cleanup(&path);
}
