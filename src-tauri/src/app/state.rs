use crate::app::continuation::QuotaAutoContinueErrorCode;
use crate::app::continuation::QuotaAutoContinueRuntime;
use crate::app::continuation::STATE_FILE_NAME as QUOTA_AUTO_CONTINUE_STATE_FILE_NAME;
use crate::domain::history::AccountIdentity;
use crate::domain::history::HistoryStorageStatus;
use crate::domain::history::UsageHistory;
use crate::domain::history::USAGE_HISTORY_FILE_NAME;
use crate::domain::notifications::NotificationTracker;
use crate::domain::reset_credit_details;
use crate::domain::settings::Language;
use crate::domain::settings::Settings;
use crate::domain::settings::StoredSettings;
use crate::domain::usage::DashboardSnapshot;
use crate::infra::account_profile;
use crate::infra::accounts;
use crate::infra::app_update::AppUpdateState;
use crate::infra::credit_baseline::ResetCreditNotificationRuntime;
use crate::infra::credit_baseline::STATE_FILE_NAME as RESET_CREDIT_NOTIFICATION_STATE_FILE_NAME;
use crate::infra::history_store::clear_history_storage;
use crate::infra::history_store::load_history;
use crate::infra::history_store::save_history;
use crate::infra::settings::save_settings as persist_settings;
use crate::infra::settings::update_preferences;
use crate::infra::usage::ResetCreditAvailability;
use crate::infra::usage::UsageAccountIdentity;
use crate::infra::usage::UsageClient;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::watch;
use tokio::sync::Mutex as AsyncMutex;

pub(crate) struct SharedState {
    pub(crate) stored_settings: StdMutex<StoredSettings>,
    pub(crate) settings_path: PathBuf,
    pub(crate) history_path: PathBuf,
    pub(crate) usage_history: AsyncMutex<HistoryRuntime>,
    pub(crate) update_check_sender: watch::Sender<bool>,
    pub(crate) app_update: AppUpdateState,
    pub(crate) geometry_generations: StdMutex<HashMap<String, u64>>,
}

pub(crate) struct AppState {
    pub(crate) shared: Arc<SharedState>,
    pub(crate) reset_credit_cache: StdMutex<reset_credit_details::ResetCreditCache>,
    pub(crate) reset_credit_query_guard: AsyncMutex<()>,
    /// 完整账号资料仅驻留内存，通过设置窗口的白名单 IPC 返回，不进入日志或主卡快照。
    pub(crate) account_profile: StdMutex<Option<account_profile::UsageProfile>>,
    pub(crate) account: Option<Arc<accounts::ManagedAccount>>,
    pub(crate) retired: AtomicBool,
    pub(crate) usage_client: UsageClient,
    pub(crate) network_revision: AtomicU64,
    pub(crate) completed_network_revision: AtomicU64,
    pub(crate) snapshot: AsyncMutex<DashboardSnapshot>,
    pub(crate) refresh_guard: AsyncMutex<()>,
    /// 区分“另一个刷新正在执行”和“历史维护暂时占用刷新屏障”。
    /// 等待者只有观察到代次推进时才复用结果，否则会在屏障释放后真正刷新。
    pub(crate) refresh_generation: AtomicU64,
    /// 串行化 deadline 的读取、验证和写入，避免在途刷新覆盖刚保存的刷新间隔。
    pub(crate) schedule_guard: AsyncMutex<()>,
    pub(crate) quota_auto_continue: QuotaAutoContinueRuntime,
    pub(crate) reset_credit_notifications: ResetCreditNotificationRuntime,
    /// 让通知设置发布与 baseline 重建相对通知评估保持原子。
    pub(crate) notification_evaluation_guard: StdMutex<()>,
    pub(crate) notification_tracker: StdMutex<NotificationTracker>,
    /// 值本身只是修订号；每次真实 deadline 变化都会唤醒并取消旧 sleep。
    pub(crate) schedule_sender: watch::Sender<u64>,
    pub(crate) consecutive_failures: AtomicU32,
}

pub(crate) struct HistoryRuntime {
    pub(crate) history: UsageHistory,
    pub(crate) storage_status: HistoryStorageStatus,
}

/// 刷新提交前短暂保留的 Rust 内部观察；账号材料不会进入快照、IPC、日志或状态文件。
pub(crate) struct ResetCreditObservation {
    pub(crate) account_identity: UsageAccountIdentity,
    pub(crate) availability: Option<ResetCreditAvailability>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct HistoryRefreshEffect {
    pub(crate) changed: bool,
    pub(crate) account_changed: bool,
}

pub(crate) struct RefreshResult {
    pub(crate) started: std::time::Instant,
    pub(crate) snapshot: DashboardSnapshot,
    pub(crate) was_successful: bool,
    pub(crate) should_emit: bool,
    pub(crate) history_effect: HistoryRefreshEffect,
    pub(crate) quota_auto_continue_changed: bool,
    pub(crate) reset_credit_observation: Option<ResetCreditObservation>,
}

impl AppState {
    pub(crate) fn new(
        usage_client: UsageClient,
        stored_settings: StoredSettings,
        settings_path: PathBuf,
    ) -> Result<Self, QuotaAutoContinueErrorCode> {
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
        let directory = settings_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf();
        let shared = Arc::new(SharedState {
            stored_settings: StdMutex::new(stored_settings),
            settings_path,
            history_path,
            usage_history: AsyncMutex::new(HistoryRuntime {
                history: loaded_history.history,
                storage_status: history_status,
            }),
            update_check_sender,
            app_update: AppUpdateState::default(),
            geometry_generations: StdMutex::new(HashMap::new()),
        });
        Self::for_account(usage_client, shared, &directory, None)
    }

