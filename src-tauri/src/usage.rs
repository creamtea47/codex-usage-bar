use crate::{
    auth::{read_auth_credentials, AuthError},
    models::{DashboardSnapshot, DashboardStatus, QuotaWindow},
};
use chrono::{DateTime, Duration, TimeZone, Utc};
use reqwest::{Client, StatusCode};
use serde_json::{Map, Value};
use std::{collections::HashSet, time::Duration as StdDuration};

const USAGE_ENDPOINT: &str = "https://chatgpt.com/backend-api/wham/usage";

#[derive(Debug, thiserror::Error)]
pub enum UsageError {
    #[error("{0}")]
    Auth(#[from] AuthError),
    #[error("无法建立安全的用量请求。")]
    Client,
    #[error("用量请求超时或网络不可用。")]
    Network,
    #[error("Codex 登录已失效，请重新登录后再刷新。")]
    Unauthorized,
    #[error("用量服务暂时拒绝请求，请稍后重试。")]
    Rejected,
    #[error("用量服务暂时不可用，请稍后重试。")]
    Server,
    #[error("用量服务返回了未识别的数据，请更新应用后重试。")]
    InvalidPayload,
}

#[derive(Clone)]
pub struct UsageClient {
    client: Client,
}

impl UsageClient {
    pub fn new() -> Result<Self, UsageError> {
        // reqwest 的 system-proxy feature 会安全读取系统代理和 HTTP(S)_PROXY，
        // 仅用于发起本次只读请求；代理配置绝不记录到日志或传给 React。
        let client = Client::builder()
            .user_agent("CodexUsageBar/0.2 (read-only)")
            .connect_timeout(StdDuration::from_secs(10))
            .timeout(StdDuration::from_secs(20))
            .build()
            .map_err(|_| UsageError::Client)?;
        Ok(Self { client })
    }

    /// 仅使用 access_token 查询额度。这里没有 OAuth、重置卡或任何写入路径。
    pub async fn fetch_dashboard(&self) -> Result<DashboardSnapshot, UsageError> {
        let credentials = read_auth_credentials()?;
        let mut request = self
            .client
            .get(USAGE_ENDPOINT)
            .bearer_auth(&credentials.access_token)
            .header("Accept", "application/json")
            .header("Cache-Control", "no-cache")
            .header("Pragma", "no-cache");
        if let Some(account_id) = credentials.account_id.as_deref() {
            request = request.header("ChatGPT-Account-Id", account_id);
        }

        let response = request.send().await.map_err(|error| {
            // 只保留可排查的传输类别，不写入 URL、请求头、代理地址或任何认证数据。
            let category = if error.is_timeout() {
                "timeout"
            } else if error.is_connect() {
                "connect"
            } else if error.is_request() {
                "request"
            } else {
                "other"
            };
            log::warn!("用量请求传输失败：类别={category}");
            UsageError::Network
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_to_error(status));
        }
        let payload: Value = response
            .json()
            .await
            .map_err(|_| UsageError::InvalidPayload)?;
        parse_usage_payload(&payload)
    }
}

fn status_to_error(status: StatusCode) -> UsageError {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => UsageError::Unauthorized,
        StatusCode::TOO_MANY_REQUESTS => UsageError::Rejected,
        value if value.is_server_error() => UsageError::Server,
        _ => UsageError::Rejected,
    }
}

