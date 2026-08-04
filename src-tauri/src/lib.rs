mod auth;
mod models;
mod settings;
mod update;
mod usage;

use crate::{
    models::{
        DashboardSnapshot, DashboardStatus, MainWindowSizeMode, Settings, StoredSettings,
        UpdateInfo, WindowPlacement,
    },
    settings::{
        apply_compact_layout_migration, cleanup_logs, load_settings,
        save_settings as persist_settings, update_main_window_placement, update_preferences,
        update_settings_window_placement,
    },
    update::UpdateClient,
    usage::UsageClient,
};
use chrono::{Duration as ChronoDuration, Utc};
use std::{
    collections::HashMap,
    io,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Position, Size,
    State, WebviewWindow, Window, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_log::{Target, TargetKind};
use tokio::sync::{watch, Mutex as AsyncMutex};

const SETTINGS_FILE_NAME: &str = "settings.json";
const MAIN_WINDOW_LABEL: &str = "main";
const SETTINGS_WINDOW_LABEL: &str = "settings";
const GEOMETRY_SAVE_DELAY: Duration = Duration::from_millis(250);
const LOG_RETENTION: Duration = Duration::from_secs(14 * 24 * 60 * 60);

// 悬浮卡的一张额度窗口尺寸。260px 恰好容纳成功态的一张额度卡，避免底部无效留白；
// 窗口数量增加时只扩展高度，最高后由前端滚动额度区域。
const MAIN_COMPACT_HEIGHT: u32 = 260;
const MAIN_COMPACT_HEIGHT_PER_EXTRA_WINDOW: u32 = 145;
const MAIN_COMPACT_MAX_HEIGHT: u32 = 560;
const MAIN_MIN_WIDTH: u32 = 340;
const MAIN_MIN_HEIGHT: u32 = 260;
const SETTINGS_MIN_WIDTH: u32 = 620;
const SETTINGS_MIN_HEIGHT: u32 = 420;

struct AppState {
    usage_client: UsageClient,
    snapshot: AsyncMutex<DashboardSnapshot>,
    refresh_guard: AsyncMutex<()>,
    // 设置读取与几何保存都只访问很小的本地 JSON；使用同步锁以便关闭窗口前可靠落盘。
    stored_settings: StdMutex<StoredSettings>,
    settings_path: PathBuf,
    interval_sender: watch::Sender<u64>,
    // 每个窗口独立去抖，避免设置窗口的调整取消主悬浮卡的几何保存。
    geometry_generations: StdMutex<HashMap<String, u64>>,
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
            snapshot: AsyncMutex::new(DashboardSnapshot::default()),
            refresh_guard: AsyncMutex::new(()),
            stored_settings: StdMutex::new(stored_settings),
            settings_path,
            interval_sender,
            geometry_generations: StdMutex::new(HashMap::new()),
        }
    }

    fn stored_settings(&self) -> std::sync::MutexGuard<'_, StoredSettings> {
        // 发生写入线程异常时仍应让用户继续使用已知设置，而不是因锁中毒退出应用。
        self.stored_settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    async fn current_snapshot(&self) -> DashboardSnapshot {
        self.snapshot.lock().await.clone()
    }

    fn current_settings(&self) -> Settings {
        self.stored_settings().preferences.clone()
    }

    async fn refresh_dashboard(&self) -> DashboardSnapshot {
        // 定时刷新与手动刷新同时到达时，复用现有快照，避免重复携带凭据发出请求。
        let Ok(_guard) = self.refresh_guard.try_lock() else {
            return self.current_snapshot().await;
        };

        let interval = self.current_settings().refresh_interval_seconds;
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

    fn save_preferences(&self, preferences: Settings) -> Result<Settings, String> {
        let result = {
            let mut stored = self.stored_settings();
            // 先对副本落盘，失败时既不污染内存，也不会把轮询间隔切到未保存的值。
            let mut candidate = stored.clone();
            update_preferences(&mut candidate, preferences);
            persist_settings(&self.settings_path, &candidate)
                .map_err(|_| "无法保存本地设置。".to_owned())?;
            let result = candidate.preferences.clone();
            *stored = candidate;
            result
        };
        let _ = self.interval_sender.send(result.refresh_interval_seconds);
        log::info!(
            "已保存挂件设置：置顶={}、锁定={}、刷新间隔={}秒",
            result.always_on_top,
            result.lock_position,
            result.refresh_interval_seconds
        );
        Ok(result)
    }

    fn save_window_placement_for_label(&self, app: &AppHandle, label: &str) {
        let Some(window) = app.get_webview_window(label) else {
            return;
        };
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
        let mut stored = self.stored_settings();
        match label {
            MAIN_WINDOW_LABEL => update_main_window_placement(&mut stored, placement),
            SETTINGS_WINDOW_LABEL => update_settings_window_placement(&mut stored, placement),
            _ => return,
        }
        if persist_settings(&self.settings_path, &stored).is_err() {
            log::warn!("无法保存窗口位置与大小：窗口={label}。");
        }
    }

    fn next_geometry_generation(&self, label: &str) -> u64 {
        let mut generations = self
            .geometry_generations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let generation = generations.entry(label.to_owned()).or_insert(0);
        *generation += 1;
        *generation
    }

    fn is_current_geometry_generation(&self, label: &str, generation: u64) -> bool {
        self.geometry_generations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(label)
            .is_some_and(|current| *current == generation)
    }

    /// 只有主卡真实开始边缘拖拽时才切换手动尺寸。不能根据 `Resized` 推断，
    /// 因为启动恢复、DPI 切换和 Tauri 的程序化调整都会产生同样的窗口事件。
    fn mark_main_size_manual(&self) -> Result<(), String> {
        let mut stored = self.stored_settings();
        if stored.main_window_size_mode == MainWindowSizeMode::Manual {
            return Ok(());
        }
        let mut candidate = stored.clone();
        candidate.main_window_size_mode = MainWindowSizeMode::Manual;
        persist_settings(&self.settings_path, &candidate)
            .map_err(|_| "无法保存手动窗口尺寸模式。".to_owned())?;
        *stored = candidate;
        log::info!("主悬浮卡已切换为手动尺寸模式。");
        Ok(())
    }

    /// 自动模式只按窗口数量收敛高度，不触碰用户的位置和宽度。
    fn apply_auto_main_height(&self, app: &AppHandle, snapshot: &DashboardSnapshot) {
        let (size_mode, locked) = {
            let stored = self.stored_settings();
            (
                stored.main_window_size_mode,
                stored.preferences.lock_position,
            )
        };
        if size_mode != MainWindowSizeMode::Auto || locked {
            return;
        }
        let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
            return;
        };
        let Ok(current_size) = window.outer_size() else {
            return;
        };
        let Ok(scale_factor) = window.scale_factor() else {
            return;
        };
        let target_height = compact_height(snapshot.quota_windows.len());
        // tauri.conf.json 的尺寸是逻辑像素；自动高度必须保持同一单位，
        // 否则 125%/150% DPI 下会把 260 错当作物理像素而压扁主卡。
        let current_logical = current_size.to_logical::<f64>(scale_factor);
        let target_size = LogicalSize::new(
            current_logical.width.max(f64::from(MAIN_MIN_WIDTH)),
            f64::from(target_height),
        );
        if (current_logical.height - target_size.height).abs() < 0.5 {
            return;
        }

        if window.set_size(Size::Logical(target_size)).is_err() {
            log::warn!("无法按额度窗口数量更新主悬浮卡高度。");
        }
    }
}

