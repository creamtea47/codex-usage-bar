use crate::{
    models::{AppUpdateInfo, AppUpdateProgress, AppUpdateStage},
    SETTINGS_WINDOW_LABEL,
};
use chrono::Utc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Error as UpdaterError, Update, UpdaterExt};
use tokio::sync::Mutex as AsyncMutex;

pub const APP_UPDATE_EVENT: &str = "app-update-updated";
pub const APP_UPDATE_PROGRESS_EVENT: &str = "app-update-progress";

/// 待安装对象只保存在内存中。它带有下载地址和签名，因此绝不经 IPC、日志或设置文件输出。
struct PendingAppUpdate {
    update: Option<Update>,
    info: AppUpdateInfo,
}

impl Default for PendingAppUpdate {
    fn default() -> Self {
        Self {
            update: None,
            info: AppUpdateInfo::default(),
        }
    }
}

/// 更新检查与下载共用操作锁，避免并发命令重复下载或替换应用。
pub struct AppUpdateState {
    pending: AsyncMutex<PendingAppUpdate>,
    operation_guard: AsyncMutex<()>,
}

impl Default for AppUpdateState {
    fn default() -> Self {
        Self {
            pending: AsyncMutex::new(PendingAppUpdate::default()),
            operation_guard: AsyncMutex::new(()),
        }
    }
}

impl AppUpdateState {
    pub async fn current_info(&self) -> AppUpdateInfo {
        self.pending.lock().await.info.clone()
    }

    async fn clear_pending(&self) {
        self.pending.lock().await.update = None;
    }

    async fn replace_pending(&self, update: Option<Update>, info: AppUpdateInfo) {
        let mut pending = self.pending.lock().await;
        pending.update = update;
        pending.info = info;
    }

    async fn take_pending(&self) -> Option<Update> {
        self.pending.lock().await.update.take()
    }

    async fn restore_pending(&self, update: Update) {
        self.pending.lock().await.update = Some(update);
    }
}

/// 检查到更新后只保留版本等安全摘要；更新包会在下载后验签，签名、下载 URL 和原始 Release JSON 永远留在 Rust 内存。
pub async fn check_for_update(
    app: &AppHandle,
    state: &AppUpdateState,
    automatic: bool,
) -> Result<AppUpdateInfo, String> {
    let _operation = state
        .operation_guard
        .try_lock()
        .map_err(|_| "更新操作正在进行，请稍后再试。".to_owned())?;

    // 每次检查先废弃旧对象，确保失败后不会安装过期的待更新包。
    state.clear_pending().await;
    let current_version = env!("CARGO_PKG_VERSION").to_owned();
    let checked_at = Utc::now();
    let updater = match app
        .updater_builder()
        // 该插件沿用系统代理；仅设置超时，避免网络异常阻塞桌面交互。
        .timeout(Duration::from_secs(20))
        .build()
    {
        Ok(updater) => updater,
        Err(error) => {
            return record_check_failure(state, current_version, checked_at, automatic, &error).await;
        }
    };

    match updater.check().await {
        Ok(update) => {
            let info = match update.as_ref() {
                Some(update) => AppUpdateInfo {
                    current_version,
                    latest_version: update.version.clone(),
                    update_available: true,
                    checked_at: Some(checked_at),
                },
                None => AppUpdateInfo {
                    current_version: current_version.clone(),
                    latest_version: current_version,
                    update_available: false,
                    checked_at: Some(checked_at),
                },
            };
            state.replace_pending(update, info.clone()).await;
            emit_update_info(app, &info);
            log::info!(
                "应用更新检查完成：来源={}，结果={}，当前版本={}，最新版本={}",
                if automatic { "自动" } else { "手动" },
                if info.update_available { "发现新版本" } else { "已是最新" },
                info.current_version,
                info.latest_version
            );
            Ok(info)
        }
        Err(error) => record_check_failure(state, current_version, checked_at, automatic, &error).await,
    }
}

async fn record_check_failure(
    state: &AppUpdateState,
    current_version: String,
    checked_at: chrono::DateTime<Utc>,
    automatic: bool,
    error: &UpdaterError,
) -> Result<AppUpdateInfo, String> {
    let info = AppUpdateInfo {
        latest_version: current_version.clone(),
        current_version,
        update_available: false,
        checked_at: Some(checked_at),
    };
    state.replace_pending(None, info).await;
    // 仅保存归类结果，避免第三方错误文本把 URL、代理细节或远端响应写入本地日志。
    log::warn!(
        "应用更新检查失败：来源={}，类别={}",
        if automatic { "自动" } else { "手动" },
        error_category(error)
    );
    Err(user_message_for_error(error).to_owned())
}

