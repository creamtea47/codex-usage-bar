//! 重置卡只读明细和短期缓存；不保留卡 ID，也没有兑换路径。
use crate::models::DashboardErrorCode;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

pub const CACHE_SECONDS: i64 = 300;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResetCreditItem {
    pub expires_at: Option<DateTime<Utc>>,
    pub status: &'static str,
    pub supported_by_plan: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetCreditDetails {
    pub account_id: String,
    pub status: &'static str,
    pub fetched_at: Option<DateTime<Utc>>,
    pub checked_at: DateTime<Utc>,
    pub credits: Vec<ResetCreditItem>,
    pub error_code: Option<DashboardErrorCode>,
}

#[derive(Default)]
pub struct ResetCreditCache {
    count: Option<u64>,
    generation: u64,
    cached_generation: u64,
    value: Option<ResetCreditDetails>,
}

impl ResetCreditCache {
    /// 数量从 1→0→1 也必须换代，不能把新获得的卡与旧缓存混用。
    pub fn observe_count(&mut self, count: Option<u64>) {
        if self.count != count {
            self.count = count;
            self.generation = self.generation.wrapping_add(1);
        }
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn get(&self, now: DateTime<Utc>) -> Option<ResetCreditDetails> {
        self.value
            .as_ref()
            .filter(|v| {
                self.cached_generation == self.generation
                    && (0..CACHE_SECONDS).contains(&(now - v.checked_at).num_seconds())
            })
            .cloned()
    }
    /// 失败保留上次明细；在途查询遇到摘要换代时只返回过期标记，不覆盖新一代缓存。
    pub fn finish(
        &mut self,
        generation: u64,
        account_id: &str,
        result: Result<Vec<ResetCreditItem>, DashboardErrorCode>,
        now: DateTime<Utc>,
    ) -> ResetCreditDetails {
        let mut value = match result {
            Ok(credits) => ResetCreditDetails {
                account_id: account_id.into(),
                status: "ready",
                fetched_at: Some(now),
                checked_at: now,
                credits,
                error_code: None,
            },
            Err(code) => ResetCreditDetails {
                account_id: account_id.into(),
                status: if self.value.as_ref().is_some_and(|v| v.fetched_at.is_some()) {
                    "stale"
                } else {
                    "unavailable"
                },
                fetched_at: self.value.as_ref().and_then(|v| v.fetched_at),
                checked_at: now,
                credits: self
                    .value
                    .as_ref()
                    .map(|v| v.credits.clone())
                    .unwrap_or_default(),
                error_code: Some(code),
            },
        };
        if generation == self.generation {
            self.cached_generation = generation;
            self.value = Some(value.clone());
        } else {
            value.status = "stale";
        }
        value
    }
}

/// 保留缺少日期的 available 卡，明确区分“未返回有效期”与“永久有效”。
pub fn parse_credit_details(payload: &Value) -> Result<Vec<ResetCreditItem>, DashboardErrorCode> {
    let rows = payload
        .get("credits")
        .and_then(Value::as_array)
        .ok_or(DashboardErrorCode::InvalidResponse)?;
    if rows.len() > 1000 {
        return Err(DashboardErrorCode::InvalidResponse);
    }
    let mut credits = rows
        .iter()
        .filter(|v| {
            v.get("reset_type").and_then(Value::as_str) == Some("codex_rate_limits")
                && v.get("status").and_then(Value::as_str) == Some("available")
        })
        .map(|v| ResetCreditItem {
            expires_at: v
                .get("expires_at")
                .and_then(Value::as_str)
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|v| v.with_timezone(&Utc)),
            status: "available",
            supported_by_plan: v.get("is_supported_by_plan").and_then(Value::as_bool),
        })
        .collect::<Vec<_>>();
    credits.sort_by_key(|v| (v.expires_at.is_none(), v.expires_at));
    Ok(credits)
}

#[cfg(test)]
mod tests {
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
}
