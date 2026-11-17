use crate::domain::usage::ExtraCredits;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountConfig {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    #[serde(default)]
    pub auto_continue: bool,
    #[serde(default)]
    pub model: Option<String>,
}
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub auto_continue: bool,
    pub model: Option<String>,
    pub bound_to_codex: bool,
    pub auth_status: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub can_refresh: bool,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageProfile {
    pub email: Option<String>,
    pub user_id: Option<String>,
    pub account_id: Option<String>,
    pub plan_type: Option<String>,
    pub observed_at: Option<DateTime<Utc>>,
    pub reset_credits: Option<ResetCredits>,
    pub extra_credits: Option<ExtraCredits>,
    pub models: Vec<ModelAvailability>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetCredits {
    pub available: u64,
    pub applicable: Option<u64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelAvailability {
    pub id: String,
    pub available: bool,
    pub available_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDetails {
    pub email: Option<String>,
    pub name: Option<String>,
    pub login_provider: Option<String>,
    pub user_id: Option<String>,
    pub account_id: Option<String>,
    pub plan_type: Option<String>,
    pub identity_source: &'static str,
    pub plan_source: &'static str,
    pub credential_observed_at: Option<DateTime<Utc>>,
    pub usage_observed_at: Option<DateTime<Utc>>,
    pub subscription_started_at: Option<DateTime<Utc>>,
    pub subscription_ends_at: Option<DateTime<Utc>>,
    pub subscription_checked_at: Option<DateTime<Utc>>,
    pub managed_auth_path: String,
    pub reset_credits: Option<ResetCredits>,
    pub extra_credits: Option<ExtraCredits>,
    pub models: Vec<ModelAvailability>,
}
