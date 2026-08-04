mod auth;
mod models;
mod settings;
mod update;
mod usage;

use crate::{
    models::{
        DashboardSnapshot, DashboardStatus, Settings, StoredSettings, UpdateInfo, WindowPlacement,
    },
    settings::{
        cleanup_logs, load_settings, save_settings as persist_settings, update_preferences,
        update_window_placement,
    },
    update::UpdateClient,
    usage::UsageClient,
};
use chrono::{Duration as ChronoDuration, Utc};
use std::{
    io,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Position, Size, State,
    WebviewWindow, Window, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_log::{Target, TargetKind};
use tokio::sync::{watch, Mutex};

const SETTINGS_FILE_NAME: &str = "settings.json";
const GEOMETRY_SAVE_DELAY: Duration = Duration::from_millis(250);
const LOG_RETENTION: Duration = Duration::from_secs(14 * 24 * 60 * 60);

struct AppState {
    usage_client: UsageClient,
    snapshot: Mutex<DashboardSnapshot>,
    refresh_guard: Mutex<()>,
    stored_settings: Mutex<StoredSettings>,
    settings_path: PathBuf,
    interval_sender: watch::Sender<u64>,
    geometry_generation: AtomicU64,
}

impl AppState {
    fn new(
        usage_client: UsageClient,
        stored_settings: StoredSettings,
        settings_path: PathBuf,
    ) -> Self {
        let (interval_sender, _) =
            watch::channel(stored_settings.preferences.refresh_interval_seconds);
        Self {
            usage_client,
            snapshot: Mutex::new(DashboardSnapshot::default()),
            refresh_guard: Mutex::new(()),
            stored_settings: Mutex::new(stored_settings),
            settings_path,
            interval_sender,
            geometry_generation: AtomicU64::new(0),
        }
    }

    async fn current_snapshot(&self) -> DashboardSnapshot {
        self.snapshot.lock().await.clone()
    }

    async fn current_settings(&self) -> Settings {
        self.stored_settings.lock().await.preferences.clone()
    }

    async fn refresh_dashboard(&self) -> DashboardSnapshot {
        // 定时刷新与手动刷新同时到达时，复用现有快照，避免重复携带凭据发出请求。
        let Ok(_guard) = self.refresh_guard.try_lock() else {
            return self.current_snapshot().await;
        };

        let interval = self.current_settings().await.refresh_interval_seconds;
        match self.usage_client.fetch_dashboard().await {
            Ok(mut fresh) => {
                fresh.next_refresh_at = Some(Utc::now() + ChronoDuration::seconds(interval as i64));
                log::info!(
                    "用量刷新成功：窗口数={}，下次刷新间隔={}秒",
                    fresh.quota_windows.len(),
                    interval
                );
                let mut snapshot = self.snapshot.lock().await;
                *snapshot = fresh.clone();
                fresh
            }
            Err(error) => {
                // 错误对象只包含脱敏的用户提示，绝不写入 Token、认证内容或请求头。
                let mut snapshot = self.snapshot.lock().await;
                snapshot.status = if snapshot.quota_windows.is_empty() {
                    DashboardStatus::Error
                } else {
                    DashboardStatus::Stale
                };
                snapshot.message = Some(error.to_string());
                snapshot.next_refresh_at =
                    Some(Utc::now() + ChronoDuration::seconds(interval as i64));
                log::warn!("用量刷新失败：{}", error);
                snapshot.clone()
            }
        }
    }

    async fn save_preferences(&self, preferences: Settings) -> Result<Settings, String> {
        let mut stored = self.stored_settings.lock().await;
        update_preferences(&mut stored, preferences);
        persist_settings(&self.settings_path, &stored)
            .map_err(|_| "无法保存本地设置。".to_owned())?;
        let result = stored.preferences.clone();
        let _ = self.interval_sender.send(result.refresh_interval_seconds);
        log::info!(
            "已保存挂件设置：置顶={}、锁定={}、刷新间隔={}秒",
            result.always_on_top,
            result.lock_position,
            result.refresh_interval_seconds
        );
        Ok(result)
    }

    async fn save_window_placement(&self, window: &WebviewWindow) {
        let Ok(position) = window.outer_position() else {
            return;
        };
        let Ok(size) = window.outer_size() else {
            return;
        };
        let placement = WindowPlacement {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        };
        let mut stored = self.stored_settings.lock().await;
        update_window_placement(&mut stored, placement);
        if persist_settings(&self.settings_path, &stored).is_err() {
            log::warn!("无法保存窗口位置与大小。");
        }
    }
}

#[tauri::command]
async fn get_dashboard(state: State<'_, Arc<AppState>>) -> Result<DashboardSnapshot, String> {
    Ok(state.current_snapshot().await)
}

#[tauri::command]
async fn refresh_dashboard(state: State<'_, Arc<AppState>>) -> Result<DashboardSnapshot, String> {
    Ok(state.refresh_dashboard().await)
}

#[tauri::command]
async fn get_settings(state: State<'_, Arc<AppState>>) -> Result<Settings, String> {
    Ok(state.current_settings().await)
}

#[tauri::command]
async fn save_settings(
    settings: Settings,
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<Settings, String> {
    let stored = state.save_preferences(settings).await?;
    window
        .set_always_on_top(stored.always_on_top)
        .map_err(|_| "无法更新窗口置顶状态。".to_owned())?;
    Ok(stored)
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|_| "无法读取开机自启状态。".to_owned())
}

#[tauri::command]
fn set_autostart(enabled: bool, app: AppHandle) -> Result<bool, String> {
    let autostart = app.autolaunch();
    let result = if enabled {
        autostart.enable()
    } else {
        autostart.disable()
    };
    result.map_err(|_| "无法更新开机自启设置。".to_owned())?;
    log::info!("已更新开机自启状态：{}", enabled);
    Ok(enabled)
}

#[tauri::command]
async fn check_update() -> Result<UpdateInfo, String> {
    let client = UpdateClient::new().map_err(|error| error.to_string())?;
    match client.check().await {
        Ok(update) => {
            log::info!(
                "更新检查完成：当前版本={}，最新版本={}，可更新={}",
                update.current_version,
                update.latest_version,
                update.update_available
            );
            Ok(update)
        }
        Err(error) => {
            // 错误文本是固定的脱敏提示，日志不记录 URL、响应体或任何认证数据。
            log::warn!("更新检查失败：{}", error);
            Err(error.to_string())
        }
    }
}

fn start_refresh_loop(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        emit_dashboard(&app, state.refresh_dashboard().await);
        let mut interval_receiver = state.interval_sender.subscribe();
        loop {
            let interval = *interval_receiver.borrow_and_update();
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval)) => {
                    emit_dashboard(&app, state.refresh_dashboard().await);
                }
                changed = interval_receiver.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
            }
        }
    });
}