/// 下载、验签和安装都在 Rust 中完成。Windows 交接安装器时会由插件结束当前进程；
/// macOS 在替换 App 后显式重启。失败时保留同一个已检查候选，允许用户重新下载并验签。
pub async fn install_pending_update(app: &AppHandle, state: &AppUpdateState) -> Result<(), String> {
    let _operation = state
        .operation_guard
        .try_lock()
        .map_err(|_| "更新操作正在进行，请稍后再试。".to_owned())?;
    let update = state
        .take_pending()
        .await
        .ok_or_else(|| "没有待安装的更新，请先重新检查更新。".to_owned())?;

    log::info!("应用更新下载开始：目标版本={}", update.version);
    emit_update_progress(
        app,
        AppUpdateProgress {
            stage: AppUpdateStage::Downloading,
            downloaded_bytes: 0,
            total_bytes: None,
        },
    );

    let progress_app = app.clone();
    let verify_app = app.clone();
    let mut downloaded_bytes = 0_u64;
    let bytes = update
        .download(
            move |chunk_size, total_bytes| {
                downloaded_bytes = downloaded_bytes.saturating_add(chunk_size as u64);
                emit_update_progress(
                    &progress_app,
                    AppUpdateProgress {
                        stage: AppUpdateStage::Downloading,
                        downloaded_bytes,
                        total_bytes,
                    },
                );
            },
            move || {
                emit_update_progress(
                    &verify_app,
                    AppUpdateProgress {
                        stage: AppUpdateStage::Verifying,
                        downloaded_bytes: 0,
                        total_bytes: None,
                    },
                );
            },
        )
        .await;

    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(error) => {
            log::warn!("应用更新下载或验签失败：类别={}", error_category(&error));
            state.restore_pending(update).await;
            return Err(user_message_for_error(&error).to_owned());
        }
    };

    emit_update_progress(
        app,
        AppUpdateProgress {
            stage: AppUpdateStage::Installing,
            downloaded_bytes: 0,
            total_bytes: None,
        },
    );
    log::info!("应用更新已完成签名验证，正在交接安装：目标版本={}", update.version);

    if let Err(error) = update.install(bytes) {
        log::warn!("应用更新安装交接失败：类别={}", error_category(&error));
        state.restore_pending(update).await;
        return Err(user_message_for_error(&error).to_owned());
    }

    #[cfg(target_os = "macos")]
    {
        log::info!("应用更新替换完成，正在重启应用。");
        app.restart();
    }

    // Windows 的 `install` 会接管 NSIS 安装器并结束进程；其余桌面平台保留安全成功返回。
    #[cfg(not(target_os = "macos"))]
    Ok(())
}

fn emit_update_info(app: &AppHandle, info: &AppUpdateInfo) {
    if app.emit(APP_UPDATE_EVENT, info.clone()).is_err() {
        log::warn!("无法向界面广播应用更新状态。");
    }
}

fn emit_update_progress(app: &AppHandle, progress: AppUpdateProgress) {
    if app
        .emit_to(SETTINGS_WINDOW_LABEL, APP_UPDATE_PROGRESS_EVENT, progress)
        .is_err()
    {
        log::warn!("无法向设置窗口广播应用更新进度。");
    }
}

fn error_category(error: &UpdaterError) -> &'static str {
    match error {
        UpdaterError::Minisign(_) | UpdaterError::Base64(_) | UpdaterError::SignatureUtf8(_) => {
            "签名"
        }
        UpdaterError::ReleaseNotFound
        | UpdaterError::Serialization(_)
        | UpdaterError::TargetNotFound(_)
        | UpdaterError::TargetsNotFound(_) => "更新清单",
        UpdaterError::Reqwest(_) | UpdaterError::Network(_) => "网络",
        UpdaterError::InsecureTransportProtocol | UpdaterError::EmptyEndpoints => "配置",
        _ => "安装或系统",
    }
}

fn user_message_for_error(error: &UpdaterError) -> &'static str {
    match error_category(error) {
        "签名" => "更新包的签名验证失败，已取消安装。",
        "更新清单" => "暂时没有适用于当前设备的有效更新。",
        "网络" => "无法连接更新服务，请检查网络或代理后重试。",
        "配置" => "更新服务配置不可用，请稍后重试。",
        _ => "更新安装未完成，请稍后重试。",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_info_never_contains_transport_or_signature_fields() {
        let serialized = serde_json::to_string(&AppUpdateInfo {
            current_version: "0.2.5".to_owned(),
            latest_version: "0.2.6".to_owned(),
            update_available: true,
            checked_at: None,
        })
        .unwrap();
        assert!(!serialized.contains("signature"));
        assert!(!serialized.contains("downloadUrl"));
        assert!(!serialized.contains("rawJson"));
    }

    #[test]
    fn signature_failures_have_a_safe_specific_message() {
        let error = UpdaterError::SignatureUtf8("not-a-real-signature".to_owned());
        assert_eq!(error_category(&error), "签名");
        assert_eq!(user_message_for_error(&error), "更新包的签名验证失败，已取消安装。");
    }
}
