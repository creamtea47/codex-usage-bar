mod app_update;
mod auth;
mod models;
mod notification_rules;
mod refresh_scheduler;
mod settings;
mod usage;
mod usage_history;

use crate::{
    app_update::{check_for_update, install_pending_update, AppUpdateState},
    models::{
        AppUpdateInfo, DashboardSnapshot, DashboardStatus, ForecastStatus as ModelForecastStatus,
        Language, MainWindowSizeMode, NotificationEnableResult, NotificationPermission,
        QuotaFallbackLabel, QuotaForecast, Settings, StoredSettings, WindowPlacement,
    },
    notification_rules::{
        NotificationBatch, NotificationPolicy, NotificationReason, NotificationSnapshot,
        NotificationTracker, NotificationWindow, QuietHours,
    },
    refresh_scheduler::failure_retry_seconds,
    settings::{
        apply_compact_layout_migration, cleanup_logs, load_settings,
        save_settings as persist_settings, update_main_window_placement, update_preferences,
        update_settings_window_placement,
    },
    usage::{UsageAccountIdentity, UsageClient},
    usage_history::{
        clear_history_storage, load_history, save_history, AccountIdentity, AccountSelection,
        Forecast as HistoryForecast, ForecastStatus as HistoryForecastStatus, HistoryStorageStatus,
        HistoryWindowInput, UsageHistory, UsageHistoryRange, USAGE_HISTORY_FILE_NAME,
    },
};
use chrono::{DateTime, Duration as ChronoDuration, Local, Timelike, Utc};
use serde::Serialize;
use std::{
    collections::HashMap,
    fmt::Write as _,
    io,
    path::PathBuf,
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};
use tauri::{
    plugin::PermissionState, AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition,
    PhysicalSize, Position, Size, State, WebviewWindow, Window, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_log::{Target, TargetKind};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::{watch, Mutex as AsyncMutex};

const SETTINGS_FILE_NAME: &str = "settings.json";
const MAIN_WINDOW_LABEL: &str = "main";
pub(crate) const SETTINGS_WINDOW_LABEL: &str = "settings";
const GEOMETRY_SAVE_DELAY: Duration = Duration::from_millis(250);
const LOG_RETENTION: Duration = Duration::from_secs(14 * 24 * 60 * 60);
// 自动检查只读取公开 HTTPS 更新清单；不下载、不安装，更新包签名会在用户确认下载后验证。
const AUTO_UPDATE_CHECK_START_DELAY: Duration = Duration::from_secs(8);
const AUTO_UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

// 悬浮卡的一张额度窗口尺寸。260px 恰好容纳成功态的一张额度卡，避免底部无效留白；
// 窗口数量增加时只扩展高度，最高后由前端滚动额度区域。
const MAIN_COMPACT_HEIGHT: u32 = 260;
const MAIN_COMPACT_HEIGHT_PER_EXTRA_WINDOW: u32 = 145;
const MAIN_COMPACT_MAX_HEIGHT: u32 = 560;
const MAIN_MIN_WIDTH: u32 = 340;
const MAIN_MIN_HEIGHT: u32 = 260;
const SETTINGS_MIN_WIDTH: u32 = 620;
const SETTINGS_MIN_HEIGHT: u32 = 420;

fn refresh_completed_since(observed_generation: u64, current_generation: u64) -> bool {
    observed_generation != current_generation
}

fn scheduled_deadline_is_current(
    expected_revision: u64,
    current_revision: u64,
    expected_deadline: Option<DateTime<Utc>>,
    current_deadline: Option<DateTime<Utc>>,
) -> bool {
    expected_revision == current_revision && expected_deadline == current_deadline
}

struct AppState {
    usage_client: UsageClient,
    snapshot: AsyncMutex<DashboardSnapshot>,
    refresh_guard: AsyncMutex<()>,
    /// 区分“另一个刷新正在执行”和“历史维护暂时占用刷新屏障”。
    /// 等待者只有观察到代次推进时才复用结果，否则会在屏障释放后真正刷新。
    refresh_generation: AtomicU64,
    /// 串行化 deadline 的读取、验证和写入，避免在途刷新覆盖刚保存的刷新间隔。
    schedule_guard: AsyncMutex<()>,
    // 设置读取与几何保存都只访问很小的本地 JSON；使用同步锁以便关闭窗口前可靠落盘。
    stored_settings: StdMutex<StoredSettings>,
    settings_path: PathBuf,
    history_path: PathBuf,
    usage_history: AsyncMutex<HistoryRuntime>,
    /// 让通知设置发布与 baseline 重建相对通知评估保持原子。
    notification_evaluation_guard: StdMutex<()>,
    notification_tracker: StdMutex<NotificationTracker>,
    /// 值本身只是修订号；每次真实 deadline 变化都会唤醒并取消旧 sleep。
    schedule_sender: watch::Sender<u64>,
    consecutive_failures: AtomicU32,
    update_check_sender: watch::Sender<bool>,
    app_update: AppUpdateState,
    // 每个窗口独立去抖，避免设置窗口的调整取消主悬浮卡的几何保存。
    geometry_generations: StdMutex<HashMap<String, u64>>,
}

struct HistoryRuntime {
    history: UsageHistory,
    storage_status: HistoryStorageStatus,
}

#[derive(Debug, Clone, Copy, Default)]
struct HistoryRefreshEffect {
    changed: bool,
    account_changed: bool,
}

struct RefreshResult {
    snapshot: DashboardSnapshot,
    was_successful: bool,
    should_emit: bool,
    history_effect: HistoryRefreshEffect,
}

impl AppState {
    fn new(
        usage_client: UsageClient,
        stored_settings: StoredSettings,
        settings_path: PathBuf,
    ) -> Self {
        let (schedule_sender, _) = watch::channel(0_u64);
        let (update_check_sender, _) =
            watch::channel(stored_settings.preferences.auto_check_updates);
        let history_path = settings_path.with_file_name(USAGE_HISTORY_FILE_NAME);
        let loaded_history = load_history(&history_path);
        let mut history_status = loaded_history.status;
        if loaded_history.needs_rewrite {
            let discard_unknown_contents = matches!(
                history_status,
                HistoryStorageStatus::RecoveredCorrupt
                    | HistoryStorageStatus::RecoveredUnsupported
                    | HistoryStorageStatus::Unavailable
            );
            let rewrite = if discard_unknown_contents {
                clear_history_storage(&history_path)
                    .and_then(|_| save_history(&history_path, &loaded_history.history))
            } else {
                save_history(&history_path, &loaded_history.history)
            };
            if rewrite.is_err() {
                history_status = HistoryStorageStatus::Unavailable;
            }
        }
        Self {
            usage_client,
            snapshot: AsyncMutex::new(DashboardSnapshot::default()),
            refresh_guard: AsyncMutex::new(()),
            refresh_generation: AtomicU64::new(0),
            schedule_guard: AsyncMutex::new(()),
            stored_settings: StdMutex::new(stored_settings),
            settings_path,
            history_path,
            usage_history: AsyncMutex::new(HistoryRuntime {
                history: loaded_history.history,
                storage_status: history_status,
            }),
            notification_evaluation_guard: StdMutex::new(()),
            notification_tracker: StdMutex::new(NotificationTracker::new()),
            schedule_sender,
            consecutive_failures: AtomicU32::new(0),
            update_check_sender,
            app_update: AppUpdateState::default(),
            geometry_generations: StdMutex::new(HashMap::new()),
        }
    }

    fn stored_settings(&self) -> std::sync::MutexGuard<'_, StoredSettings> {
        // 发生写入线程异常时仍应让用户继续使用已知设置，而不是因锁中毒退出应用。
        self.stored_settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn notification_evaluation(&self) -> std::sync::MutexGuard<'_, ()> {
        self.notification_evaluation_guard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    async fn current_snapshot(&self) -> DashboardSnapshot {
        self.snapshot.lock().await.clone()
    }

    fn current_settings(&self) -> Settings {
        self.stored_settings().preferences.clone()
    }

    fn notify_schedule_changed(&self) {
        self.schedule_sender.send_modify(|revision| {
            *revision = revision.wrapping_add(1);
        });
    }

    fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures.load(Ordering::Relaxed)
    }

    fn reset_notification_baseline(&self) {
        self.notification_tracker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .reset_baseline();
    }

    fn retry_delay_seconds(&self) -> u64 {
        let failures = self.consecutive_failures();
        if failures == 0 {
            self.current_settings().refresh_interval_seconds
        } else {
            failure_retry_seconds(failures)
        }
    }

    async fn reschedule_from_now_locked(&self) -> DashboardSnapshot {
        let delay = self.retry_delay_seconds();
        let mut snapshot = self.snapshot.lock().await;
        snapshot.next_refresh_at = Some(Utc::now() + ChronoDuration::seconds(delay as i64));
        let result = snapshot.clone();
        drop(snapshot);
        self.notify_schedule_changed();
        result
    }

    async fn clear_snapshot_forecasts(&self) -> DashboardSnapshot {
        let mut snapshot = self.snapshot.lock().await;
        for window in &mut snapshot.quota_windows {
            window.forecast = None;
        }
        snapshot.clone()
    }

    async fn apply_history_to_snapshot(
        &self,
        snapshot: &mut DashboardSnapshot,
        account_identity: &UsageAccountIdentity,
    ) -> HistoryRefreshEffect {
        let sampled_at = snapshot.refreshed_at.unwrap_or_else(Utc::now);
        let inputs = snapshot
            .quota_windows
            .iter()
            .map(|window| HistoryWindowInput {
                window_id: window.id.clone(),
                window_seconds: window.window_seconds,
                cycle_reset_at: window.reset_at,
                remaining_percent: window.remaining_percent,
            })
            .collect::<Vec<_>>();
        let identity = match account_identity {
            UsageAccountIdentity::AccountId(value) => AccountIdentity::AccountId(value),
            UsageAccountIdentity::Token(value) => AccountIdentity::Token(value),
        };
        let mut runtime = self.usage_history.lock().await;
        // 与专属采集开关命令在同一锁序下重读，避免禁用期间的在途刷新继续采样。
        let history_enabled = self.current_settings().history_enabled;
        if !history_enabled {
            let account_changed = match runtime.history.select_account(identity) {
                Ok(AccountSelection::Unchanged) => false,
                Ok(AccountSelection::Initialized | AccountSelection::ChangedAndCleared { .. }) => {
                    true
                }
                Err(_) => {
                    runtime.storage_status = HistoryStorageStatus::Unavailable;
                    false
                }
            };
            if account_changed {
                let saved = clear_history_storage(&self.history_path)
                    .and_then(|_| save_history(&self.history_path, &runtime.history));
                runtime.storage_status = if saved.is_ok() {
                    HistoryStorageStatus::Ready
                } else {
                    HistoryStorageStatus::Unavailable
                };
            }
            for window in &mut snapshot.quota_windows {
                window.forecast = None;
            }
            return HistoryRefreshEffect {
                changed: account_changed,
                account_changed,
            };
        }
        let mutation = match runtime
            .history
            .record_successful_snapshot(identity, sampled_at, &inputs)
        {
            Ok(mutation) => mutation,
            Err(_) => {
                runtime.storage_status = HistoryStorageStatus::Unavailable;
                log::warn!("本地趋势采样未完成：类别=identity-or-salt。");
                return HistoryRefreshEffect::default();
            }
        };

        if mutation.changed() {
            let saved = if mutation.account_changed {
                // 先删除旧账号文件；安全新文件即使写失败，也不会继续保留旧样本。
                clear_history_storage(&self.history_path)
                    .and_then(|_| save_history(&self.history_path, &runtime.history))
            } else {
                save_history(&self.history_path, &runtime.history)
            };
            runtime.storage_status = if saved.is_ok() {
                HistoryStorageStatus::Ready
            } else {
                log::warn!("本地趋势保存失败：类别=storage。");
                HistoryStorageStatus::Unavailable
            };
        }

        for window in &mut snapshot.quota_windows {
            window.forecast = runtime
                .history
                .forecast_for(&window.id, window.window_seconds, sampled_at)
                .map(convert_history_forecast);
        }
        HistoryRefreshEffect {
            changed: mutation.changed(),
            account_changed: mutation.account_changed,
        }
    }

    async fn refresh_dashboard(&self) -> RefreshResult {
        // 定时刷新与手动刷新同时到达时，复用现有快照，避免重复携带凭据发出请求。
        let observed_generation = self.refresh_generation.load(Ordering::Acquire);
        let _guard = match self.refresh_guard.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                let completed = self.refresh_guard.lock().await;
                if refresh_completed_since(
                    observed_generation,
                    self.refresh_generation.load(Ordering::Acquire),
                ) {
                    // 单飞竞争者等待当前刷新结束后复用最终快照，不发第二个请求或重复广播。
                    let snapshot = self.current_snapshot().await;
                    drop(completed);
                    return RefreshResult {
                        snapshot,
                        was_successful: false,
                        should_emit: false,
                        history_effect: HistoryRefreshEffect::default(),
                    };
                }
                // 锁若只由历史开关/清除操作占用，不得吞掉用户或定时刷新。
                completed
            }
        };

        let result = match self.usage_client.fetch_dashboard().await {
            Ok(fetched) => {
                let mut fresh = fetched.snapshot;
                let history_effect = self
                    .apply_history_to_snapshot(&mut fresh, &fetched.account_identity)
                    .await;
                self.consecutive_failures.store(0, Ordering::Relaxed);
                let _schedule_guard = self.schedule_guard.lock().await;
                // 必须在请求完成后、持有排期锁时重读，避免旧请求覆盖新设置。
                let interval = self.current_settings().refresh_interval_seconds;
                fresh.next_refresh_at = Some(Utc::now() + ChronoDuration::seconds(interval as i64));
                log::info!(
                    "用量刷新成功：窗口数={}，下次刷新间隔={}秒",
                    fresh.quota_windows.len(),
                    interval
                );
                let mut snapshot = self.snapshot.lock().await;
                *snapshot = fresh.clone();
                drop(snapshot);
                self.notify_schedule_changed();
                RefreshResult {
                    snapshot: fresh,
                    was_successful: true,
                    should_emit: true,
                    history_effect,
                }
            }
            Err(error) => {
                let failures = self
                    .consecutive_failures
                    .fetch_add(1, Ordering::Relaxed)
                    .saturating_add(1);
                let retry_seconds = failure_retry_seconds(failures);
                let _schedule_guard = self.schedule_guard.lock().await;
                let mut snapshot = self.snapshot.lock().await;
                snapshot.status = if snapshot.quota_windows.is_empty() {
                    DashboardStatus::Error
                } else {
                    DashboardStatus::Stale
                };
                snapshot.message = Some(error.code());
                snapshot.next_refresh_at =
                    Some(Utc::now() + ChronoDuration::seconds(retry_seconds as i64));
                let result = snapshot.clone();
                drop(snapshot);
                self.notify_schedule_changed();
                log::warn!(
                    "用量刷新失败：类别={:?}、连续失败={}、下次重试={}秒",
                    error.code(),
                    failures,
                    retry_seconds
                );
                RefreshResult {
                    snapshot: result,
                    was_successful: false,
                    should_emit: true,
                    history_effect: HistoryRefreshEffect::default(),
                }
            }
        };
        self.refresh_generation.fetch_add(1, Ordering::Release);
        result
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
        let _ = self.update_check_sender.send(result.auto_check_updates);
        log::info!(
            "已保存挂件设置：置顶={}、锁定={}、刷新间隔={}秒、自动检查更新={}",
            result.always_on_top,
            result.lock_position,
            result.refresh_interval_seconds,
            result.auto_check_updates
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

fn convert_history_forecast(forecast: HistoryForecast) -> QuotaForecast {
    QuotaForecast {
        status: match forecast.status {
            HistoryForecastStatus::Collecting => ModelForecastStatus::Collecting,
            HistoryForecastStatus::Stable => ModelForecastStatus::Stable,
            HistoryForecastStatus::ExhaustsBeforeReset => ModelForecastStatus::ExhaustsBeforeReset,
            HistoryForecastStatus::LastsUntilReset => ModelForecastStatus::LastsUntilReset,
        },
        exhausts_at: forecast.exhausts_at,
        sample_count: forecast.sample_count as usize,
        observed_span_seconds: forecast.observed_span_seconds,
        consumed_percent: f64::from(forecast.consumed_percent),
    }
}

fn resolved_language(preference: Language) -> Language {
    match preference {
        Language::System => {
            if sys_locale::get_locale()
                .is_some_and(|locale| locale.to_ascii_lowercase().starts_with("zh"))
            {
                Language::ZhCn
            } else {
                Language::En
            }
        }
        selected => selected,
    }
}

fn settings_window_title(language: Language) -> &'static str {
    match resolved_language(language) {
        Language::ZhCn => "CodexUsageBar 设置",
        Language::En | Language::System => "CodexUsageBar Settings",
    }
}

