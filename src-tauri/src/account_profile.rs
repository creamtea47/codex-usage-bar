//! 账号页展示资料的白名单。此模块不发网络请求，不持久化资料，也不实现 Debug，
//! 避免错误日志意外打印完整邮箱、ID 或路径。
use crate::accounts::{document_id, read_document, token_claims, AccountStore};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

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
pub struct ExtraCredits {
    pub has_credits: Option<bool>,
    pub unlimited: Option<bool>,
    pub balance: Option<String>,
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

/// 主窗快照保持原有脱敏结构；仅 FetchedDashboard 的 Rust 内部旁路携带此资料。
pub fn parse_usage_profile(payload: &Value, observed_at: DateTime<Utc>) -> UsageProfile {
    let email = ["/email", "/account/email", "/user/email", "/profile/email"]
        .iter()
        .find_map(|p| email_text(payload.pointer(p)));
    let reset = payload.get("rate_limit_reset_credits");
    let credits = payload
        .get("credits")
        .filter(|v| v.is_object())
        .map(|v| ExtraCredits {
            has_credits: v.get("has_credits").and_then(Value::as_bool),
            unlimited: v.get("unlimited").and_then(Value::as_bool),
            balance: text(v.get("balance")),
        })
        .filter(|v| v.balance.is_some() || v.has_credits.is_some() || v.unlimited.is_some());
    let models = payload
        .get("model_usage")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|v| v.iter())
        .take(64)
        .filter_map(|(id, v)| {
            Some(ModelAvailability {
                id: text(Some(&Value::String(id.clone())))?,
                available: v.get("available")?.as_bool()?,
                available_at: timestamp(v.get("available_at")),
            })
        })
        .collect();
    UsageProfile {
        email,
        user_id: text(payload.get("user_id")),
        account_id: text(payload.get("account_id")),
        plan_type: text(payload.get("plan_type")),
        observed_at: Some(observed_at),
        reset_credits: reset.and_then(|v| {
            Some(ResetCredits {
                available: v.get("available_count")?.as_u64()?,
                applicable: v.get("applicable_available_count").and_then(Value::as_u64),
            })
        }),
        extra_credits: credits,
        models,
    }
}