/// Command 的调用窗口不是前端可伪造参数。这里在 Rust 端二次限定窗口标签，
/// 避免独立设置页取得主悬浮卡不需要的用量数据，或主卡直接执行设置副作用。
fn require_window_label(window: &WebviewWindow, expected_label: &str) -> Result<(), String> {
    if has_exact_window_label(window.label(), expected_label) {
        Ok(())
    } else {
        Err("当前窗口无权执行该操作。".to_owned())
    }
}

fn require_known_window(window: &WebviewWindow) -> Result<(), String> {
    if is_known_window_label(window.label()) {
        Ok(())
    } else {
        Err("当前窗口无权执行该操作。".to_owned())
    }
}

fn has_exact_window_label(actual_label: &str, expected_label: &str) -> bool {
    actual_label == expected_label
}

fn is_known_window_label(label: &str) -> bool {
    matches!(label, MAIN_WINDOW_LABEL | SETTINGS_WINDOW_LABEL)
}

#[tauri::command]
async fn get_dashboard(
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<DashboardSnapshot, String> {
    // 此命令只服务主悬浮卡，设置窗口不会接触用量快照。
    require_window_label(&window, MAIN_WINDOW_LABEL)?;
    Ok(state.current_snapshot().await)
}

#[tauri::command]
async fn refresh_dashboard(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<DashboardSnapshot, String> {
    require_window_label(&window, MAIN_WINDOW_LABEL)?;
    let snapshot = state.refresh_dashboard().await;
    state.apply_auto_main_height(&app, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
async fn get_settings(
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<Settings, String> {
    // 主卡用于首屏主题/锁定状态，设置窗口用于编辑；两者均只收到非敏感偏好。
    require_known_window(&window)?;
    Ok(state.current_settings())
}

#[tauri::command]
async fn save_settings(
    settings: Settings,
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Settings, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let next_settings = settings.normalized();
    let previous_settings = state.current_settings();
    let main_window = app
        .get_webview_window(MAIN_WINDOW_LABEL)
        .ok_or_else(|| "无法定位主悬浮卡。".to_owned())?;
    // 先应用可回滚的原生窗口状态，成功后才落盘，避免“UI 报错但配置已保存”。
    main_window
        .set_always_on_top(next_settings.always_on_top)
        .map_err(|_| "无法更新窗口置顶状态。".to_owned())?;
    if main_window
        .set_resizable(!next_settings.lock_position)
        .is_err()
    {
        let _ = main_window.set_always_on_top(previous_settings.always_on_top);
        return Err("无法更新窗口锁定状态。".to_owned());
    }
    let stored = match state.save_preferences(next_settings) {
        Ok(saved) => saved,
        Err(error) => {
            let _ = main_window.set_always_on_top(previous_settings.always_on_top);
            let _ = main_window.set_resizable(!previous_settings.lock_position);
            log::warn!("挂件设置持久化失败，已尝试恢复主窗口状态。");
            return Err(error);
        }
    };
    if app.emit("settings-updated", stored.clone()).is_err() {
        log::warn!("无法向界面广播已保存设置。");
    }
    Ok(stored)
}

#[tauri::command]
fn open_settings_window(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    require_window_label(&window, MAIN_WINDOW_LABEL)?;
    let window = app
        .get_webview_window(SETTINGS_WINDOW_LABEL)
        .ok_or_else(|| "无法创建设置窗口。".to_owned())?;
    window.show().map_err(|_| "无法显示设置窗口。".to_owned())?;
    window
        .set_focus()
        .map_err(|_| "无法聚焦设置窗口。".to_owned())?;
    log::info!("已打开设置窗口。");
    Ok(())
}

#[tauri::command]
fn mark_main_size_manual(
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    require_window_label(&window, MAIN_WINDOW_LABEL)?;
    state.mark_main_size_manual()
}

#[tauri::command]
fn get_autostart(window: WebviewWindow, app: AppHandle) -> Result<bool, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    app.autolaunch()
        .is_enabled()
        .map_err(|_| "无法读取开机自启状态。".to_owned())
}

#[tauri::command]
fn set_autostart(enabled: bool, window: WebviewWindow, app: AppHandle) -> Result<bool, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
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
async fn check_update(window: WebviewWindow) -> Result<UpdateInfo, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
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
        refresh_and_emit(&app, &state).await;
        let mut interval_receiver = state.interval_sender.subscribe();
        loop {
            let interval = *interval_receiver.borrow_and_update();
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval)) => {
                    refresh_and_emit(&app, &state).await;
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

async fn refresh_and_emit(app: &AppHandle, state: &AppState) {
    let snapshot = state.refresh_dashboard().await;
    state.apply_auto_main_height(app, &snapshot);
    emit_dashboard(app, snapshot);
}

fn emit_dashboard(app: &AppHandle, snapshot: DashboardSnapshot) {
    // 即便设置窗口拥有通用事件监听能力，也不能收到主卡专属的账号摘要和额度快照。
    if app
        .emit_to(MAIN_WINDOW_LABEL, "dashboard-updated", snapshot)
        .is_err()
    {
        log::warn!("无法向主悬浮卡发送已刷新快照。");
    }
}

fn compact_height(window_count: usize) -> u32 {
    MAIN_COMPACT_HEIGHT
        .saturating_add(
            (window_count.saturating_sub(1) as u32)
                .saturating_mul(MAIN_COMPACT_HEIGHT_PER_EXTRA_WINDOW),
        )
        .min(MAIN_COMPACT_MAX_HEIGHT)
}

fn restore_main_window(window: &WebviewWindow, stored: &StoredSettings, _state: &AppState) {
    if let Some(placement) = stored.window_placement.as_ref() {
        let placement = clamp_placement(window, placement, MAIN_MIN_WIDTH, MAIN_MIN_HEIGHT);
        apply_window_placement(window, &placement);
    }
    // 只有主悬浮卡可以置顶和锁定尺寸；设置窗口始终保持普通应用窗口行为。
    let _ = window.set_always_on_top(stored.preferences.always_on_top);
    let _ = window.set_resizable(!stored.preferences.lock_position);
}

fn restore_settings_window(window: &WebviewWindow, stored: &StoredSettings) {
    if let Some(placement) = stored.settings_window_placement.as_ref() {
        let placement = clamp_placement(window, placement, SETTINGS_MIN_WIDTH, SETTINGS_MIN_HEIGHT);
        apply_window_placement(window, &placement);
    }
}

fn apply_window_placement(window: &WebviewWindow, placement: &WindowPlacement) {
    let _ = window.set_size(Size::Physical(PhysicalSize::new(
        placement.width,
        placement.height,
    )));
    let _ = window.set_position(Position::Physical(PhysicalPosition::new(
        placement.x,
        placement.y,
    )));
}

/// 恢复位置前限制在当前显示器的可用工作区，避免任务栏或显示器变更后窗口落在屏幕外。
fn clamp_placement(
    window: &WebviewWindow,
    placement: &WindowPlacement,
    minimum_width: u32,
    minimum_height: u32,
) -> WindowPlacement {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return placement.clone();
    };
    let work_area = monitor.work_area();
    let width = placement.width.min(work_area.size.width).max(minimum_width);
    let height = placement
        .height
        .min(work_area.size.height)
        .max(minimum_height);
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
    let generation = state.next_geometry_generation(&window_label);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(GEOMETRY_SAVE_DELAY).await;
        if state.is_current_geometry_generation(&window_label, generation) {
            state.save_window_placement_for_label(&app, &window_label);
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
            let mut stored_settings = load_settings(&settings_path);
            if apply_compact_layout_migration(&mut stored_settings, MAIN_COMPACT_HEIGHT) {
                // 迁移只写本应用 settings.json，不读取或改写 Codex 的 auth.json。
                persist_settings(&settings_path, &stored_settings)
                    .map_err(|_| io::Error::other("无法保存紧凑布局迁移。"))?;
                log::info!("已完成一次性紧凑布局迁移。");
            }

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

            let main_window = app
                .get_webview_window(MAIN_WINDOW_LABEL)
                .ok_or_else(|| io::Error::other("无法创建主窗口。"))?;
            let settings_window = app
                .get_webview_window(SETTINGS_WINDOW_LABEL)
                .ok_or_else(|| io::Error::other("无法创建设置窗口。"))?;
            restore_main_window(&main_window, &stored_settings, &state);
            restore_settings_window(&settings_window, &stored_settings);
            // 静态窗口在配置中默认隐藏；显式隐藏保证升级时不会与主卡同时出现。
            let _ = settings_window.hide();
            main_window.show()?;
            start_refresh_loop(app.handle().clone(), state);
            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                WindowEvent::Moved(_) => schedule_geometry_save(window.clone()),
                WindowEvent::Resized(_) => schedule_geometry_save(window.clone()),
                WindowEvent::CloseRequested { api, .. } => {
                    let app = window.app_handle().clone();
                    let state = app.state::<Arc<AppState>>().inner().clone();
                    match window.label() {
                        SETTINGS_WINDOW_LABEL => {
                            // 关闭设置仅隐藏其独立窗口，主悬浮卡继续运行。
                            api.prevent_close();
                            state.save_window_placement_for_label(&app, SETTINGS_WINDOW_LABEL);
                            if window.hide().is_err() {
                                log::warn!("无法隐藏设置窗口。");
                            }
                        }
                        MAIN_WINDOW_LABEL => {
                            // 关闭主卡就是退出整个应用。先同步保存几何，避免退出竞态丢失用户布局。
                            api.prevent_close();
                            state.save_window_placement_for_label(&app, MAIN_WINDOW_LABEL);
                            log::info!("主悬浮卡已关闭，应用即将退出。");
                            app.exit(0);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_dashboard,
            refresh_dashboard,
            get_settings,
            save_settings,
            open_settings_window,
            mark_main_size_manual,
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

    #[test]
    fn compact_height_grows_by_window_and_stops_at_maximum() {
        assert_eq!(compact_height(0), MAIN_COMPACT_HEIGHT);
        assert_eq!(compact_height(1), MAIN_COMPACT_HEIGHT);
        assert_eq!(
            compact_height(2),
            MAIN_COMPACT_HEIGHT + MAIN_COMPACT_HEIGHT_PER_EXTRA_WINDOW
        );
        assert_eq!(compact_height(9), MAIN_COMPACT_MAX_HEIGHT);
    }

    #[test]
    fn command_window_scopes_do_not_overlap() {
        assert!(has_exact_window_label(MAIN_WINDOW_LABEL, MAIN_WINDOW_LABEL));
        assert!(!has_exact_window_label(
            SETTINGS_WINDOW_LABEL,
            MAIN_WINDOW_LABEL
        ));
        assert!(is_known_window_label(MAIN_WINDOW_LABEL));
        assert!(is_known_window_label(SETTINGS_WINDOW_LABEL));
        assert!(!is_known_window_label("untrusted-window"));
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
            account_email_masked: Some("p***@example.com".to_owned()),
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