fn local_time_minutes(value: &str) -> u16 {
    value
        .split_once(':')
        .and_then(|(hour, minute)| Some((hour.parse::<u16>().ok()?, minute.parse::<u16>().ok()?)))
        .filter(|(hour, minute)| *hour < 24 && *minute < 60)
        .map(|(hour, minute)| hour * 60 + minute)
        .unwrap_or(0)
}

fn notification_policy(settings: &Settings) -> NotificationPolicy {
    let notifications = &settings.notifications;
    NotificationPolicy {
        enabled: notifications.enabled,
        low_remaining_enabled: notifications.low_quota_enabled,
        pace_deficit_enabled: notifications.pace_enabled,
        reset_enabled: notifications.reset_enabled,
        low_remaining_threshold: notifications.low_quota_threshold_percent,
        pace_deficit_threshold: notifications.pace_deficit_threshold_percent,
        quiet_hours: QuietHours {
            enabled: notifications.quiet_hours_enabled,
            start_minute: local_time_minutes(&notifications.quiet_hours_start),
            end_minute: local_time_minutes(&notifications.quiet_hours_end),
        },
    }
}

fn localized_quota_label(window: &crate::models::QuotaWindow, language: Language) -> String {
    match (resolved_language(language), window.fallback_label) {
        (Language::ZhCn, QuotaFallbackLabel::FiveHour) => "5 小时限额".to_owned(),
        (Language::ZhCn, QuotaFallbackLabel::Weekly) => "周限额".to_owned(),
        (Language::ZhCn, QuotaFallbackLabel::Window) => "额度窗口".to_owned(),
        (_, QuotaFallbackLabel::FiveHour) => "5-hour limit".to_owned(),
        (_, QuotaFallbackLabel::Weekly) => "Weekly limit".to_owned(),
        (_, QuotaFallbackLabel::Window) => "Quota window".to_owned(),
    }
}

