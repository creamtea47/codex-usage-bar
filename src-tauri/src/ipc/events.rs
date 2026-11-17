use crate::app::account_hub;
use crate::app::account_hub::AccountHub;
use crate::app::state::AppState;
use crate::domain::usage::DashboardSnapshot;
use crate::infra::desktop::MAIN_WINDOW_LABEL;
use crate::infra::desktop::SETTINGS_WINDOW_LABEL;
use chrono::Utc;
use std::sync::Arc;
use tauri::AppHandle;
use tauri::Emitter;
use tauri::Manager;

pub(crate) fn emit_quota_auto_continue_status(app: &AppHandle, state: &AppState) {
    let enabled = state.current_settings().quota_auto_continue_enabled;
    let mut status = state.quota_auto_continue.status(enabled, Utc::now());
    status.account_id = state.account.as_ref().map(|a| a.id.clone());
    if !account_hub::is_selected(app, state) {
        return;
    }
    if app
        .emit_to(SETTINGS_WINDOW_LABEL, "quota-auto-continue-updated", status)
        .is_err()
    {
        log::warn!("无法向设置窗口广播额度自动接续状态。");
    }
}

pub(crate) fn emit_usage_history_updated(app: &AppHandle, account_id: Option<String>) {
    if app
        .emit_to(
            SETTINGS_WINDOW_LABEL,
            "usage-history-updated",
            serde_json::json!({ "accountId": account_id }),
        )
        .is_err()
    {
        log::warn!("无法向设置窗口广播本地趋势更新。");
    }
}

pub(crate) fn emit_dashboard(app: &AppHandle, snapshot: DashboardSnapshot) {
    if app
        .try_state::<Arc<AccountHub>>()
        .is_some_and(|hub| hub.store.selected_id() != snapshot.account_id)
    {
        return;
    }
    // 即便设置窗口拥有通用事件监听能力，也不能收到主卡专属的账号摘要和额度快照。
    if app
        .emit_to(MAIN_WINDOW_LABEL, "dashboard-updated", snapshot)
        .is_err()
    {
        log::warn!("无法向主悬浮卡发送已刷新快照。");
    }
}