/// 实时用量优先；订阅日期只取明确凭证字段，不把 JWT exp 误作订阅有效期。
pub fn account_details(doc: &Value, usage: Option<&UsageProfile>, path: &Path) -> AccountDetails {
    let id = token_claims(doc.pointer("/tokens/id_token").and_then(Value::as_str));
    let access = token_claims(doc.pointer("/tokens/access_token").and_then(Value::as_str));
    let auth = id
        .get("https://api.openai.com/auth")
        .unwrap_or(&Value::Null);
    let access_auth = access
        .get("https://api.openai.com/auth")
        .unwrap_or(&Value::Null);
    let profile = access
        .get("https://api.openai.com/profile")
        .unwrap_or(&Value::Null);
    let live = usage.cloned().unwrap_or_default();
    AccountDetails {
        identity_source: if live.email.is_some() {
            "usage"
        } else {
            "credentials"
        },
        plan_source: if live.plan_type.is_some() {
            "usage"
        } else {
            "credentials"
        },
        email: live
            .email
            .or_else(|| email_text(id.get("email")))
            .or_else(|| email_text(profile.get("email"))),
        name: text(id.get("name")).or_else(|| text(profile.get("name"))),
        login_provider: text(id.get("auth_provider")).or_else(|| text(doc.get("auth_provider"))),
        user_id: live
            .user_id
            .or_else(|| text(auth.get("chatgpt_user_id")))
            .or_else(|| text(access_auth.get("chatgpt_user_id"))),
        account_id: live
            .account_id
            .or_else(|| text(doc.pointer("/tokens/account_id")))
            .or_else(|| text(auth.get("chatgpt_account_id"))),
        plan_type: live
            .plan_type
            .or_else(|| text(auth.get("chatgpt_plan_type")))
            .or_else(|| text(access_auth.get("chatgpt_plan_type"))),
        credential_observed_at: timestamp(doc.get("last_refresh"))
            .or_else(|| timestamp(id.get("iat"))),
        usage_observed_at: live.observed_at,
        subscription_started_at: timestamp(auth.get("chatgpt_subscription_active_start")),
        subscription_ends_at: timestamp(auth.get("chatgpt_subscription_active_until")),
        subscription_checked_at: timestamp(auth.get("chatgpt_subscription_last_checked")),
        managed_auth_path: path.to_string_lossy().into_owned(),
        reset_credits: live.reset_credits,
        extra_credits: live.extra_credits,
        models: live.models,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLoginSource {
    pub path: Option<String>,
    pub storage: Option<String>,
    pub status: &'static str,
    pub matched_account_id: Option<String>,
    pub email: Option<String>,
    pub checked_at: DateTime<Utc>,
    pub binding_matches: Option<bool>,
}

impl AccountStore {
    /// 只读重新检查磁盘文件，独立于查看选择和历史绑定；不声称读取了运行中会话缓存。
    pub fn inspect_codex_login(&self) -> CodexLoginSource {
        match crate::accounts::codex_auth_location() {
            Ok((path, storage)) => self.inspect_codex_login_at(&path, &storage),
            Err(_) => CodexLoginSource {
                path: None,
                storage: None,
                status: "unavailable",
                matched_account_id: None,
                email: None,
                checked_at: Utc::now(),
                binding_matches: None,
            },
        }
    }

    fn inspect_codex_login_at(&self, path: &Path, storage: &str) -> CodexLoginSource {
        let mut result = CodexLoginSource {
            path: (storage == "file").then(|| path.to_string_lossy().into_owned()),
            storage: Some(storage.into()),
            status: "unsupportedStore",
            matched_account_id: None,
            email: None,
            checked_at: Utc::now(),
            binding_matches: None,
        };
        if storage != "file" {
            return result;
        }
        if !path.exists() {
            result.status = "missing";
            return result;
        }
        let Ok(doc) = read_document(path) else {
            result.status = "invalid";
            return result;
        };
        let Ok(id) = document_id(&doc) else {
            result.status = "invalid";
            return result;
        };
        result.email = account_details(&doc, None, path).email;
        result.matched_account_id = self
            .configs()
            .into_iter()
            .find(|c| c.id == id)
            .map(|c| c.id);
        result.binding_matches = self.bound_id().map(|bound| bound == id);
        result.status = if result.matched_account_id.is_some() {
            "matched"
        } else {
            "unmanaged"
        };
        result
    }
}

fn text(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    (!text.is_empty() && text.chars().count() <= 512 && !text.chars().any(char::is_control))
        .then(|| text.into())
}
fn email_text(value: Option<&Value>) -> Option<String> {
    text(value).filter(|v| {
        v.split_once('@')
            .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty())
            && !v.chars().any(char::is_whitespace)
    })
}
fn timestamp(value: Option<&Value>) -> Option<DateTime<Utc>> {
    let value = value?;
    if let Some(seconds) = value.as_i64() {
        DateTime::from_timestamp(seconds, 0)
    } else {
        DateTime::parse_from_rfc3339(value.as_str()?)
            .ok()
            .map(|v| v.with_timezone(&Utc))
    }
}

#[cfg(test)]
mod tests {
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
        let root =
            std::env::temp_dir().join(format!("codex-profile-check-{}", rand::random::<u64>()));
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
        crate::accounts::atomic_write(&path, document("one").to_string().as_bytes()).unwrap();
        let one = store.import_with_binding(&path, None).unwrap();
        assert_eq!(
            store
                .inspect_codex_login_at(&path, "file")
                .matched_account_id,
            Some(one)
        );
        crate::accounts::atomic_write(&path, document("two").to_string().as_bytes()).unwrap();
        assert_eq!(
            store.inspect_codex_login_at(&path, "file").status,
            "unmanaged"
        );
        crate::accounts::atomic_write(&path, b"invalid").unwrap();
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
}