fn notification_body(
    batch: &NotificationBatch,
    snapshot: &DashboardSnapshot,
    language: Language,
) -> String {
    let language = resolved_language(language);
    let mut lines = Vec::new();
    for item in &batch.items {
        let Some(window) = snapshot.quota_windows.get(item.window_index) else {
            continue;
        };
        let label = localized_quota_label(window, language);
        let reasons = item
            .reasons
            .iter()
            .map(|reason| match (language, reason) {
                (Language::ZhCn, NotificationReason::LowRemaining { remaining_percent }) => {
                    format!("剩余 {remaining_percent}%")
                }
                (Language::ZhCn, NotificationReason::PaceDeficit { deficit_points }) => {
                    format!("消耗进度快 {deficit_points} 个百分点")
                }
                (Language::ZhCn, NotificationReason::Reset) => "额度已重置".to_owned(),
                (_, NotificationReason::LowRemaining { remaining_percent }) => {
                    format!("{remaining_percent}% remaining")
                }
                (_, NotificationReason::PaceDeficit { deficit_points }) => {
                    format!("{deficit_points} points ahead of pace")
                }
                (_, NotificationReason::Reset) => "quota reset".to_owned(),
            })
            .collect::<Vec<_>>()
            .join(if language == Language::ZhCn {
                "、"
            } else {
                ", "
            });
        lines.push(format!("{label}: {reasons}"));
    }
    lines.join("\n")
}

