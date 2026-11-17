use crate::app::account_hub::AccountHub;
use crate::app::notifications::quota_window_index;
use crate::app::state::AppState;
use crate::app::state::HistoryRefreshEffect;
use crate::domain::history::AccountIdentity;
use crate::domain::history::AccountSelection;
use crate::domain::history::Forecast as HistoryForecast;
use crate::domain::history::ForecastStatus as HistoryForecastStatus;
use crate::domain::history::HistoryGenerationInput;
use crate::domain::history::HistoryStorageStatus;
use crate::domain::history::HistoryWindowInput;
use crate::domain::history::UsageHistoryQueryError;
use crate::domain::history::UsageHistoryRequest;
use crate::domain::settings::Settings;
use crate::domain::usage::DashboardSnapshot;
use crate::domain::usage::ForecastStatus as ModelForecastStatus;
use crate::domain::usage::QuotaFallbackLabel;
use crate::domain::usage::QuotaForecast;
use crate::infra::history_store::clear_legacy_backup_for_current_account;
use crate::infra::history_store::save_history;
use crate::infra::usage::UsageAccountIdentity;
use crate::ipc::events::emit_dashboard;
use crate::ipc::events::emit_usage_history_updated;
use chrono::DateTime;
use chrono::Duration as ChronoDuration;
use chrono::Local;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::sync::Arc;
use tauri::AppHandle;
use tauri::Emitter;
use tauri::Manager;

impl AppState {
    pub(crate) async fn clear_snapshot_forecasts(&self) -> DashboardSnapshot {
        let mut snapshot = self.snapshot.lock().await;
        for window in &mut snapshot.quota_windows {
            window.forecast = None;
        }
        snapshot.clone()
    }

    pub(crate) async fn apply_history_to_snapshot(
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
        let generations = self
            .quota_auto_continue
            .current_generation()
            .and_then(|generation| {
                quota_window_index(snapshot, generation.window_seconds).map(|window_index| {
                    HistoryGenerationInput {
                        window_id: snapshot.quota_windows[window_index].id.clone(),
                        window_seconds: generation.window_seconds,
                        generation_id: generation.generation_id,
                    }
                })
            })
            .into_iter()
            .collect::<Vec<_>>();
        let identity = match account_identity {
            UsageAccountIdentity::AccountId(value) => AccountIdentity::AccountId(value),
            UsageAccountIdentity::Token(value) => AccountIdentity::Token(value),
        };
        let mut runtime = self.shared.usage_history.lock().await;
        // 与专属采集开关命令在同一锁序下重读，避免禁用期间的在途刷新继续采样。
        let history_enabled = self.current_settings().history_enabled;
        if !history_enabled {
            let account_changed = match runtime.history.select_account(identity) {
                Ok(AccountSelection::Unchanged) => false,
                Ok(AccountSelection::Initialized | AccountSelection::Switched { .. }) => true,
                Err(_) => {
                    runtime.storage_status = HistoryStorageStatus::Unavailable;
                    false
                }
            };
            if account_changed {
                let saved = save_history(&self.shared.history_path, &runtime.history);
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
                account_changed: account_changed && self.account.is_none(),
            };
        }
        let mutation = match runtime.history.record_successful_snapshot_with_generations(
            identity,
            sampled_at,
            &inputs,
            &generations,
        ) {
            Ok(mutation) => mutation,
            Err(_) => {
                runtime.storage_status = HistoryStorageStatus::Unavailable;
                log::warn!("本地趋势采样未完成：类别=identity-or-salt。");
                return HistoryRefreshEffect::default();
            }
        };

        if mutation.changed() {
            let saved = save_history(&self.shared.history_path, &runtime.history);
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
            account_changed: mutation.account_changed && self.account.is_none(),
        }
    }
}

