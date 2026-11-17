use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 更新检查只返回 HTTPS 清单解析后的安全摘要；更新包签名会在下载后验证，且不包含地址、签名或原始清单。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub checked_at: Option<DateTime<Utc>>,
}

impl Default for AppUpdateInfo {
    fn default() -> Self {
        Self {
            current_version: env!("CARGO_PKG_VERSION").to_owned(),
            latest_version: env!("CARGO_PKG_VERSION").to_owned(),
            update_available: false,
            checked_at: None,
        }
    }
}

/// 更新下载进度仅包含字节数和阶段，绝不包含下载地址、签名或任何认证资料。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AppUpdateStage {
    Downloading,
    Verifying,
    Installing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateProgress {
    pub stage: AppUpdateStage,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}