fn send_usage_notifications(
    app: &AppHandle,
    state: &AppState,
    snapshot: &DashboardSnapshot,
    account_changed: bool,
) {
    // 与设置发布/baseline 重建互斥，避免用新阈值评估旧 tracker 后补发历史事件。
    let _notification_evaluation = state.notification_evaluation();
    let settings = state.current_settings();
    if account_changed {
        state.reset_notification_baseline();
    }
    let policy = notification_policy(&settings);
    let now_local = Local::now();
    let windows = snapshot
        .quota_windows
        .iter()
        .map(|window| NotificationWindow {
            key: window.id.clone(),
            remaining_percent: window.remaining_percent,
            reset_at_unix_seconds: window.reset_at.map(|value| value.timestamp()),
            start_at_unix_seconds: window.start_at.map(|value| value.timestamp()),
            window_seconds: window.window_seconds,
            is_long_period: window.show_pace_marker,
        })
        .collect::<Vec<_>>();
    let observed_at = snapshot.refreshed_at.unwrap_or_else(Utc::now);
    let batch = state
        .notification_tracker
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .evaluate(
            &NotificationSnapshot {
                observed_at_unix_seconds: observed_at.timestamp(),
                local_minute_of_day: now_local.hour() as u16 * 60 + now_local.minute() as u16,
                windows: &windows,
            },
            &policy,
            |event_at| {
                DateTime::<Utc>::from_timestamp(event_at, 0)
                    .map(|value| value.with_timezone(&Local))
                    .is_some_and(|local| {
                        policy
                            .quiet_hours
                            .contains(local.hour() as u16 * 60 + local.minute() as u16)
                    })
            },
        );
    let Some(batch) = batch else {
        return;
    };
    let language = resolved_language(settings.language);
    let title = if language == Language::ZhCn {
        "Codex 额度提醒"
    } else {
        "Codex quota alert"
    };
    let body = notification_body(&batch, snapshot, language);
    if body.is_empty() {
        return;
    }
    if app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .is_err()
    {
        log::warn!("系统通知发送失败：类别=platform。");
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum HistoryResponseStorageStatus {
    Ready,
    Empty,
    Recovered,
    Unavailable,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageHistorySeriesResponse {
    window_id: String,
    window_seconds: i64,
    fallback_label: QuotaFallbackLabel,
    current_remaining_percent: Option<u8>,
    consumed_percent: f64,
    points: Vec<crate::usage_history::UsageHistoryPoint>,
    forecast: QuotaForecast,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageHistoryResponse {
    range: UsageHistoryRange,
    history_enabled: bool,
    storage_status: HistoryResponseStorageStatus,
    sample_count: usize,
    earliest_sample_at: Option<DateTime<Utc>>,
    latest_sample_at: Option<DateTime<Utc>>,
    series: Vec<UsageHistorySeriesResponse>,
}

fn fallback_label_for_duration(window_seconds: i64) -> QuotaFallbackLabel {
    if window_seconds <= 12 * 60 * 60 {
        QuotaFallbackLabel::FiveHour
    } else if (6 * 24 * 60 * 60..=8 * 24 * 60 * 60).contains(&window_seconds) {
        QuotaFallbackLabel::Weekly
    } else {
        QuotaFallbackLabel::Window
    }
}

fn normalize_general_settings(requested: Settings, current: &Settings) -> Settings {
    let mut normalized = requested.normalized();
    // 两个副作用开关必须走各自带权限/并发屏障的专属命令。
    normalized.notifications.enabled = current.notifications.enabled;
    normalized.history_enabled = current.history_enabled;
    normalized
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
    let result = state.refresh_dashboard().await;
    if result.should_emit && result.was_successful {
        send_usage_notifications(
            &app,
            &state,
            &result.snapshot,
            result.history_effect.account_changed,
        );
    }
    if result.should_emit && result.history_effect.changed {
        emit_usage_history_updated(&app);
    }
    let snapshot = result.snapshot;
    if result.should_emit {
        state.apply_auto_main_height(&app, &snapshot);
        emit_dashboard(&app, snapshot.clone());
    }
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
    let previous_settings = state.current_settings();
    let next_settings = normalize_general_settings(settings, &previous_settings);
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
    let should_reset_notification_baseline = next_settings.language != previous_settings.language
        || next_settings.notifications != previous_settings.notifications;
    // 设置发布与 deadline 替换共用一把排期锁：旧 sleep 要么先开始刷新，
    // 要么看到新的修订号，不能在两者之间使用已取消的截止时间。
    let schedule_guard = state.schedule_guard.lock().await;
    let stored = {
        let _notification_evaluation = state.notification_evaluation();
        let saved = match state.save_preferences(next_settings) {
            Ok(saved) => saved,
            Err(error) => {
                let _ = main_window.set_always_on_top(previous_settings.always_on_top);
                let _ = main_window.set_resizable(!previous_settings.lock_position);
                log::warn!("挂件设置持久化失败，已尝试恢复主窗口状态。");
                return Err(error);
            }
        };
        if should_reset_notification_baseline {
            state.reset_notification_baseline();
        }
        saved
    };
    let snapshot = state.reschedule_from_now_locked().await;
    drop(schedule_guard);
    if app.emit("settings-updated", stored.clone()).is_err() {
        log::warn!("无法向界面广播已保存设置。");
    }
    if let Some(settings_window) = app.get_webview_window(SETTINGS_WINDOW_LABEL) {
        let _ = settings_window.set_title(settings_window_title(stored.language));
    }
    emit_dashboard(&app, snapshot);
    Ok(stored)
}

fn map_notification_permission(permission: PermissionState) -> NotificationPermission {
    match permission {
        PermissionState::Granted => NotificationPermission::Granted,
        PermissionState::Denied => NotificationPermission::Denied,
        PermissionState::Prompt | PermissionState::PromptWithRationale => {
            NotificationPermission::Prompt
        }
    }
}

#[tauri::command]
async fn set_notifications_enabled(
    enabled: bool,
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<NotificationEnableResult, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let permission = if enabled {
        let current = app
            .notification()
            .permission_state()
            .map_err(|_| "notificationUnavailable".to_owned())?;
        let resolved = match current {
            PermissionState::Prompt | PermissionState::PromptWithRationale => app
                .notification()
                .request_permission()
                .map_err(|_| "notificationUnavailable".to_owned())?,
            state => state,
        };
        map_notification_permission(resolved)
    } else {
        app.notification()
            .permission_state()
            .map(map_notification_permission)
            .unwrap_or(NotificationPermission::Unavailable)
    };
    let actual_enabled = enabled && permission == NotificationPermission::Granted;
    let schedule_guard = state.schedule_guard.lock().await;
    let saved = {
        let _notification_evaluation = state.notification_evaluation();
        let mut settings = state.current_settings();
        settings.notifications.enabled = actual_enabled;
        let saved = state.save_preferences(settings)?;
        state.reset_notification_baseline();
        saved
    };
    let snapshot = state.reschedule_from_now_locked().await;
    drop(schedule_guard);
    if app.emit("settings-updated", saved.clone()).is_err() {
        log::warn!("无法广播通知设置变更。");
    }
    emit_dashboard(&app, snapshot);
    Ok(NotificationEnableResult {
        enabled: actual_enabled,
        permission,
        settings: saved,
    })
}

#[tauri::command]
fn send_test_notification(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let settings = state.current_settings();
    if !settings.notifications.enabled {
        return Err("notificationDisabled".to_owned());
    }
    if app
        .notification()
        .permission_state()
        .map_err(|_| "notificationUnavailable".to_owned())?
        != PermissionState::Granted
    {
        return Err("notificationPermissionDenied".to_owned());
    }
    let language = resolved_language(settings.language);
    let (title, body) = if language == Language::ZhCn {
        ("CodexUsageBar 通知测试", "通知已成功提交给系统。")
    } else {
        (
            "CodexUsageBar notification test",
            "The notification was submitted to the system.",
        )
    };
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|_| "notificationUnavailable".to_owned())
}

#[tauri::command]
async fn get_usage_history(
    range: UsageHistoryRange,
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<UsageHistoryResponse, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let runtime = state.usage_history.lock().await;
    let now = Utc::now();
    let storage_summary = runtime.history.summary();
    let range_summary = runtime.history.summary_for_range(range, now);
    let query = runtime.history.query(range, now);
    let storage_status = match runtime.storage_status {
        HistoryStorageStatus::Ready if storage_summary.sample_count > 0 => {
            HistoryResponseStorageStatus::Ready
        }
        HistoryStorageStatus::Missing | HistoryStorageStatus::Ready => {
            HistoryResponseStorageStatus::Empty
        }
        HistoryStorageStatus::RecoveredCorrupt | HistoryStorageStatus::RecoveredUnsupported => {
            HistoryResponseStorageStatus::Recovered
        }
        HistoryStorageStatus::Unavailable => HistoryResponseStorageStatus::Unavailable,
    };
    let series = query
        .series
        .into_iter()
        .map(|series| {
            let forecast = convert_history_forecast(series.forecast);
            UsageHistorySeriesResponse {
                window_id: series.window_id,
                window_seconds: series.window_seconds,
                fallback_label: fallback_label_for_duration(series.window_seconds),
                current_remaining_percent: Some(series.current_remaining_percent),
                consumed_percent: forecast.consumed_percent,
                points: series.points,
                forecast,
            }
        })
        .collect();
    Ok(UsageHistoryResponse {
        range,
        history_enabled: state.current_settings().history_enabled,
        storage_status,
        sample_count: range_summary.sample_count as usize,
        earliest_sample_at: range_summary.oldest_sample_at,
        latest_sample_at: range_summary.latest_sample_at,
        series,
    })
}

#[tauri::command]
async fn set_history_enabled(
    enabled: bool,
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Settings, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    // 先等待在途刷新，再同时占用刷新与排期锁。这样命令返回后既不会回写旧采集状态，
    // 也不会让已取消的 deadline 在配置发布和重新排期之间启动。
    let refresh_barrier = state.refresh_guard.lock().await;
    let schedule_guard = state.schedule_guard.lock().await;
    let mut settings = state.current_settings();
    settings.history_enabled = enabled;
    let saved = state.save_preferences(settings)?;
    if !enabled {
        state.clear_snapshot_forecasts().await;
    }
    let snapshot = state.reschedule_from_now_locked().await;
    drop(schedule_guard);
    drop(refresh_barrier);
    if app.emit("settings-updated", saved.clone()).is_err() {
        log::warn!("无法广播本地趋势采集设置。");
    }
    emit_dashboard(&app, snapshot);
    emit_usage_history_updated(&app);
    Ok(saved)
}

#[tauri::command]
async fn clear_usage_history(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    // 与刷新严格使用 refresh_guard -> usage_history 的统一锁序，防止固定临时文件争用或旧数据回写。
    let _refresh_barrier = state.refresh_guard.lock().await;
    let replacement = UsageHistory::new_random();
    {
        let mut runtime = state.usage_history.lock().await;
        let saved = clear_history_storage(&state.history_path)
            .and_then(|_| save_history(&state.history_path, &replacement));
        runtime.history = replacement;
        runtime.storage_status = if saved.is_ok() {
            HistoryStorageStatus::Ready
        } else {
            HistoryStorageStatus::Unavailable
        };
        saved.map_err(|_| "historyStorageUnavailable".to_owned())?;
    }
    let snapshot = state.clear_snapshot_forecasts().await;
    emit_dashboard(&app, snapshot);
    emit_usage_history_updated(&app);
    log::info!("用户已清除本地趋势历史。");
    Ok(())
}

fn language_name(language: Language) -> &'static str {
    match language {
        Language::System => "system",
        Language::ZhCn => "zh-CN",
        Language::En => "en",
    }
}

fn theme_name(theme: &crate::models::Theme) -> &'static str {
    match theme {
        crate::models::Theme::System => "system",
        crate::models::Theme::Light => "light",
        crate::models::Theme::Dark => "dark",
    }
}

fn notification_permission_name(permission: &NotificationPermission) -> &'static str {
    match permission {
        NotificationPermission::Granted => "granted",
        NotificationPermission::Denied => "denied",
        NotificationPermission::Prompt => "prompt",
        NotificationPermission::Unavailable => "unavailable",
    }
}