/// 支持现有 primary/secondary 字段，也兼容未来可能出现的 windows 数组或额外 *_window 字段。
pub fn parse_usage_payload(payload: &Value) -> Result<DashboardSnapshot, UsageError> {
    let rate_limit = payload
        .get("rate_limit")
        .and_then(Value::as_object)
        .ok_or(UsageError::InvalidPayload)?;
    let mut seen = HashSet::new();
    let mut windows = Vec::new();

    for key in ["primary_window", "secondary_window"] {
        if let Some(value) = rate_limit.get(key) {
            if let Some(window) = parse_window(key, value) {
                seen.insert(window.id.clone());
                windows.push(window);
            }
        }
    }
    if let Some(values) = rate_limit.get("windows").and_then(Value::as_array) {
        for (index, value) in values.iter().enumerate() {
            let id = value.get("id").and_then(Value::as_str).unwrap_or("window");
            let key = format!("{id}-{index}");
            if let Some(window) = parse_window(&key, value) {
                if seen.insert(window.id.clone()) {
                    windows.push(window);
                }
            }
        }
    }
    for (key, value) in rate_limit {
        if key.ends_with("_window") && !seen.contains(key) {
            if let Some(window) = parse_window(key, value) {
                if seen.insert(window.id.clone()) {
                    windows.push(window);
                }
            }
        }
    }

    if windows.is_empty() {
        return Err(UsageError::InvalidPayload);
    }
    windows.sort_by_key(|window| window.window_seconds);
    Ok(DashboardSnapshot {
        status: DashboardStatus::Ready,
        plan_label: payload
            .get("plan_type")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        refreshed_at: Some(Utc::now()),
        next_refresh_at: None,
        message: None,
        quota_windows: windows,
    })
}

fn parse_window(default_id: &str, value: &Value) -> Option<QuotaWindow> {
    let object = value.as_object()?;
    let used_percent = number(object, "used_percent")?.clamp(0, 100) as u8;
    let window_seconds = number(object, "limit_window_seconds")?.max(0);
    let reset_after_seconds = number(object, "reset_after_seconds")?.max(0);
    let reset_at = parse_timestamp(object.get("reset_at")?)?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or(default_id)
        .to_owned();
    let label = object
        .get("label")
        .or_else(|| object.get("name"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| label_for_window(default_id, window_seconds));
    let start_at = reset_at.checked_sub_signed(Duration::seconds(window_seconds));
    Some(QuotaWindow {
        id,
        label,
        remaining_percent: 100 - used_percent,
        used_percent,
        window_seconds,
        reset_at: Some(reset_at),
        reset_after_seconds,
        start_at,
        show_pace_marker: window_seconds >= 24 * 60 * 60,
    })
}

fn number(object: &Map<String, Value>, key: &str) -> Option<i64> {
    object
        .get(key)?
        .as_i64()
        .or_else(|| object.get(key)?.as_f64().map(|value| value.round() as i64))
}

fn parse_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    if let Some(timestamp) = value
        .as_i64()
        .or_else(|| value.as_f64().map(|value| value as i64))
    {
        return Utc.timestamp_opt(timestamp, 0).single();
    }
    value
        .as_str()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn label_for_window(id: &str, seconds: i64) -> String {
    if seconds <= 12 * 60 * 60 {
        return "短周期限额".to_owned();
    }
    if seconds >= 6 * 24 * 60 * 60 && seconds <= 8 * 24 * 60 * 60 {
        return "周限额".to_owned();
    }
    if seconds >= 24 * 60 * 60 {
        return format!("{} 天限额", (seconds as f64 / 86_400.0).round() as i64);
    }
    if id.contains("secondary") {
        "长期限额".to_owned()
    } else {
        "额度限额".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(snapshot.quota_windows.len(), 2);
        assert_eq!(snapshot.quota_windows[0].remaining_percent, 70);
        assert!(snapshot.quota_windows[1].show_pace_marker);
        let rendered = serde_json::to_string(&snapshot).unwrap();
        assert!(!rendered.contains("private@example.com"));
        assert!(!rendered.contains("access_token"));
    }

    #[test]
    fn accepts_future_windows_array() {
        let payload = serde_json::json!({
            "rate_limit": {"windows": [
                {"id":"monthly", "label":"月限额", "used_percent":10, "limit_window_seconds":2592000, "reset_after_seconds":120, "reset_at":1800000000}
            ]}
        });
        let snapshot = parse_usage_payload(&payload).unwrap();
        assert_eq!(snapshot.quota_windows[0].label, "月限额");
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
            Err(UsageError::InvalidPayload)
        ));
    }
}
