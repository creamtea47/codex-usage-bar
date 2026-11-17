use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
fn document(account: &str) -> Value {
    let claims = serde_json::json!({"email":"saved@example.com","name":"Example User","auth_provider":"google","iat":100,"exp":900,
        "https://api.openai.com/auth":{"chatgpt_user_id":"saved-user","chatgpt_account_id":account,"chatgpt_plan_type":"plus","chatgpt_subscription_active_until":"2030-01-01T00:00:00Z","chatgpt_subscription_last_checked":"2029-12-01T00:00:00Z"}});
    serde_json::json!({"tokens":{"id_token":format!("e30.{}.signature",URL_SAFE_NO_PAD.encode(claims.to_string())),"access_token":"private-secret","account_id":account}})
}
#[test]
fn whitelist_prioritizes_live_fields_and_keeps_subscription_provenance() {
    let now = Utc::now();
    let payload = serde_json::json!({"email":"live@example.com","user_id":"live-user","plan_type":"unknown-plan","access_token":"must-not-escape","rate_limit_reset_credits":{"available_count":2},"model_usage":{"model-one":{"available":true}},"credits":{"balance":"7","unlimited":false}});
    let live = parse_usage_profile(&payload, now);
    let view = account_details(
        &document("one"),
        Some(&live),
        Path::new("managed/auth.json"),
    );
    assert_eq!(view.email.as_deref(), Some("live@example.com"));
    assert_eq!(view.user_id.as_deref(), Some("live-user"));
    assert_eq!(view.plan_type.as_deref(), Some("unknown-plan"));
    assert_eq!(view.identity_source, "usage");
    assert_eq!(
        view.subscription_ends_at.unwrap().to_rfc3339(),
        "2030-01-01T00:00:00+00:00"
    );
    assert_eq!(
        view.subscription_checked_at.unwrap().to_rfc3339(),
        "2029-12-01T00:00:00+00:00"
    );
    let json = serde_json::to_string(&view).unwrap();
    assert!(
        !json.contains("secret")
            && !json.contains("must-not-escape")
            && !json.contains("signature")
    );
    let missing = account_details(&Value::Null, None, Path::new("empty"));
    assert!(missing.email.is_none());
    assert!(missing.subscription_ends_at.is_none());
}
#[test]
fn disk_inspection_detects_external_changes_and_file_store_boundaries() {
    let root = std::env::temp_dir().join(format!("codex-profile-check-{}", rand::random::<u64>()));
    let store = AccountStore::open(root.join("accounts")).unwrap();
    let path = root.join("source.json");
    assert_eq!(
        store.inspect_codex_login_at(&path, "file").status,
        "missing"
    );
    assert_eq!(
        store.inspect_codex_login_at(&path, "keyring").status,
        "unsupportedStore"
    );
    crate::infra::accounts::atomic_write(&path, document("one").to_string().as_bytes()).unwrap();
    let one = store.import_with_binding(&path, None).unwrap();
    assert_eq!(
        store
            .inspect_codex_login_at(&path, "file")
            .matched_account_id,
        Some(one)
    );
    crate::infra::accounts::atomic_write(&path, document("two").to_string().as_bytes()).unwrap();
    assert_eq!(
        store.inspect_codex_login_at(&path, "file").status,
        "unmanaged"
    );
    crate::infra::accounts::atomic_write(&path, b"invalid").unwrap();
    assert_eq!(
        store.inspect_codex_login_at(&path, "file").status,
        "invalid"
    );
    assert!(root.starts_with(std::env::temp_dir()));
    assert!(root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("codex-profile-check-"));
    std::fs::remove_dir_all(root).unwrap();
}