fn dashboard_status_name(status: &DashboardStatus) -> &'static str {
    match status {
        DashboardStatus::Loading => "loading",
        DashboardStatus::Ready => "ready",
        DashboardStatus::Stale => "stale",
        DashboardStatus::Error => "error",
    }
}

fn dashboard_error_code_name(code: Option<crate::models::DashboardErrorCode>) -> &'static str {
    match code {
        Some(crate::models::DashboardErrorCode::AuthMissing) => "authMissing",
        Some(crate::models::DashboardErrorCode::AuthInvalid) => "authInvalid",
        Some(crate::models::DashboardErrorCode::Network) => "network",
        Some(crate::models::DashboardErrorCode::RateLimited) => "rateLimited",
        Some(crate::models::DashboardErrorCode::ServiceUnavailable) => "serviceUnavailable",
        Some(crate::models::DashboardErrorCode::InvalidResponse) => "invalidResponse",
        Some(crate::models::DashboardErrorCode::LocalBridge) => "localBridge",
        None => "none",
    }
}

fn history_storage_status_name(status: HistoryStorageStatus) -> &'static str {
    match status {
        HistoryStorageStatus::Missing => "missing",
        HistoryStorageStatus::Ready => "ready",
        HistoryStorageStatus::RecoveredCorrupt => "recoveredCorrupt",
        HistoryStorageStatus::RecoveredUnsupported => "recoveredUnsupported",
        HistoryStorageStatus::Unavailable => "unavailable",
    }
}

fn diagnostic_timestamp(value: Option<DateTime<Utc>>) -> String {
    value
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| "none".to_owned())
}