fn emit_dashboard(app: &AppHandle, snapshot: DashboardSnapshot) {
    if app.emit("dashboard-updated", snapshot).is_err() {
        log::warn!("无法向界面发送已刷新快照。");
    }
}

fn restore_window(window: &WebviewWindow, stored: &StoredSettings) {
    if let Some(placement) = stored.window_placement.as_ref() {
        let placement = clamp_placement(window, placement);
        let _ = window.set_size(Size::Physical(PhysicalSize::new(
            placement.width,
            placement.height,
        )));
        let _ = window.set_position(Position::Physical(PhysicalPosition::new(
            placement.x,
            placement.y,
        )));
    }
    let _ = window.set_always_on_top(stored.preferences.always_on_top);
}

/// 恢复位置前限制在当前显示器的可用工作区，避免任务栏或显示器变更后窗口落在屏幕外。
fn clamp_placement(window: &WebviewWindow, placement: &WindowPlacement) -> WindowPlacement {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return placement.clone();
    };
    let work_area = monitor.work_area();
    let width = placement.width.min(work_area.size.width).max(340);
    let height = placement.height.min(work_area.size.height).max(260);
    let max_x = work_area.position.x + work_area.size.width.saturating_sub(width) as i32;
    let max_y = work_area.position.y + work_area.size.height.saturating_sub(height) as i32;
    WindowPlacement {
        x: placement.x.clamp(work_area.position.x, max_x),
        y: placement.y.clamp(work_area.position.y, max_y),
        width,
        height,
    }
}

