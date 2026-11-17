# 桌面 IPC 兼容基线

以包含 Plus 功能的重构前工作树为基线。表中使用前端参数名，Rust 类型简写保留可选值与结果类型。Tauri 注入窗口、应用句柄和运行时状态。权限由调用窗口标签和 capabilities 共同决定。

| 命令 | JS 参数与类型 | 返回类型 | 允许窗口 |
|---|---|---|---|
| `account_login_pending` | — | `Result<bool,String>` | settings |
| `apply_codex_account` | `accountId: String` | `Result<(),String>` | settings |
| `cancel_account_login` | — | `Result<(),String>` | settings |
| `check_app_update` | — | `Result<AppUpdateInfo,String>` | settings |
| `clear_usage_history` | `accountId: Option<String>` | `Result<(),String>` | settings |
| `enter_lightweight` | — | `Result<(),String>` | settings |
| `get_account_models` | `accountId: String` | `Result<Vec<ModelOption>,String>` | settings |
| `get_accounts` | — | `Result<AccountsResponse,String>` | main, settings |
| `get_app_update_info` | — | `Result<AppUpdateInfo,String>` | main, settings |
| `get_autostart` | — | `Result<bool,String>` | settings |
| `get_dashboard` | `accountId: Option<String>` | `Result<DashboardSnapshot,String>` | main |
| `get_diagnostics` | — | `Result<String,String>` | settings |
| `get_lightweight_measurement` | — | `Result<Option<String>,String>` | settings |
| `get_quota_auto_continue_status` | `accountId: Option<String>` | `Result<QuotaAutoContinueStatus,String>` | settings |
| `get_reset_credit_details` | `accountId: String`, `force: Option<bool>` | `Result<ResetCreditDetails,String>` | settings |
| `get_settings` | — | `Result<Settings,String>` | main, settings |
| `get_usage_analytics` | `accountId: Option<String>`, `request: AnalyticsRequest` | `Result<Value,String>` | settings |
| `get_usage_history` | `request: UsageHistoryRequest`, `accountId: Option<String>` | `Result<UsageHistoryResponse,String>` | settings |
| `import_accounts` | — | `Result<ImportResponse,String>` | settings |
| `install_app_update` | — | `Result<(),String>` | settings |
| `login_and_import` | `std: :Arc<AccountHub>>` | `Result<(),String>` | settings |
| `mark_main_size_manual` | — | `Result<(),String>` | main |
| `open_settings_window` | `section: Option<String>` | `Result<(),String>` | main |
| `refresh_dashboard` | `accountId: Option<String>` | `Result<DashboardSnapshot,String>` | main |
| `remove_account` | `accountId: String` | `Result<(),String>` | settings |
| `report_settings_ui_fault` | `faultCode: SettingsUiFaultCode` | `Result<(),String>` | settings |
| `restore_codex_account` | — | `Result<(),String>` | settings |
| `save_settings` | `settings: Settings` | `Result<Settings,String>` | settings |
| `select_account` | `accountId: String` | `Result<AccountsResponse,String>` | main, settings |
| `send_test_notification` | — | `Result<(),String>` | settings |
| `set_autostart` | `enabled: bool` | `Result<bool,String>` | settings |
| `set_history_enabled` | `enabled: bool` | `Result<Settings,String>` | settings |
| `set_notifications_enabled` | `enabled: bool` | `Result<NotificationEnableResult,String>` | settings |
| `set_quota_auto_continue_enabled` | `enabled: bool`, `accountId: Option<String>` | `Result<QuotaAutoContinueStatus,String>` | settings |
| `test_quota_auto_continue` | `accountId: Option<String>` | `Result<QuotaAutoContinueStatus,String>` | settings |
| `update_account` | `config: AccountConfig` | `Result<(),String>` | settings |

## 事件

| 名称 | 接收窗口 | 有效载荷 |
|---|---|---|
| `dashboard-updated` | main | 选中账号的 DashboardSnapshot |
| `accounts-updated` | 两个窗口 | 空，按调用窗口权限重新查询列表 |
| `account-login-finished` | settings | 空 |
| `usage-history-updated` | settings | 含可空 accountId 的对象 |
| `quota-auto-continue-updated` | settings | 当前账号的 QuotaAutoContinueStatus |
| `settings-updated` | 两个窗口 | Settings |
| `settings-navigate` | settings | about |
| `app-update-updated` | 两个窗口 | AppUpdateInfo |
| `app-update-progress` | settings | AppUpdateProgress |

`get_accounts` 的主挂件响应省略完整账号资料和登录路径。错误码、旧持久化数据和字段脱敏由相应 Rust 模块的回归用例覆盖。两套窗口权限清单在 `src-tauri/capabilities/`。
