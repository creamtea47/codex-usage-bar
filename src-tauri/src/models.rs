use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 仅传给界面的额度窗口。这个类型刻意不含账号、Token、请求头或原始响应。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    pub id: String,
    pub label: String,
    pub remaining_percent: u8,
    pub used_percent: u8,
    pub window_seconds: i64,
    pub reset_at: Option<DateTime<Utc>>,
    pub reset_after_seconds: i64,
    pub start_at: Option<DateTime<Utc>>,
    pub show_pace_marker: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSnapshot {
    pub status: DashboardStatus,
    pub plan_label: Option<String>,
    pub refreshed_at: Option<DateTime<Utc>>,
    pub next_refresh_at: Option<DateTime<Utc>>,
    pub message: Option<String>,
    pub quota_windows: Vec<QuotaWindow>,
}

impl Default for DashboardSnapshot {
    fn default() -> Self {
        Self {
            status: DashboardStatus::Loading,
            plan_label: None,
            refreshed_at: None,
            next_refresh_at: None,
            message: None,
            quota_windows: Vec::new(),
        }
    }
}

/// 更新检查只返回公开 Release 的版本与下载入口，不包含本机路径、认证信息或请求详情。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub release_url: String,
    pub download_url: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub always_on_top: bool,
    pub lock_position: bool,
    pub refresh_interval_seconds: u64,
    pub theme: Theme,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            always_on_top: false,
            lock_position: false,
            refresh_interval_seconds: 60,
            theme: Theme::System,
        }
    }
}

impl Settings {
    /// 只接受菜单暴露的刷新间隔，避免损坏配置让后台轮询失控。
    pub fn normalized(mut self) -> Self {
        self.refresh_interval_seconds = match self.refresh_interval_seconds {
            60 | 180 | 300 | 600 | 1800 => self.refresh_interval_seconds,
            _ => 60,
        };
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StoredSettings {
    #[serde(default)]
    pub preferences: Settings,
    #[serde(default)]
    pub window_placement: Option<WindowPlacement>,
}