fn schedule_geometry_save(window: Window) {
    let app = window.app_handle().clone();
    let state = app.state::<Arc<AppState>>().inner().clone();
    let window_label = window.label().to_owned();
    let generation = state.geometry_generation.fetch_add(1, Ordering::Relaxed) + 1;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(GEOMETRY_SAVE_DELAY).await;
        if state.geometry_generation.load(Ordering::Relaxed) == generation {
            if let Some(webview_window) = app.get_webview_window(&window_label) {
                state.save_window_placement(&webview_window).await;
            }
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_opener::init())
        // 仅写入应用日志目录；关闭默认 stdout/Trace，避免第三方依赖输出无关运行细节。
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([Target::new(TargetKind::LogDir {
                    file_name: Some("codex-usage-bar".into()),
                })])
                .level(log::LevelFilter::Info)
                .build(),
        )
        .setup(|app| {
            let config_directory = app
                .path()
                .app_config_dir()
                .map_err(|_| io::Error::other("无法定位应用配置目录。"))?;
            let settings_path = config_directory.join(SETTINGS_FILE_NAME);
            let stored_settings = load_settings(&settings_path);
            let usage_client =
                UsageClient::new().map_err(|error| io::Error::other(error.to_string()))?;
            let state = Arc::new(AppState::new(
                usage_client,
                stored_settings.clone(),
                settings_path,
            ));
            app.manage(state.clone());

            if let Ok(log_directory) = app.path().app_log_dir() {
                cleanup_logs(&log_directory, LOG_RETENTION);
            }

            let window = app
                .get_webview_window("main")
                .ok_or_else(|| io::Error::other("无法创建主窗口。"))?;
            restore_window(&window, &stored_settings);
            window.show()?;
            start_refresh_loop(app.handle().clone(), state);
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_)) {
                schedule_geometry_save(window.clone());
            }
            if matches!(event, WindowEvent::CloseRequested { .. }) {
                schedule_geometry_save(window.clone());
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_dashboard,
            refresh_dashboard,
            get_settings,
            save_settings,
            get_autostart,
            set_autostart,
            check_update
        ])
        .run(tauri::generate_context!())
        .expect("启动 CodexUsageBar 失败");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{models::Theme, usage::UsageClient};

    #[test]
    fn only_supported_refresh_intervals_are_accepted() {
        let result = Settings {
            always_on_top: true,
            lock_position: false,
            refresh_interval_seconds: 99,
            theme: Theme::Dark,
        }
        .normalized();
        assert_eq!(result.refresh_interval_seconds, 60);
        assert!(result.always_on_top);
    }

    #[tokio::test]
    async fn concurrent_refresh_returns_the_cached_snapshot_without_second_request() {
        let state = AppState::new(
            UsageClient::new().unwrap(),
            StoredSettings::default(),
            PathBuf::from("unused-test-settings.json"),
        );
        let expected = DashboardSnapshot {
            status: DashboardStatus::Ready,
            plan_label: Some("Pro".to_owned()),
            refreshed_at: None,
            next_refresh_at: None,
            message: None,
            quota_windows: Vec::new(),
        };
        *state.snapshot.lock().await = expected.clone();

        // 占住刷新锁后，第二次请求必须直接返回缓存，不能读取 auth.json 或发起 HTTP 请求。
        let _first_refresh = state.refresh_guard.lock().await;
        assert_eq!(state.refresh_dashboard().await, expected);
    }
}
