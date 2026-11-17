use crate::app::state::AppState;
use crate::infra::app_update::check_for_update;
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;

// 自动检查只读取公开 HTTPS 更新清单；不下载、不安装，更新包签名会在用户确认下载后验证。
pub(crate) const AUTO_UPDATE_CHECK_START_DELAY: Duration = Duration::from_secs(8);

pub(crate) const AUTO_UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

pub(crate) fn start_update_check_loop(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        // 不与主卡首屏初始化抢资源；启动后的短延迟可避免网络尚未恢复时产生无意义失败。
        tokio::time::sleep(AUTO_UPDATE_CHECK_START_DELAY).await;
        let mut enabled_receiver = state.shared.update_check_sender.subscribe();
        loop {
            if *enabled_receiver.borrow_and_update() {
                if let Err(message) = check_for_update(&app, &state.shared.app_update, true).await {
                    // command 返回的文案固定且不含 URL、签名或网络实现细节，可安全用于诊断日志。
                    log::info!("自动检查应用更新未完成：{}", message);
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(AUTO_UPDATE_CHECK_INTERVAL) => {}
                changed = enabled_receiver.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
            }
        }
    });
}