    pub(crate) fn for_account(
        usage_client: UsageClient,
        shared: Arc<SharedState>,
        directory: &std::path::Path,
        account: Option<Arc<accounts::ManagedAccount>>,
    ) -> Result<Self, QuotaAutoContinueErrorCode> {
        let (schedule_sender, _) = watch::channel(0_u64);
        let mut continuation =
            QuotaAutoContinueRuntime::new(directory.join(QUOTA_AUTO_CONTINUE_STATE_FILE_NAME))?;
        if let Some(account) = account.as_ref() {
            continuation.bind_account(account.clone());
        }
        Ok(Self {
            reset_credit_cache: StdMutex::new(reset_credit_details::ResetCreditCache::default()),
            reset_credit_query_guard: AsyncMutex::new(()),
            account_profile: StdMutex::new(None),
            shared,
            account,
            retired: AtomicBool::new(false),
            usage_client,
            network_revision: AtomicU64::new(0),
            completed_network_revision: AtomicU64::new(0),
            snapshot: AsyncMutex::new(DashboardSnapshot::default()),
            refresh_guard: AsyncMutex::new(()),
            refresh_generation: AtomicU64::new(0),
            schedule_guard: AsyncMutex::new(()),
            quota_auto_continue: continuation,
            reset_credit_notifications: ResetCreditNotificationRuntime::new(
                directory.join(RESET_CREDIT_NOTIFICATION_STATE_FILE_NAME),
            ),
            notification_evaluation_guard: StdMutex::new(()),
            notification_tracker: StdMutex::new(NotificationTracker::new()),
            schedule_sender,
            consecutive_failures: AtomicU32::new(0),
        })
    }

    pub(crate) fn stored_settings(&self) -> std::sync::MutexGuard<'_, StoredSettings> {
        // 发生写入线程异常时仍应让用户继续使用已知设置，而不是因锁中毒退出应用。
        self.shared
            .stored_settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn notification_evaluation(&self) -> std::sync::MutexGuard<'_, ()> {
        self.notification_evaluation_guard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) async fn current_snapshot(&self) -> DashboardSnapshot {
        let mut snapshot = self.snapshot.lock().await.clone();
        snapshot.account_id = self.account.as_ref().map(|a| a.id.clone());
        snapshot
    }

    pub(crate) fn current_settings(&self) -> Settings {
        let mut settings = self.stored_settings().preferences.clone();
        if let Some(account) = &self.account {
            settings.quota_auto_continue_enabled = account
                .config()
                .is_some_and(|a| a.enabled && a.auto_continue);
        }
        settings
    }

    pub(crate) fn select_history_account(
        &self,
        runtime: &mut HistoryRuntime,
    ) -> Result<(), String> {
        if let Some(account) = &self.account {
            let credentials = account.identity()?;
            let identity = credentials
                .account_id
                .as_deref()
                .map(AccountIdentity::AccountId)
                .unwrap_or_else(|| AccountIdentity::Token(&credentials.access_token));
            runtime
                .history
                .select_account(identity)
                .map_err(|_| "accountIdentityMissing")?;
        }
        Ok(())
    }
}
impl AppState {
    pub(crate) fn save_preferences(&self, preferences: Settings) -> Result<Settings, String> {
        let result = {
            let mut stored = self.stored_settings();
            // 先对副本落盘，失败时既不污染内存，也不会把轮询间隔切到未保存的值。
            let mut candidate = stored.clone();
            update_preferences(&mut candidate, preferences);
            persist_settings(&self.shared.settings_path, &candidate)
                .map_err(|_| "无法保存本地设置。".to_owned())?;
            let result = candidate.preferences.clone();
            *stored = candidate;
            result
        };
        let _ = self
            .shared
            .update_check_sender
            .send(result.auto_check_updates);
        log::info!(
            "已保存挂件设置：置顶={}、锁定={}、关闭到托盘={}、刷新间隔={}秒、自动检查更新={}",
            result.always_on_top,
            result.lock_position,
            result.minimize_to_tray_on_close,
            result.refresh_interval_seconds,
            result.auto_check_updates
        );
        Ok(result)
    }
}

pub(crate) const SETTINGS_FILE_NAME: &str = "settings.json";

pub(crate) fn resolved_language(preference: Language) -> Language {
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

pub(crate) fn normalize_general_settings(requested: Settings, current: &Settings) -> Settings {
    let mut normalized = requested.normalized();
    // 三个副作用开关必须走各自带权限/并发屏障的专属命令。
    normalized.notifications.enabled = current.notifications.enabled;
    normalized.history_enabled = current.history_enabled;
    normalized.quota_auto_continue_enabled = current.quota_auto_continue_enabled;
    normalized
}

#[cfg(test)]
mod tests;
