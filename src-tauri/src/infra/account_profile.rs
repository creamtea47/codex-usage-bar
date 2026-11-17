//! 账号页展示资料的白名单。此模块不发网络请求，不持久化资料，也不实现 Debug，
//! 避免错误日志意外打印完整邮箱、ID 或路径。
use crate::infra::accounts::document_id;
use crate::infra::accounts::read_document;
use crate::infra::accounts::token_claims;
use crate::infra::accounts::AccountStore;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

pub(crate) use crate::domain::accounts::{
    AccountDetails, ModelAvailability, ResetCredits, UsageProfile,
};
use crate::domain::usage::extra_credits;
/// 主窗快照保持原有脱敏结构；仅 FetchedDashboard 的 Rust 内部旁路携带此资料。
pub fn parse_usage_profile(payload: &Value, observed_at: DateTime<Utc>) -> UsageProfile {
    let email = ["/email", "/account/email", "/user/email", "/profile/email"]
        .iter()
        .find_map(|p| email_text(payload.pointer(p)));
    let reset = payload.get("rate_limit_reset_credits");
    let credits = extra_credits(payload);
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
        match crate::infra::accounts::codex_auth_location() {
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
mod tests;
