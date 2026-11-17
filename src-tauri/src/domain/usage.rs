use chrono::{DateTime, Utc};
use chrono::{Duration, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum QuotaFallbackLabel {
    FiveHour,
    Weekly,
    Window,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ForecastStatus {
    Collecting,
    Stable,
    ExhaustsBeforeReset,
    LastsUntilReset,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaForecast {
    pub status: ForecastStatus,
    pub exhausts_at: Option<DateTime<Utc>>,
    pub sample_count: usize,
    pub observed_span_seconds: i64,
    pub consumed_percent: f64,
}

/// 仅传给界面的额度窗口。这个类型刻意不含账号、Token、请求头或原始响应。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    pub id: String,
    /// 服务端明确提供的名称原样展示；缺省名称由前端根据 fallback_label 翻译。
    pub label: Option<String>,
    pub fallback_label: QuotaFallbackLabel,
    pub remaining_percent: u8,
    pub used_percent: u8,
    pub window_seconds: i64,
    pub reset_at: Option<DateTime<Utc>>,
    pub reset_after_seconds: i64,
    pub start_at: Option<DateTime<Utc>>,
    pub show_pace_marker: bool,
    pub forecast: Option<QuotaForecast>,
}

/// 前端只能根据该状态展示安全提示，不会收到网络或认证的底层细节。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DashboardStatus {
    Loading,
    Ready,
    Stale,
    Error,
}

/// IPC 只传稳定类别，避免 Rust 文案穿透双语界面或泄露底层错误细节。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DashboardErrorCode {
    AuthMissing,
    AuthInvalid,
    Network,
    RateLimited,
    ServiceUnavailable,
    InvalidResponse,
    LocalBridge,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSnapshot {
    #[serde(default)]
    pub account_id: Option<String>,
    pub status: DashboardStatus,
    /// 仅来自成功的用量响应，并已在 Rust 内完成掩码；绝不保存或传递原始邮箱。
    pub account_email_masked: Option<String>,
    pub plan_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_credits: Option<ExtraCredits>,
    pub refreshed_at: Option<DateTime<Utc>>,
    pub next_refresh_at: Option<DateTime<Utc>>,
    pub message: Option<DashboardErrorCode>,
    pub quota_windows: Vec<QuotaWindow>,
}

impl Default for DashboardSnapshot {
    fn default() -> Self {
        Self {
            account_id: None,
            status: DashboardStatus::Loading,
            account_email_masked: None,
            plan_label: None,
            extra_credits: None,
            refreshed_at: None,
            next_refresh_at: None,
            message: None,
            quota_windows: Vec::new(),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraCredits {
    pub has_credits: Option<bool>,
    pub unlimited: Option<bool>,
    pub balance: Option<String>,
}

pub fn parse_balance(value: &str) -> Option<rust_decimal::Decimal> {
    let value = value.trim();
    if value.len() > 64 {
        return None;
    }
    rust_decimal::Decimal::from_str_exact(value)
        .ok()
        .filter(|n| !n.is_sign_negative())
}

pub fn extra_credits(payload: &Value) -> Option<ExtraCredits> {
    payload
        .get("credits")
        .filter(|v| v.is_object())
        .map(|v| ExtraCredits {
            has_credits: v.get("has_credits").and_then(Value::as_bool),
            unlimited: v.get("unlimited").and_then(Value::as_bool),
            balance: v
                .get("balance")
                .and_then(Value::as_str)
                .and_then(parse_balance)
                .map(|v| v.normalize().to_string()),
        })
        .filter(|v| v.balance.is_some() || v.has_credits.is_some() || v.unlimited == Some(true))
}

pub enum UsageAccountIdentity {
    AccountId(String),
    Token(String),
}

/// 内部用量响应中的重置卡摘要。该类型只在 Rust 刷新与通知链路中流转。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResetCreditAvailability {
    pub available_count: u64,
    pub applicable_available_count: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct InvalidUsagePayload;
/// 支持现有 primary/secondary 字段，也兼容未来可能出现的 windows 数组或额外 *_window 字段。
pub fn parse_usage_payload(payload: &Value) -> Result<DashboardSnapshot, InvalidUsagePayload> {
    let extra_credits = extra_credits(payload);
    let empty = Map::new();
    let rate_limit = payload
        .get("rate_limit")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
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

    if windows.is_empty() && extra_credits.is_none() {
        return Err(InvalidUsagePayload);
    }
    windows.sort_by_key(|window| window.window_seconds);
    Ok(DashboardSnapshot {
        account_id: None,
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
        extra_credits,
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

/// `available_count` 是检测到账的唯一触发源；可用数量只用于补充通知文案。
/// 缺失的可用数量可以安全省略，但显式的非法值会让本次摘要整体失效。
pub fn parse_reset_credit_availability(payload: &Value) -> Option<ResetCreditAvailability> {
    let object = payload.get("rate_limit_reset_credits")?.as_object()?;
    let available_count = object.get("available_count")?.as_u64()?;
    let applicable_available_count = match object.get("applicable_available_count") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_u64()?),
    };
    Some(ResetCreditAvailability {
        available_count,
        applicable_available_count,
    })
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
        show_pace_marker: supports_pace(window_seconds),
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

pub fn supports_pace(seconds: i64) -> bool {
    seconds == 5 * 3600 || seconds >= 24 * 3600
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
mod tests;
