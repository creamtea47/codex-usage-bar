use serde::{Deserialize, Serialize};

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
    #[serde(default = "enabled_by_default")]
    pub reset_credit_enabled: bool,
    #[serde(default = "enabled_by_default")]
    pub extra_credit_enabled: bool,
    #[serde(default = "enabled_by_default")]
    pub connection_enabled: bool,
    pub quiet_hours_enabled: bool,
    pub quiet_hours_start: String,
    pub quiet_hours_end: String,
}

fn enabled_by_default() -> bool {
    true
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
            reset_credit_enabled: true,
            extra_credit_enabled: true,
            connection_enabled: true,
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
    #[serde(default)]
    pub auto_lightweight: bool,
    #[serde(default = "default_lightweight_minutes")]
    pub lightweight_minutes: u32,
    pub always_on_top: bool,
    pub lock_position: bool,
    /// 默认拦截主窗口关闭并隐藏到托盘；旧设置缺少字段时也保持该安全默认。
    #[serde(default = "default_minimize_to_tray_on_close")]
    pub minimize_to_tray_on_close: bool,
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
    /// 真实发送最小请求的副作用开关，必须通过专属 IPC 修改，默认关闭。
    #[serde(default)]
    pub quota_auto_continue_enabled: bool,
}

fn default_lightweight_minutes() -> u32 {
    10
}

fn default_minimize_to_tray_on_close() -> bool {
    true
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
            auto_lightweight: false,
            lightweight_minutes: default_lightweight_minutes(),
            always_on_top: false,
            lock_position: false,
            minimize_to_tray_on_close: true,
            refresh_interval_seconds: 60,
            theme: Theme::System,
            language: Language::System,
            notifications: NotificationSettings::default(),
            history_enabled: true,
            auto_check_updates: true,
            quota_auto_continue_enabled: false,
        }
    }
}

impl Settings {
    /// 只接受菜单暴露的刷新间隔，避免损坏配置让后台轮询失控。
    pub fn normalized(mut self) -> Self {
        self.lightweight_minutes = self.lightweight_minutes.clamp(1, 1440);
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

#[cfg(test)]
mod tests;
