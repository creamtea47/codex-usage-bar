use crate::domain::usage::DashboardErrorCode;
use crate::domain::usage::DashboardSnapshot;
use crate::infra::auth::read_auth_credentials;
use crate::infra::auth::AuthError;
use crate::infra::network::HttpClient;
use chrono::Utc;
#[cfg(test)]
use reqwest::Client;
use reqwest::StatusCode;
use serde_json::Value;
#[cfg(test)]
use std::time::Duration as StdDuration;

const USAGE_ENDPOINT: &str = "https://chatgpt.com/backend-api/wham/usage";
const RESET_CREDITS_ENDPOINT: &str =
    "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";

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
            Self::Auth(AuthError::ServiceUnavailable) => DashboardErrorCode::ServiceUnavailable,
            Self::Auth(AuthError::RateLimited) => DashboardErrorCode::RateLimited,
            Self::Auth(AuthError::Network) => DashboardErrorCode::Network,
            Self::Auth(AuthError::Persistence) => DashboardErrorCode::LocalBridge,
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

/// 认证身份留在 Rust；profile 是设置窗口专用的白名单资料，不进入主卡快照或日志。
pub struct FetchedDashboard {
    pub profile: crate::infra::account_profile::UsageProfile,
    pub snapshot: DashboardSnapshot,
    pub account_identity: UsageAccountIdentity,
    pub reset_credit_availability: Option<ResetCreditAvailability>,
}

use crate::domain::usage::{parse_reset_credit_availability, parse_usage_payload};
pub(crate) use crate::domain::usage::{ResetCreditAvailability, UsageAccountIdentity};
#[derive(Clone)]
pub struct UsageClient {
    client: HttpClient,
}

impl UsageClient {
    /// 只 GET 卡片明细，复用同一客户端、代理与超时策略；原始卡 ID 不离开解析栈。
    pub async fn fetch_reset_credit_details(
        &self,
        credentials: crate::infra::auth::AuthCredentials,
    ) -> Result<Vec<crate::domain::reset_credit_details::ResetCreditItem>, UsageError> {
        self.fetch_reset_credit_details_at(credentials, RESET_CREDITS_ENDPOINT)
            .await
    }

    async fn fetch_reset_credit_details_at(
        &self,
        credentials: crate::infra::auth::AuthCredentials,
        endpoint: &str,
    ) -> Result<Vec<crate::domain::reset_credit_details::ResetCreditItem>, UsageError> {
        let mut request = self
            .client
            .build()
            .map_err(|_| UsageError::Client)?
            .get(endpoint)
            .bearer_auth(&credentials.access_token)
            .header("Accept", "application/json");
        if let Some(id) = credentials.account_id.as_deref() {
            request = request.header("ChatGPT-Account-Id", id);
        }
        let mut response = request.send().await.map_err(|_| UsageError::Network)?;
        if !response.status().is_success() {
            return Err(status_to_error(response.status()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| UsageError::Network)? {
            if bytes.len() + chunk.len() > 1024 * 1024 {
                return Err(UsageError::InvalidPayload);
            }
            bytes.extend_from_slice(&chunk);
        }
        let payload =
            serde_json::from_slice::<Value>(&bytes).map_err(|_| UsageError::InvalidPayload)?;
        crate::domain::reset_credit_details::parse_credit_details(&payload)
            .map_err(|_| UsageError::InvalidPayload)
    }
    pub fn new() -> Result<Self, UsageError> {
        Ok(Self {
            client: HttpClient::new(20),
        })
    }

    /// 仅使用 access_token 查询额度与重置卡计数；不会调用重置卡消耗或其他写入路径。
    pub async fn fetch_dashboard(&self) -> Result<FetchedDashboard, UsageError> {
        self.fetch_with_credentials(read_auth_credentials()?).await
    }

    /// 账号由调用者固定，请求期间的 UI 切换不会改变认证来源。
    pub async fn fetch_with_credentials(
        &self,
        credentials: crate::infra::auth::AuthCredentials,
    ) -> Result<FetchedDashboard, UsageError> {
        let account_identity = credentials
            .account_id
            .clone()
            .map(UsageAccountIdentity::AccountId)
            .unwrap_or_else(|| UsageAccountIdentity::Token(credentials.access_token.clone()));
        let mut request = self
            .client
            .build()
            .map_err(|_| UsageError::Client)?
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
        let bytes = response.bytes().await.map_err(|_| UsageError::Network)?;
        let payload: Value =
            serde_json::from_slice(&bytes).map_err(|_| UsageError::InvalidPayload)?;
        // 返回的身份必须属于发起请求的账号，不能把错误上游资料拼入另一张账号卡片。
        if credentials
            .account_id
            .as_deref()
            .zip(payload.get("account_id").and_then(Value::as_str))
            .is_some_and(|(expected, actual)| expected != actual)
        {
            return Err(UsageError::InvalidPayload);
        }
        Ok(FetchedDashboard {
            profile: crate::infra::account_profile::parse_usage_profile(&payload, Utc::now()),
            snapshot: parse_usage_payload(&payload).map_err(|_| UsageError::InvalidPayload)?,
            account_identity,
            reset_credit_availability: parse_reset_credit_availability(&payload),
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

#[cfg(test)]
mod tests;
