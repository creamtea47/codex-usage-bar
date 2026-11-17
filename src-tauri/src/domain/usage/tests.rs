use crate::domain::usage::DashboardErrorCode;

#[test]
fn stable_dashboard_errors_serialize_without_rust_copy() {
    assert_eq!(
        serde_json::to_string(&DashboardErrorCode::AuthMissing).unwrap(),
        r#""authMissing""#
    );
    assert_eq!(
        serde_json::to_string(&DashboardErrorCode::InvalidResponse).unwrap(),
        r#""invalidResponse""#
    );
}
