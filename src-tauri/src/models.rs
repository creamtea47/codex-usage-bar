use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
    pub status: DashboardStatus,
    /// 仅来自成功的用量响应，并已在 Rust 内完成掩码；绝不保存或传递原始邮箱。
    pub account_email_masked: Option<String>,
    pub plan_label: Option<String>,
    pub refreshed_at: Option<DateTime<Utc>>,
    pub next_refresh_at: Option<DateTime<Utc>>,
    pub message: Option<DashboardErrorCode>,
    pub quota_windows: Vec<QuotaWindow>,
}

impl Default for DashboardSnapshot {
    fn default() -> Self {
        Self {
            status: DashboardStatus::Loading,
            account_email_masked: None,
            plan_label: None,
            refreshed_at: None,
            next_refresh_at: None,
            message: None,
            quota_windows: Vec::new(),
        }
    }
}

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en")]
    En,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSettings {
    pub enabled: bool,
    pub low_quota_enabled: bool,
    pub low_quota_threshold_percent: u8,
    pub pace_enabled: bool,
    pub pace_deficit_threshold_percent: u8,
    pub reset_enabled: bool,
    pub quiet_hours_enabled: bool,
    pub quiet_hours_start: String,
    pub quiet_hours_end: String,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            low_quota_enabled: true,
            low_quota_threshold_percent: 20,
            pace_enabled: true,
            pace_deficit_threshold_percent: 10,
            reset_enabled: true,
            quiet_hours_enabled: false,
            quiet_hours_start: "22:00".to_owned(),
            quiet_hours_end: "08:00".to_owned(),
        }
    }
}

impl NotificationSettings {
    pub fn normalized(mut self) -> Self {
        self.low_quota_threshold_percent = self.low_quota_threshold_percent.min(100);
        self.pace_deficit_threshold_percent = self.pace_deficit_threshold_percent.min(100);
        if !valid_local_time(&self.quiet_hours_start) {
            self.quiet_hours_start = "22:00".to_owned();
        }
        if !valid_local_time(&self.quiet_hours_end) {
            self.quiet_hours_end = "08:00".to_owned();
        }
        self
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NotificationPermission {
    Granted,
    Denied,
    Prompt,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationEnableResult {
    pub enabled: bool,
    pub permission: NotificationPermission,
    pub settings: Settings,
}

fn valid_local_time(value: &str) -> bool {
    let Some((hour, minute)) = value.split_once(':') else {
        return false;
    };
    hour.len() == 2
        && minute.len() == 2
        && hour.parse::<u8>().is_ok_and(|value| value < 24)
        && minute.parse::<u8>().is_ok_and(|value| value < 60)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub always_on_top: bool,
    pub lock_position: bool,
    pub refresh_interval_seconds: u64,
    pub theme: Theme,
    #[serde(default)]
    pub language: Language,
    #[serde(default)]
    pub notifications: NotificationSettings,
    #[serde(default = "default_history_enabled")]
    pub history_enabled: bool,
    /// 默认开启；自动任务只检查公开签名清单，绝不自动下载、安装或重启。
    #[serde(default = "default_auto_check_updates")]
    pub auto_check_updates: bool,
}

fn default_auto_check_updates() -> bool {
    true
}

fn default_history_enabled() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            always_on_top: false,
            lock_position: false,
            refresh_interval_seconds: 60,
            theme: Theme::System,
            language: Language::System,
            notifications: NotificationSettings::default(),
            history_enabled: true,
            auto_check_updates: true,
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
        self.notifications = self.notifications.normalized();
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

/// 主悬浮卡的尺寸控制权。自动模式只会根据额度窗口数量调整高度；用户一旦手动调整，
/// 就切换到手动模式，避免后续刷新打断用户的布局选择。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MainWindowSizeMode {
    #[default]
    Auto,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StoredSettings {
    #[serde(default)]
    pub preferences: Settings,
    #[serde(default)]
    pub window_placement: Option<WindowPlacement>,
    /// 设置窗口与主悬浮卡独立保存，避免两个窗口互相覆盖位置和尺寸。
    #[serde(default)]
    pub settings_window_placement: Option<WindowPlacement>,
    /// 旧版本升级后仅执行一次紧凑布局迁移。
    #[serde(default)]
    pub compact_layout_migration_completed: bool,
    #[serde(default)]
    pub main_window_size_mode: MainWindowSizeMode,
}
