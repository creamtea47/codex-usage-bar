use crate::{
    auth::{read_auth_credentials, AuthError},
    models::{
        DashboardErrorCode, DashboardSnapshot, DashboardStatus, QuotaFallbackLabel, QuotaWindow,
    },
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

impl UsageError {
    pub fn code(&self) -> DashboardErrorCode {
        match self {
            Self::Auth(AuthError::MissingFile) => DashboardErrorCode::AuthMissing,
            Self::Auth(_) | Self::Unauthorized => DashboardErrorCode::AuthInvalid,
            Self::Client => DashboardErrorCode::LocalBridge,
            Self::Network => DashboardErrorCode::Network,
            Self::Rejected => DashboardErrorCode::RateLimited,
            Self::Server => DashboardErrorCode::ServiceUnavailable,
            Self::InvalidPayload => DashboardErrorCode::InvalidResponse,
        }
    }
}

/// 账号材料只在 Rust 刷新调用栈内短暂存在，供本地加盐哈希；不会经 IPC 或日志输出。
pub struct FetchedDashboard {
    pub snapshot: DashboardSnapshot,
    pub account_identity: UsageAccountIdentity,
}

pub enum UsageAccountIdentity {
    AccountId(String),
    Token(String),
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
            .user_agent("CodexUsageBar/0.3 (usage-read-only)")
            .connect_timeout(StdDuration::from_secs(10))
            .timeout(StdDuration::from_secs(20))
            .build()
            .map_err(|_| UsageError::Client)?;
        Ok(Self { client })
    }

    /// 仅使用 access_token 查询额度。这里没有 OAuth、重置卡或任何写入路径。
    pub async fn fetch_dashboard(&self) -> Result<FetchedDashboard, UsageError> {
        let credentials = read_auth_credentials()?;
        let account_identity = credentials
            .account_id
            .clone()
            .map(UsageAccountIdentity::AccountId)
            .unwrap_or_else(|| UsageAccountIdentity::Token(credentials.access_token.clone()));
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
        Ok(FetchedDashboard {
            snapshot: parse_usage_payload(&payload)?,
            account_identity,
        })
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
        // 原始邮箱只存在于本次 JSON 解析栈内。前端、快照和日志都只能看到掩码结果。
        account_email_masked: parse_account_email_masked(payload),
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

/// 用量接口不同账户形态可能把账号信息放在不同对象中。只尝试已知公开展示字段，
/// 并在离开此函数前掩码，避免意外把原始邮箱扩大到应用状态或 IPC。
fn parse_account_email_masked(payload: &Value) -> Option<String> {
    ["/email", "/account/email", "/user/email", "/profile/email"]
        .into_iter()
        .filter_map(|pointer| payload.pointer(pointer).and_then(Value::as_str))
        .find_map(mask_email)
}

/// 仅保留本地部分首字符和完整域名，例如 `jane@example.com` 显示为
/// `j***@example.com`。缺少合法分隔符时不展示，宁可缺省也不回传原文。
fn mask_email(value: &str) -> Option<String> {
    let (local, domain) = value.trim().rsplit_once('@')?;
    let first = local.trim().chars().next()?;
    let domain = domain.trim();
    if domain.is_empty() || domain.chars().any(char::is_whitespace) {
        return None;
    }
    Some(format!("{first}***@{domain}"))
}

fn parse_window(default_id: &str, value: &Value) -> Option<QuotaWindow> {
    let object = value.as_object()?;
    let used_percent = number(object, "used_percent")?.clamp(0, 100) as u8;
    let window_seconds = number(object, "limit_window_seconds")?;
    if window_seconds <= 0 {
        return None;
    }
    let window_duration = Duration::try_seconds(window_seconds)?;
    let reset_after_seconds = number(object, "reset_after_seconds")?.max(0);
    let reset_at = object.get("reset_at").and_then(parse_timestamp);
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
        .map(ToOwned::to_owned);
    let start_at = reset_at.and_then(|value| value.checked_sub_signed(window_duration));
    Some(QuotaWindow {
        id,
        label,
        fallback_label: fallback_label_for_window(window_seconds),
        remaining_percent: 100 - used_percent,
        used_percent,
        window_seconds,
        reset_at,
        reset_after_seconds,
        start_at,
        show_pace_marker: window_seconds >= 24 * 60 * 60,
        forecast: None,
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

fn fallback_label_for_window(seconds: i64) -> QuotaFallbackLabel {
    if seconds <= 12 * 60 * 60 {
        return QuotaFallbackLabel::FiveHour;
    }
    if (6 * 24 * 60 * 60..=8 * 24 * 60 * 60).contains(&seconds) {
        return QuotaFallbackLabel::Weekly;
    }
    QuotaFallbackLabel::Window
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
        assert_eq!(
            snapshot.account_email_masked.as_deref(),
            Some("p***@example.com")
        );
        assert_eq!(snapshot.quota_windows.len(), 2);
        assert_eq!(snapshot.quota_windows[0].remaining_percent, 70);
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
                Err(UsageError::InvalidPayload)
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
            Err(UsageError::InvalidPayload)
        ));
    }
}