struct DiagnosticsSummary<'a> {
    app_version: &'a str,
    platform: &'a str,
    architecture: &'a str,
    language_preference: Language,
    resolved_language: Language,
    theme: &'a crate::models::Theme,
    refresh_interval_seconds: u64,
    consecutive_failures: u32,
    notifications_enabled: bool,
    notification_permission: &'a NotificationPermission,
    dashboard_status: &'a DashboardStatus,
    dashboard_error_code: Option<crate::models::DashboardErrorCode>,
    window_count: usize,
    last_refresh_at: Option<DateTime<Utc>>,
    update_status: &'a str,
    update_last_checked_at: Option<DateTime<Utc>>,
    history_enabled: bool,
    history_sample_count: u32,
    history_storage_status: HistoryStorageStatus,
}

/// 诊断输出只接受固定白名单摘要，类型中没有账号、Token、路径、URL、百分比、
/// reset 时间、历史点或原始错误/日志，因此这些数据无法被意外格式化进报告。
fn build_diagnostics(summary: DiagnosticsSummary<'_>) -> String {
    let mut report = String::with_capacity(768);
    let _ = writeln!(report, "CodexUsageBar diagnostics schema: 1");
    let _ = writeln!(report, "appVersion: {}", summary.app_version);
    let _ = writeln!(report, "platform: {}", summary.platform);
    let _ = writeln!(report, "architecture: {}", summary.architecture);
    let _ = writeln!(
        report,
        "languagePreference: {}",
        language_name(summary.language_preference)
    );
    let _ = writeln!(
        report,
        "resolvedLanguage: {}",
        language_name(summary.resolved_language)
    );
    let _ = writeln!(report, "theme: {}", theme_name(summary.theme));
    let _ = writeln!(
        report,
        "refreshIntervalSeconds: {}",
        summary.refresh_interval_seconds
    );
    let _ = writeln!(
        report,
        "consecutiveFailures: {}",
        summary.consecutive_failures
    );
    let _ = writeln!(
        report,
        "notificationsEnabled: {}",
        summary.notifications_enabled
    );
    let _ = writeln!(
        report,
        "notificationPermission: {}",
        notification_permission_name(summary.notification_permission)
    );
    let _ = writeln!(
        report,
        "dashboardStatus: {}",
        dashboard_status_name(summary.dashboard_status)
    );
    let _ = writeln!(
        report,
        "dashboardErrorCode: {}",
        dashboard_error_code_name(summary.dashboard_error_code)
    );
    let _ = writeln!(report, "windowCount: {}", summary.window_count);
    let _ = writeln!(
        report,
        "lastRefreshAt: {}",
        diagnostic_timestamp(summary.last_refresh_at)
    );
    let _ = writeln!(report, "updateStatus: {}", summary.update_status);
    let _ = writeln!(
        report,
        "updateLastCheckedAt: {}",
        diagnostic_timestamp(summary.update_last_checked_at)
    );
    let _ = writeln!(report, "historyEnabled: {}", summary.history_enabled);
    let _ = writeln!(
        report,
        "historySampleCount: {}",
        summary.history_sample_count
    );
    let _ = writeln!(
        report,
        "historyStorageStatus: {}",
        history_storage_status_name(summary.history_storage_status)
    );
    report
}

#[tauri::command]
async fn get_diagnostics(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    let settings = state.current_settings();
    let snapshot = state.current_snapshot().await;
    let update = state.app_update.current_info().await;
    let runtime = state.usage_history.lock().await;
    let history = runtime.history.summary();
    let permission = app
        .notification()
        .permission_state()
        .map(map_notification_permission)
        .unwrap_or(NotificationPermission::Unavailable);
    let update_status = if update.checked_at.is_none() {
        "notChecked"
    } else if update.update_available {
        "available"
    } else {
        "current"
    };

    Ok(build_diagnostics(DiagnosticsSummary {
        app_version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        language_preference: settings.language,
        resolved_language: resolved_language(settings.language),
        theme: &settings.theme,
        refresh_interval_seconds: settings.refresh_interval_seconds,
        consecutive_failures: state.consecutive_failures(),
        notifications_enabled: settings.notifications.enabled,
        notification_permission: &permission,
        dashboard_status: &snapshot.status,
        dashboard_error_code: snapshot.message,
        window_count: app.webview_windows().len(),
        last_refresh_at: snapshot.refreshed_at,
        update_status,
        update_last_checked_at: update.checked_at,
        history_enabled: settings.history_enabled,
        history_sample_count: history.sample_count,
        history_storage_status: runtime.storage_status,
    }))
}

#[tauri::command]
fn open_settings_window(
    section: Option<String>,
    window: WebviewWindow,
    app: AppHandle,
) -> Result<(), String> {
    require_window_label(&window, MAIN_WINDOW_LABEL)?;
    if section.as_deref().is_some_and(|value| value != "about") {
        return Err("无法打开指定的设置页面。".to_owned());
    }
    let window = app
        .get_webview_window(SETTINGS_WINDOW_LABEL)
        .ok_or_else(|| "无法创建设置窗口。".to_owned())?;
    window.show().map_err(|_| "无法显示设置窗口。".to_owned())?;
    window
        .set_focus()
        .map_err(|_| "无法聚焦设置窗口。".to_owned())?;
    if section.as_deref() == Some("about")
        && app
            .emit_to(SETTINGS_WINDOW_LABEL, "settings-navigate", "about")
            .is_err()
    {
        log::warn!("无法导航到更新设置页面。");
    }
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
async fn get_app_update_info(
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<AppUpdateInfo, String> {
    // 主卡只读取安全摘要以显示发现新版本的提醒；下载和安装始终限定在设置窗口。
    require_known_window(&window)?;
    Ok(state.app_update.current_info().await)
}

#[tauri::command]
async fn check_app_update(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<AppUpdateInfo, String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    check_for_update(&app, &state.app_update, false).await
}

#[tauri::command]
async fn install_app_update(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    require_window_label(&window, SETTINGS_WINDOW_LABEL)?;
    install_pending_update(&app, &state.app_update).await
}

fn start_refresh_loop(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let mut schedule_receiver = state.schedule_sender.subscribe();
        refresh_and_emit(&app, &state).await;
        loop {
            let revision = *schedule_receiver.borrow_and_update();
            let deadline = state.current_snapshot().await.next_refresh_at;
            let wait = deadline
                .as_ref()
                .and_then(|deadline| (*deadline - Utc::now()).to_std().ok())
                .unwrap_or(Duration::ZERO);
            tokio::select! {
                biased;
                changed = schedule_receiver.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
                _ = tokio::time::sleep(wait) => {
                    // 与 deadline 写入使用同一把锁。设置变更若先完成，这里会看到
                    // 新修订号并丢弃旧 sleep；若这里先验证，则本次刷新已正式开始。
                    let schedule_guard = state.schedule_guard.lock().await;
                    let current_revision = *schedule_receiver.borrow();
                    let current_deadline = state.current_snapshot().await.next_refresh_at;
                    let should_refresh = scheduled_deadline_is_current(
                        revision,
                        current_revision,
                        deadline,
                        current_deadline,
                    );
                    drop(schedule_guard);
                    if should_refresh {
                        refresh_and_emit(&app, &state).await;
                    }
                }
            }
        }
    });
}