pub(crate) fn convert_history_forecast(forecast: HistoryForecast) -> QuotaForecast {
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

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum HistoryResponseStorageStatus {
    Ready,
    Empty,
    Recovered,
    Unavailable,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageHistorySeriesResponse {
    pub(crate) current_reset_at: Option<DateTime<Utc>>,
    pub(crate) window_id: String,
    pub(crate) window_seconds: i64,
    pub(crate) fallback_label: QuotaFallbackLabel,
    pub(crate) current_remaining_percent: Option<u8>,
    pub(crate) today_consumed_percent: u32,
    pub(crate) points: Vec<crate::domain::history::UsageHistoryPoint>,
    pub(crate) forecast: QuotaForecast,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageHistoryResponse {
    pub(crate) account_id: Option<String>,
    pub(crate) request: UsageHistoryRequest,
    pub(crate) applied_start_at: DateTime<Utc>,
    pub(crate) applied_end_at_exclusive: DateTime<Utc>,
    pub(crate) available_start_at: Option<DateTime<Utc>>,
    pub(crate) available_end_at: Option<DateTime<Utc>>,
    pub(crate) truncated_by_retention: bool,
    pub(crate) bucket_seconds: Option<i64>,
    pub(crate) history_enabled: bool,
    pub(crate) storage_status: HistoryResponseStorageStatus,
    pub(crate) sample_count: usize,
    pub(crate) earliest_sample_at: Option<DateTime<Utc>>,
    pub(crate) latest_sample_at: Option<DateTime<Utc>>,
    pub(crate) series: Vec<UsageHistorySeriesResponse>,
}

pub(crate) fn fallback_label_for_duration(window_seconds: i64) -> QuotaFallbackLabel {
    if window_seconds <= 12 * 60 * 60 {
        QuotaFallbackLabel::FiveHour
    } else if (6 * 24 * 60 * 60..=8 * 24 * 60 * 60).contains(&window_seconds) {
        QuotaFallbackLabel::Weekly
    } else {
        QuotaFallbackLabel::Window
    }
}

pub(crate) fn history_query_error_code(error: UsageHistoryQueryError) -> String {
    match error {
        UsageHistoryQueryError::EmptyOrReversedRange => "historyRangeInvalid",
        UsageHistoryQueryError::FutureEnd => "historyRangeFuture",
    }
    .to_owned()
}

/// 将系统本地日期的第一个有效墙上时刻转换为 UTC。极少数时区会在午夜切换
/// 夏令时，因此逐分钟寻找可表示的当天起点，避免错误退化成滚动 24 小时。
pub(crate) fn local_day_start_utc(now: DateTime<Local>) -> DateTime<Utc> {
    let date = now.date_naive();
    for minute in 0..24 * 60 {
        let Some(local_time) = date
            .and_hms_opt(0, 0, 0)
            .map(|midnight| midnight + ChronoDuration::minutes(minute))
        else {
            break;
        };
        if let Some(start) = local_time.and_local_timezone(Local).earliest() {
            return start.with_timezone(&Utc);
        }
    }
    // 理论上仅当系统时区数据库异常才会到达；保持边界不晚于当前时刻更安全。
    now.with_timezone(&Utc)
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AnalyticsRequest {
    #[serde(rename_all = "camelCase")]
    Range {
        window_id: String,
        start_at: DateTime<Utc>,
        end_at: DateTime<Utc>,
    },
    #[serde(rename_all = "camelCase")]
    Cycles {
        window_id: String,
        request: UsageHistoryRequest,
    },
    #[serde(rename_all = "camelCase")]
    Samples {
        window_id: String,
        cycle_id: String,
        offset: usize,
    },
}

pub(crate) async fn query(
    state: &AppState,
    request: UsageHistoryRequest,
) -> Result<UsageHistoryResponse, String> {
    let now_local = Local::now();
    let now = now_local.with_timezone(&Utc);
    let today_started_at = local_day_start_utc(now_local);
    let (query, storage_status) = {
        let mut runtime = state.shared.usage_history.lock().await;
        state.select_history_account(&mut runtime)?;
        let storage_summary = runtime.history.summary();
        let query = runtime
            .history
            .query_request_with_day_start(request, now, today_started_at)
            .map_err(history_query_error_code)?;
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
        (query, storage_status)
    };
    let series = query
        .series
        .into_iter()
        .map(|series| {
            let forecast = convert_history_forecast(series.forecast);
            UsageHistorySeriesResponse {
                current_reset_at: series.current_reset_at,
                window_id: series.window_id,
                window_seconds: series.window_seconds,
                fallback_label: fallback_label_for_duration(series.window_seconds),
                current_remaining_percent: Some(series.current_remaining_percent),
                today_consumed_percent: series.today_consumed_percent,
                points: series.points,
                forecast,
            }
        })
        .collect();
    Ok(UsageHistoryResponse {
        account_id: state.account.as_ref().map(|a| a.id.clone()),
        request: query.request,
        applied_start_at: query.applied_start_at,
        applied_end_at_exclusive: query.applied_end_at_exclusive,
        available_start_at: query.available_start_at,
        available_end_at: query.available_end_at,
        truncated_by_retention: query.truncated_by_retention,
        bucket_seconds: query.bucket_seconds,
        history_enabled: state.current_settings().history_enabled,
        storage_status,
        sample_count: query.sample_count as usize,
        earliest_sample_at: query.earliest_sample_at,
        latest_sample_at: query.latest_sample_at,
        series,
    })
}

pub(crate) async fn set_collection_enabled(
    state: &AppState,
    app: &AppHandle,
    enabled: bool,
) -> Result<Settings, String> {
    // 先等待在途刷新，再同时占用刷新与排期锁。这样命令返回后既不会回写旧采集状态，
    // 也不会让已取消的 deadline 在配置发布和重新排期之间启动。
    let refresh_barrier = state.refresh_guard.lock().await;
    let schedule_guard = state.schedule_guard.lock().await;
    let mut settings = state.current_settings();
    settings.history_enabled = enabled;
    let saved = state.save_preferences(settings).inspect_err(|_| {
        log::warn!("更新本地趋势采集状态失败：类别=settings-storage。");
    })?;
    if !enabled {
        state.clear_snapshot_forecasts().await;
    }
    let snapshot = state.reschedule_from_now_locked().await;
    drop(schedule_guard);
    if app.emit("settings-updated", saved.clone()).is_err() {
        log::warn!("广播本地趋势采集设置失败：类别=event。");
    }
    state.apply_auto_main_height(app, &snapshot);
    emit_dashboard(app, snapshot);
    drop(refresh_barrier);
    if let Some(hub) = app.try_state::<Arc<AccountHub>>() {
        for child in hub.all() {
            let _guard = child.refresh_guard.lock().await;
            let snapshot = if enabled {
                child.current_snapshot().await
            } else {
                child.clear_snapshot_forecasts().await
            };
            child.apply_auto_main_height(app, &snapshot);
            emit_dashboard(app, snapshot);
        }
    }
    log::info!("本地趋势采集状态已更新：enabled={enabled}。");
    Ok(saved)
}

pub(crate) async fn clear_account(state: &AppState, app: &AppHandle) -> Result<(), String> {
    // 与刷新严格使用 refresh_guard -> usage_history 的统一锁序，防止固定临时文件争用或旧数据回写。
    let _refresh_barrier = state.refresh_guard.lock().await;
    let removed;
    {
        let mut runtime = state.shared.usage_history.lock().await;
        state.select_history_account(&mut runtime)?;
        let mut replacement = runtime.history.clone();
        removed = replacement.clear_samples();
        let saved =
            clear_legacy_backup_for_current_account(&state.shared.history_path, &replacement)
                .and_then(|_| save_history(&state.shared.history_path, &replacement));
        if saved.is_ok() {
            runtime.history = replacement;
        }
        runtime.storage_status = if saved.is_ok() {
            HistoryStorageStatus::Ready
        } else {
            HistoryStorageStatus::Unavailable
        };
        saved.map_err(|_| {
            log::warn!("清除本地趋势历史失败：类别=storage。");
            "historyStorageUnavailable".to_owned()
        })?;
    }
    let snapshot = state.clear_snapshot_forecasts().await;
    state.apply_auto_main_height(app, &snapshot);
    emit_dashboard(app, snapshot);
    emit_usage_history_updated(app, state.account.as_ref().map(|a| a.id.clone()));
    log::info!("用户已清除当前账号本地趋势历史：samples_removed={removed}。");
    Ok(())
}

pub(crate) async fn analytics(
    state: &AppState,
    request: AnalyticsRequest,
) -> Result<serde_json::Value, String> {
    let mut history = state.shared.usage_history.lock().await;
    state.select_history_account(&mut history)?;
    let now = Utc::now();
    let value = match request {
        AnalyticsRequest::Range {
            window_id,
            start_at,
            end_at,
        } => serde_json::to_value(
            history
                .history
                .range_summary(&window_id, start_at, end_at, now)?,
        ),
        AnalyticsRequest::Cycles { window_id, request } => {
            serde_json::to_value(history.history.cycle_summaries(&window_id, request, now)?)
        }
        AnalyticsRequest::Samples {
            window_id,
            cycle_id,
            offset,
        } => serde_json::to_value(
            history
                .history
                .cycle_samples(&window_id, &cycle_id, offset, now)?,
        ),
    };
    value.map_err(|_| "invalidResponse".into())
}

#[cfg(test)]
mod tests;