fn start_update_check_loop(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        // 不与主卡首屏初始化抢资源；启动后的短延迟可避免网络尚未恢复时产生无意义失败。
        tokio::time::sleep(AUTO_UPDATE_CHECK_START_DELAY).await;
        let mut enabled_receiver = state.update_check_sender.subscribe();
        loop {
            if *enabled_receiver.borrow_and_update() {
                if let Err(message) = check_for_update(&app, &state.app_update, true).await {
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

async fn refresh_and_emit(app: &AppHandle, state: &AppState) {
    let result = state.refresh_dashboard().await;
    if result.should_emit && result.was_successful {
        send_usage_notifications(
            app,
            state,
            &result.snapshot,
            result.history_effect.account_changed,
        );
    }
    if result.should_emit && result.history_effect.changed {
        emit_usage_history_updated(app);
    }
    let snapshot = result.snapshot;
    if result.should_emit {
        state.apply_auto_main_height(app, &snapshot);
        emit_dashboard(app, snapshot);
    }
}

fn emit_usage_history_updated(app: &AppHandle) {
    if app
        .emit_to(SETTINGS_WINDOW_LABEL, "usage-history-updated", ())
        .is_err()
    {
        log::warn!("无法向设置窗口广播本地趋势更新。");
    }
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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
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
                // Updater 依赖可能记录远端 URL 或响应解析细节；本应用已自行写入脱敏的结果类别，
                // 因此不把该依赖的原始日志落盘，保证日志边界不随第三方实现变化。
                .filter(|metadata| metadata.target().starts_with("codex_usage_bar"))
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
            let _ = settings_window
                .set_title(settings_window_title(stored_settings.preferences.language));
            restore_main_window(&main_window, &stored_settings, &state);
            restore_settings_window(&settings_window, &stored_settings);
            // 静态窗口在配置中默认隐藏；显式隐藏保证升级时不会与主卡同时出现。
            let _ = settings_window.hide();
            main_window.show()?;
            start_refresh_loop(app.handle().clone(), state.clone());
            start_update_check_loop(app.handle().clone(), state);
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
            set_notifications_enabled,
            send_test_notification,
            get_usage_history,
            set_history_enabled,
            clear_usage_history,
            get_diagnostics,
            open_settings_window,
            mark_main_size_manual,
            get_autostart,
            set_autostart,
            get_app_update_info,
            check_app_update,
            install_app_update
        ])
        .run(tauri::generate_context!())
        .expect("启动 CodexUsageBar 失败");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::{DashboardErrorCode, Language, NotificationSettings, Theme},
        usage::UsageClient,
    };

    #[test]
    fn only_supported_refresh_intervals_are_accepted() {
        let result = Settings {
            always_on_top: true,
            lock_position: false,
            refresh_interval_seconds: 99,
            theme: Theme::Dark,
            auto_check_updates: false,
            ..Settings::default()
        }
        .normalized();
        assert_eq!(result.refresh_interval_seconds, 60);
        assert!(result.always_on_top);
        assert!(!result.auto_check_updates);
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

    #[test]
    fn general_settings_cannot_bypass_side_effect_commands() {
        let current = Settings {
            notifications: NotificationSettings {
                enabled: false,
                ..NotificationSettings::default()
            },
            history_enabled: true,
            ..Settings::default()
        };
        let requested = Settings {
            notifications: NotificationSettings {
                enabled: true,
                low_quota_threshold_percent: 42,
                ..NotificationSettings::default()
            },
            history_enabled: false,
            language: Language::En,
            ..Settings::default()
        };

        let normalized = normalize_general_settings(requested, &current);
        assert!(!normalized.notifications.enabled);
        assert!(normalized.history_enabled);
        assert_eq!(normalized.notifications.low_quota_threshold_percent, 42);
        assert_eq!(normalized.language, Language::En);
    }

    #[test]
    fn stable_dashboard_errors_serialize_without_rust_copy() {
        assert_eq!(
            serde_json::to_string(&DashboardErrorCode::AuthMissing).unwrap(),
            r#""authMissing""#
        );
        assert_eq!(
            serde_json::to_string(&DashboardErrorCode::InvalidResponse).unwrap(),
            r#""invalidResponse""#
        );
    }

    #[test]
    fn macos_transparent_main_window_enables_private_api() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let main_window = config["app"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|window| window["label"] == MAIN_WINDOW_LABEL)
            .unwrap();

        assert_eq!(main_window["transparent"], true);
        assert_eq!(main_window["acceptFirstMouse"], true);
        assert_eq!(config["app"]["macOSPrivateApi"], true);
    }

    #[test]
    fn settings_titlebar_can_follow_the_webview_theme() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let settings_window = config["app"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|window| window["label"] == SETTINGS_WINDOW_LABEL)
            .unwrap();
        assert_eq!(settings_window["titleBarStyle"], "Transparent");

        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/settings.json")).unwrap();
        let permissions = capability["permissions"].as_array().unwrap();
        assert!(permissions
            .iter()
            .any(|permission| permission == "core:window:allow-set-theme"));
        assert!(permissions
            .iter()
            .any(|permission| { permission == "core:window:allow-set-background-color" }));
        assert!(permissions
            .iter()
            .any(|permission| permission == "clipboard-manager:allow-write-text"));
        assert!(!permissions
            .iter()
            .any(|permission| permission == "clipboard-manager:allow-read-text"));
        let opener = permissions
            .iter()
            .find(|permission| permission["identifier"] == "opener:allow-open-url")
            .unwrap();
        assert_eq!(
            opener["allow"],
            serde_json::json!([
                {"url": "https://github.com/creamtea47/codex-usage-bar/releases"},
                {"url": "https://github.com/creamtea47/codex-usage-bar/releases/**"},
                {"url": "https://github.com/creamtea47/codex-usage-bar/issues/new"},
                {"url": "https://github.com/creamtea47/codex-usage-bar/issues/new/**"}
            ])
        );

        let main_capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        let main_permissions = main_capability["permissions"].as_array().unwrap();
        assert!(!main_permissions.iter().any(|permission| {
            permission.as_str().is_some_and(|value| {
                value.starts_with("notification:")
                    || value.starts_with("clipboard-manager:")
                    || value.starts_with("opener:")
            }) || permission["identifier"]
                .as_str()
                .is_some_and(|value| value.starts_with("opener:"))
        }));
    }

    #[test]
    fn diagnostics_are_limited_to_the_fixed_safe_field_set() {
        let permission = NotificationPermission::Denied;
        let status = DashboardStatus::Stale;
        let theme = Theme::Dark;
        let report = build_diagnostics(DiagnosticsSummary {
            app_version: "0.3.0",
            platform: "test-os",
            architecture: "test-arch",
            language_preference: Language::ZhCn,
            resolved_language: Language::ZhCn,
            theme: &theme,
            refresh_interval_seconds: 300,
            consecutive_failures: 2,
            notifications_enabled: false,
            notification_permission: &permission,
            dashboard_status: &status,
            dashboard_error_code: Some(DashboardErrorCode::Network),
            window_count: 2,
            last_refresh_at: None,
            update_status: "current",
            update_last_checked_at: None,
            history_enabled: true,
            history_sample_count: 12,
            history_storage_status: HistoryStorageStatus::Ready,
        });
        let fields = report
            .lines()
            .map(|line| line.split_once(':').unwrap().0)
            .collect::<Vec<_>>();
        assert_eq!(
            fields,
            vec![
                "CodexUsageBar diagnostics schema",
                "appVersion",
                "platform",
                "architecture",
                "languagePreference",
                "resolvedLanguage",
                "theme",
                "refreshIntervalSeconds",
                "consecutiveFailures",
                "notificationsEnabled",
                "notificationPermission",
                "dashboardStatus",
                "dashboardErrorCode",
                "windowCount",
                "lastRefreshAt",
                "updateStatus",
                "updateLastCheckedAt",
                "historyEnabled",
                "historySampleCount",
                "historyStorageStatus",
            ]
        );
        let normalized = report.to_ascii_lowercase();
        for forbidden in [
            "accesstoken",
            "accountemail",
            "authpath",
            "proxy",
            "https://",
            "remainingpercent",
            "resetat",
            "historypoints",
            "rawerror",
            "rawlog",
        ] {
            assert!(
                !normalized.contains(forbidden),
                "found {forbidden} in {report}"
            );
        }
    }

    #[test]
    fn notification_body_ignores_account_and_upstream_window_labels() {
        let snapshot = DashboardSnapshot {
            status: DashboardStatus::Ready,
            account_email_masked: Some("s***@private.example".to_owned()),
            plan_label: Some("Sensitive Plan".to_owned()),
            refreshed_at: None,
            next_refresh_at: None,
            message: None,
            quota_windows: vec![crate::models::QuotaWindow {
                id: "internal-stream".to_owned(),
                label: Some("Sensitive upstream label".to_owned()),
                fallback_label: QuotaFallbackLabel::Weekly,
                remaining_percent: 20,
                used_percent: 80,
                window_seconds: 7 * 24 * 60 * 60,
                reset_at: None,
                reset_after_seconds: 60,
                start_at: None,
                show_pace_marker: true,
                forecast: None,
            }],
        };
        let body = notification_body(
            &NotificationBatch {
                items: vec![crate::notification_rules::NotificationItem {
                    window_index: 0,
                    reasons: vec![NotificationReason::LowRemaining {
                        remaining_percent: 20,
                    }],
                }],
                omitted_window_count: 0,
            },
            &snapshot,
            Language::En,
        );

        assert_eq!(body, "Weekly limit: 20% remaining");
        assert!(!body.contains("private.example"));
        assert!(!body.contains("Sensitive"));
        assert!(!body.contains("internal-stream"));
    }

    #[test]
    fn settings_native_title_follows_language_resolution() {
        assert_eq!(settings_window_title(Language::ZhCn), "CodexUsageBar 设置");
        assert_eq!(
            settings_window_title(Language::En),
            "CodexUsageBar Settings"
        );
        assert_eq!(
            map_notification_permission(PermissionState::Denied),
            NotificationPermission::Denied
        );
    }

    #[test]
    fn native_bundle_defaults_are_platform_specific() {
        let base: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let windows: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.windows.conf.json")).unwrap();
        let macos: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.macos.conf.json")).unwrap();
        let unsigned: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.unsigned.conf.json")).unwrap();

        assert!(base["bundle"].get("targets").is_none());
        assert_eq!(windows["bundle"]["targets"], serde_json::json!(["nsis"]));
        assert_eq!(macos["bundle"]["targets"], serde_json::json!(["dmg"]));
        assert_eq!(unsigned["bundle"]["createUpdaterArtifacts"], false);
    }

    #[test]
    fn canceled_schedule_revisions_and_deadlines_are_never_current() {
        let first = Some(Utc::now() + ChronoDuration::minutes(1));
        let replacement = Some(Utc::now() + ChronoDuration::minutes(5));
        assert!(scheduled_deadline_is_current(4, 4, first, first));
        assert!(!scheduled_deadline_is_current(4, 5, first, first));
        assert!(!scheduled_deadline_is_current(4, 4, first, replacement));

        assert!(!refresh_completed_since(9, 9));
        assert!(refresh_completed_since(9, 10));
    }

    #[tokio::test]
    async fn rescheduling_uses_current_fixed_interval_and_failure_backoff() {
        let state = AppState::new(
            UsageClient::new().unwrap(),
            StoredSettings::default(),
            PathBuf::from("unused-schedule-test-settings.json"),
        );
        state.stored_settings().preferences.refresh_interval_seconds = 1_800;
        let schedule_guard = state.schedule_guard.lock().await;
        let before = Utc::now();
        let fixed = state.reschedule_from_now_locked().await;
        let after = Utc::now();
        let fixed_deadline = fixed.next_refresh_at.unwrap();
        assert!(fixed_deadline >= before + ChronoDuration::seconds(1_800));
        assert!(fixed_deadline <= after + ChronoDuration::seconds(1_800));

        state.consecutive_failures.store(2, Ordering::Relaxed);
        let before = Utc::now();
        let retry = state.reschedule_from_now_locked().await;
        let after = Utc::now();
        let retry_deadline = retry.next_refresh_at.unwrap();
        assert!(retry_deadline >= before + ChronoDuration::seconds(180));
        assert!(retry_deadline <= after + ChronoDuration::seconds(180));
        drop(schedule_guard);
    }

    #[tokio::test]
    async fn concurrent_refresh_waits_for_the_single_flight_result_without_rebroadcasting_cache() {
        let state = Arc::new(AppState::new(
            UsageClient::new().unwrap(),
            StoredSettings::default(),
            PathBuf::from("unused-test-settings.json"),
        ));
        let expected = DashboardSnapshot {
            status: DashboardStatus::Ready,
            account_email_masked: Some("p***@example.com".to_owned()),
            plan_label: Some("Pro".to_owned()),
            refreshed_at: None,
            next_refresh_at: None,
            message: None,
            quota_windows: Vec::new(),
        };
        let first_refresh = state.refresh_guard.lock().await;
        let contender = {
            let state = state.clone();
            tokio::spawn(async move { state.refresh_dashboard().await })
        };
        tokio::task::yield_now().await;
        *state.snapshot.lock().await = expected.clone();
        state.refresh_generation.fetch_add(1, Ordering::Release);
        drop(first_refresh);

        let result = contender.await.unwrap();
        assert_eq!(result.snapshot, expected);
        assert!(!result.should_emit);
        assert!(!result.was_successful);
    }
}
