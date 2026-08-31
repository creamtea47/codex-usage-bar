use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fmt, fs, io,
    path::Path,
};

pub const USAGE_HISTORY_FILE_NAME: &str = "usage-history.json";
pub const USAGE_HISTORY_SCHEMA_VERSION: u32 = 2;

const SALT_BYTES: usize = 32;
const FINGERPRINT_BYTES: usize = 32;
const STREAM_KEY_BYTES: usize = 32;
const SAMPLE_INTERVAL_MINUTES: i64 = 5;
const FORECAST_LOOKBACK_HOURS: i64 = 6;
const FORECAST_MIN_SAMPLES: usize = 4;
const FORECAST_MIN_SPAN_MINUTES: i64 = 30;
const FORECAST_MIN_CONSUMPTION_PERCENT: u8 = 2;
const FORECAST_ROUND_SECONDS: i64 = 15 * 60;
const SEVEN_DAY_BUCKET_SECONDS: i64 = 15 * 60;
const MONTH_BUCKET_SECONDS: i64 = 60 * 60;
const DAILY_BUCKET_SECONDS: i64 = 24 * 60 * 60;
const MAX_QUERY_POINTS_PER_SERIES: usize = 1_000;
const MAX_WINDOW_ID_BYTES: usize = 256;
const GENERATION_ID_BYTES: usize = 32;

/// 原始账号标识只应短暂存在于调用栈中。自定义 `Debug` 会固定脱敏，避免测试或错误
/// 日志不慎打印 account id / Token。
#[derive(Clone, Copy)]
pub enum AccountIdentity<'a> {
    AccountId(&'a str),
    Token(&'a str),
}

impl<'a> AccountIdentity<'a> {
    /// 账号 ID 优先；只有缺失时才允许 Token 作为哈希输入。
    #[cfg(test)]
    pub fn from_parts(account_id: Option<&'a str>, token: &'a str) -> Self {
        account_id
            .filter(|value| !value.trim().is_empty())
            .map(Self::AccountId)
            .unwrap_or(Self::Token(token))
    }

    fn kind_and_value(self) -> (&'static [u8], &'a str) {
        match self {
            Self::AccountId(value) => (b"account-id", value),
            Self::Token(value) => (b"token-fallback", value),
        }
    }
}

impl fmt::Debug for AccountIdentity<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::AccountId(_) => "AccountId",
            Self::Token(_) => "Token",
        };
        formatter.debug_tuple(kind).field(&"[redacted]").finish()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FingerprintError {
    #[error("history salt is invalid")]
    InvalidSalt,
    #[error("account identity is empty")]
    EmptyIdentity,
}

#[derive(Debug, thiserror::Error)]
pub enum HistoryPersistenceError {
    #[error("could not serialize local usage history")]
    Serialize,
    #[error("could not write local usage history")]
    Storage,
}

/// 该状态可安全用于诊断摘要；它不携带路径、I/O 原因或文件内容。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HistoryStorageStatus {
    Missing,
    Ready,
    RecoveredCorrupt,
    RecoveredUnsupported,
    Unavailable,
}

#[derive(Debug)]
pub struct LoadedUsageHistory {
    pub history: UsageHistory,
    pub status: HistoryStorageStatus,
    /// 旧数据被裁剪，或文件无法安全读取时，集成层可据此尽快重写安全文件。
    pub needs_rewrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsageHistory {
    schema_version: u32,
    salt: String,
    account_fingerprint: Option<String>,
    streams: Vec<StoredUsageStream>,
    /// 非当前账号的匿名历史分区。当前账号仍保留在根字段，避免查询、预测和采样链路
    /// 为多账号存储承担不必要的间接层。
    accounts: Vec<StoredAccountHistory>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredAccountHistory {
    account_fingerprint: String,
    streams: Vec<StoredUsageStream>,
}

/// schema v1 只保存当前账号；显式旧结构确保升级不会把未知或损坏内容当成空历史。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UsageHistoryV1 {
    schema_version: u32,
    salt: String,
    account_fingerprint: Option<String>,
    streams: Vec<StoredUsageStream>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredUsageStream {
    window_id: String,
    window_seconds: i64,
    cycles: Vec<StoredUsageCycle>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredUsageCycle {
    /// 统一重置识别器生成的脱敏代次 ID。旧版历史没有该字段时保持 `None`，
    /// 由首次带代次的采样就地接管，避免升级本身制造一条虚假的趋势断点。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generation_id: Option<String>,
    reset_at: Option<DateTime<Utc>>,
    samples: Vec<StoredUsageSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredUsageSample {
    sampled_at: DateTime<Utc>,
    remaining_percent: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryWindowInput {
    pub window_id: String,
    pub window_seconds: i64,
    pub cycle_reset_at: Option<DateTime<Utc>>,
    pub remaining_percent: u8,
}

/// 调用层把统一重置识别器的当前代次与同一份成功快照配对后传入。
/// 原始窗口 ID 仅用于本次内存关联，持久化前仍会转换为本机加盐的流指纹。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryGenerationInput {
    pub window_id: String,
    pub window_seconds: i64,
    pub generation_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountSelection {
    Initialized,
    Unchanged,
    Switched { created: bool },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistoryMutation {
    pub account_changed: bool,
    /// 升级旧历史时可能只补上代次元数据而不新增采样点，仍需要触发安全落盘。
    pub generation_metadata_updated: bool,
    pub samples_recorded: usize,
    pub samples_pruned: usize,
    pub ignored_windows: usize,
}

impl HistoryMutation {
    pub fn changed(self) -> bool {
        self.account_changed
            || self.generation_metadata_updated
            || self.samples_recorded > 0
            || self.samples_pruned > 0
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum UsageHistoryPreset {
    #[serde(rename = "24h")]
    Hours24,
    #[serde(rename = "7d")]
    Days7,
    #[serde(rename = "30d")]
    Days30,
    #[serde(rename = "all")]
    All,
}

impl UsageHistoryPreset {
    fn duration(self) -> Option<Duration> {
        match self {
            Self::Hours24 => Some(Duration::hours(24)),
            Self::Days7 => Some(Duration::days(7)),
            Self::Days30 => Some(Duration::days(30)),
            Self::All => None,
        }
    }
}

/// 设置页历史查询的唯一结构化契约。前端负责把本地日历边界转换成 UTC instant；
/// Rust 只接受明确的半开区间，并再次限制顺序与未来端点。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum UsageHistoryRequest {
    Preset {
        preset: UsageHistoryPreset,
    },
    Custom {
        #[serde(rename = "startAt")]
        start_at: DateTime<Utc>,
        #[serde(rename = "endAtExclusive")]
        end_at_exclusive: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedUsageHistoryRange {
    pub start_at: DateTime<Utc>,
    pub end_at_exclusive: DateTime<Utc>,
    pub bucket_seconds: Option<i64>,
    pub truncated_by_retention: bool,
}

#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum UsageHistoryQueryError {
    #[error("history range must be non-empty and ordered")]
    EmptyOrReversedRange,
    #[error("history range cannot end in the future")]
    FutureEnd,
}

impl UsageHistoryRequest {
    pub fn resolve(
        self,
        now: DateTime<Utc>,
        available_start_at: Option<DateTime<Utc>>,
    ) -> Result<ResolvedUsageHistoryRange, UsageHistoryQueryError> {
        let (requested_start_at, end_at_exclusive) = match self {
            Self::Preset { preset } => (
                preset
                    .duration()
                    .map(|duration| safe_subtract(now, duration))
                    .or(available_start_at)
                    .unwrap_or(now),
                now,
            ),
            Self::Custom {
                start_at,
                end_at_exclusive,
            } => {
                if start_at >= end_at_exclusive {
                    return Err(UsageHistoryQueryError::EmptyOrReversedRange);
                }
                if end_at_exclusive > now {
                    return Err(UsageHistoryQueryError::FutureEnd);
                }
                (start_at, end_at_exclusive)
            }
        };
        let span = end_at_exclusive - requested_start_at;
        let bucket_seconds = if span <= Duration::hours(24) {
            None
        } else if span <= Duration::days(7) {
            Some(SEVEN_DAY_BUCKET_SECONDS)
        } else if span <= Duration::days(32) {
            Some(MONTH_BUCKET_SECONDS)
        } else {
            Some(DAILY_BUCKET_SECONDS)
        };
        Ok(ResolvedUsageHistoryRange {
            start_at: requested_start_at,
            end_at_exclusive,
            bucket_seconds,
            truncated_by_retention: false,
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ForecastStatus {
    Collecting,
    Stable,
    ExhaustsBeforeReset,
    LastsUntilReset,
}

/// 与 `QuotaWindow.forecast` 对齐的安全预测 DTO。没有可靠耗尽时间时
/// `exhausts_at` 必须为 `None`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Forecast {
    pub status: ForecastStatus,
    pub exhausts_at: Option<DateTime<Utc>>,
    pub sample_count: u32,
    pub observed_span_seconds: i64,
    pub consumed_percent: u8,
}

impl Forecast {
    fn collecting(samples: &[&StoredUsageSample]) -> Self {
        Self::from_observations(ForecastStatus::Collecting, None, samples)
    }

    fn from_observations(
        status: ForecastStatus,
        exhausts_at: Option<DateTime<Utc>>,
        samples: &[&StoredUsageSample],
    ) -> Self {
        let observed_span_seconds = samples
            .first()
            .zip(samples.last())
            .map(|(first, last)| (last.sampled_at - first.sampled_at).num_seconds().max(0))
            .unwrap_or(0);
        let consumed_percent = samples
            .first()
            .zip(samples.last())
            .map(|(first, last)| {
                first
                    .remaining_percent
                    .saturating_sub(last.remaining_percent)
            })
            .unwrap_or(0);
        Self {
            status,
            exhausts_at,
            sample_count: samples.len().min(u32::MAX as usize) as u32,
            observed_span_seconds,
            consumed_percent,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageHistoryPoint {
    pub sampled_at: DateTime<Utc>,
    pub remaining_percent: u8,
    /// 前端在此点断线；不需要获得内部保存的 reset 时间。
    pub break_before: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageHistorySeries {
    pub window_id: String,
    pub window_seconds: i64,
    pub current_remaining_percent: u8,
    /// 按调用方提供的用户本地自然日边界累计；跨额度周期时分别计算后相加。
    pub today_consumed_percent: u32,
    pub points: Vec<UsageHistoryPoint>,
    pub forecast: Forecast,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageHistoryQuery {
    pub request: UsageHistoryRequest,
    pub generated_at: DateTime<Utc>,
    pub applied_start_at: DateTime<Utc>,
    pub applied_end_at_exclusive: DateTime<Utc>,
    pub available_start_at: Option<DateTime<Utc>>,
    pub available_end_at: Option<DateTime<Utc>>,
    pub truncated_by_retention: bool,
    pub bucket_seconds: Option<i64>,
    pub sample_count: u32,
    pub earliest_sample_at: Option<DateTime<Utc>>,
    pub latest_sample_at: Option<DateTime<Utc>>,
    pub series: Vec<UsageHistorySeries>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageHistorySummary {
    pub stream_count: u32,
    pub sample_count: u32,
    pub oldest_sample_at: Option<DateTime<Utc>>,
    pub latest_sample_at: Option<DateTime<Utc>>,
}

impl UsageHistory {
    pub fn new_random() -> Self {
        Self {
            schema_version: USAGE_HISTORY_SCHEMA_VERSION,
            salt: generate_local_salt(),
            account_fingerprint: None,
            streams: Vec::new(),
            accounts: Vec::new(),
        }
    }

    /// 供迁移和确定性测试使用。生产初始化应使用 `new_random` 或 `load_history_at`。
    #[cfg(test)]
    pub fn with_salt(salt: String) -> Result<Self, FingerprintError> {
        decode_hex_exact(&salt, SALT_BYTES).ok_or(FingerprintError::InvalidSalt)?;
        Ok(Self {
            schema_version: USAGE_HISTORY_SCHEMA_VERSION,
            salt,
            account_fingerprint: None,
            streams: Vec::new(),
            accounts: Vec::new(),
        })
    }

    pub fn select_account(
        &mut self,
        identity: AccountIdentity<'_>,
    ) -> Result<AccountSelection, FingerprintError> {
        let fingerprint = account_fingerprint(&self.salt, identity)?;
        match self.account_fingerprint.as_deref() {
            None => {
                self.account_fingerprint = Some(fingerprint);
                log::info!("本地趋势账号分区已初始化：accounts=1。");
                Ok(AccountSelection::Initialized)
            }
            Some(current) if current == fingerprint => Ok(AccountSelection::Unchanged),
            Some(_) => {
                let previous = StoredAccountHistory {
                    account_fingerprint: self
                        .account_fingerprint
                        .take()
                        .expect("selected account must have a fingerprint"),
                    streams: std::mem::take(&mut self.streams),
                };
                let target = self
                    .accounts
                    .iter()
                    .position(|account| account.account_fingerprint == fingerprint)
                    .map(|index| self.accounts.remove(index));
                let created = target.is_none();
                if let Some(target) = target {
                    self.account_fingerprint = Some(target.account_fingerprint);
                    self.streams = target.streams;
                } else {
                    self.account_fingerprint = Some(fingerprint);
                }
                self.accounts.push(previous);
                log::info!(
                    "本地趋势账号分区已切换：created={created}，accounts={}。",
                    self.accounts.len() + 1
                );
                Ok(AccountSelection::Switched { created })
            }
        }
    }

    /// 只应在成功获取用量快照后调用。该方法先完成账号隔离，再依据采样策略写入。
    #[allow(dead_code)] // 保留 v0.4.0 调用兼容与旧历史启发式回归测试。
    pub fn record_successful_snapshot(
        &mut self,
        identity: AccountIdentity<'_>,
        sampled_at: DateTime<Utc>,
        windows: &[HistoryWindowInput],
    ) -> Result<HistoryMutation, FingerprintError> {
        self.record_successful_snapshot_with_generations(identity, sampled_at, windows, &[])
    }

    /// 使用统一重置识别器的代次记录成功快照。只要同一额度窗口的代次发生变化，
    /// 即使 `reset_at` 没变、额度只从 98/99% 恢复到 100%，趋势也会可靠断开。
    ///
    /// `generations` 是可选旁路输入，保留旧调用方的兼容行为；无代次时仍使用
    /// 原有的 `reset_at`/额度回升推断，便于升级期间连续采样。
    pub fn record_successful_snapshot_with_generations(
        &mut self,
        identity: AccountIdentity<'_>,
        sampled_at: DateTime<Utc>,
        windows: &[HistoryWindowInput],
        generations: &[HistoryGenerationInput],
    ) -> Result<HistoryMutation, FingerprintError> {
        let mut mutation = HistoryMutation::default();
        match self.select_account(identity)? {
            AccountSelection::Initialized => mutation.account_changed = true,
            AccountSelection::Unchanged => {}
            AccountSelection::Switched { .. } => mutation.account_changed = true,
        }

        mutation.samples_pruned = self.compact_at(sampled_at);
        let generation_by_window = generations
            .iter()
            .filter(|generation| valid_generation_input(generation))
            .map(|generation| {
                (
                    (generation.window_id.as_str(), generation.window_seconds),
                    generation.generation_id.as_str(),
                )
            })
            .collect::<HashMap<_, _>>();
        let mut seen = HashSet::new();
        for window in windows {
            if !valid_window_input(window) {
                mutation.ignored_windows += 1;
                continue;
            }
            let Some(stream_key) =
                window_stream_key(&self.salt, window.window_id.as_str(), window.window_seconds)
            else {
                mutation.ignored_windows += 1;
                continue;
            };
            if !seen.insert((stream_key.clone(), window.window_seconds)) {
                mutation.ignored_windows += 1;
                continue;
            }
            // 原始上游 ID 只参与本机加盐哈希，不进入文件、查询 DTO 或设置窗口 IPC。
            let stored_window = HistoryWindowInput {
                window_id: stream_key,
                window_seconds: window.window_seconds,
                cycle_reset_at: window.cycle_reset_at,
                remaining_percent: window.remaining_percent,
            };
            let generation_id = generation_by_window
                .get(&(window.window_id.as_str(), window.window_seconds))
                .copied();

            let stream_index = self
                .streams
                .iter()
                .position(|stream| stream.matches(&stored_window));
            let (recorded, generation_metadata_updated) = if let Some(index) = stream_index {
                self.streams[index].record(sampled_at, &stored_window, generation_id)
            } else {
                self.streams.push(StoredUsageStream::from_first_sample(
                    sampled_at,
                    &stored_window,
                    generation_id,
                ));
                (true, false)
            };
            mutation.generation_metadata_updated |= generation_metadata_updated;

            if recorded {
                mutation.samples_recorded += 1;
            }
        }
        Ok(mutation)
    }

    /// 用户清除历史时只清当前账号，并保留随机盐和账号指纹，避免影响其他分区。
    pub fn clear_samples(&mut self) -> usize {
        let removed = self.sample_count();
        self.streams.clear();
        removed
    }

    pub fn compact_at(&mut self, now: DateTime<Utc>) -> usize {
        let mut removed = 0;
        for stream in &mut self.streams {
            removed += stream.compact_for_age(now);
        }
        for account in &mut self.accounts {
            for stream in &mut account.streams {
                removed += stream.compact_for_age(now);
            }
        }
        removed
    }

    /// 测试查询默认以 UTC 自然日为“今日”边界；桌面入口会显式传入用户本地日边界。
    #[cfg(test)]
    pub fn query_request(
        &self,
        request: UsageHistoryRequest,
        now: DateTime<Utc>,
    ) -> Result<UsageHistoryQuery, UsageHistoryQueryError> {
        let today_started_at = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map(|value| value.and_utc())
            .unwrap_or(now);
        self.query_request_with_day_start(request, now, today_started_at)
    }

    /// 查询指定图表范围，同时用调用方解析好的本地自然日边界独立计算今日消耗。
    /// 图表范围可以是预设、全部历史或自定义范围，不得影响今日统计口径。
    pub fn query_request_with_day_start(
        &self,
        request: UsageHistoryRequest,
        now: DateTime<Utc>,
        today_started_at: DateTime<Utc>,
    ) -> Result<UsageHistoryQuery, UsageHistoryQueryError> {
        let available_summary = self.summary_available_at(now);
        let resolved = request.resolve(now, available_summary.oldest_sample_at)?;
        let range_summary = self.summary_between(resolved.start_at, resolved.end_at_exclusive);
        let mut series = self
            .streams
            .iter()
            .filter_map(|stream| {
                let points = stream.query_points(
                    resolved.start_at,
                    resolved.end_at_exclusive,
                    resolved.bucket_seconds,
                );
                if points.is_empty() {
                    return None;
                }
                let current_remaining_percent = stream.current_remaining_percent(now)?;
                Some(UsageHistorySeries {
                    window_id: stream.window_id.clone(),
                    window_seconds: stream.window_seconds,
                    current_remaining_percent,
                    today_consumed_percent: stream.consumed_between(today_started_at, now),
                    points,
                    forecast: stream.forecast(now),
                })
            })
            .collect::<Vec<_>>();
        series.sort_by(|left, right| {
            left.window_seconds
                .cmp(&right.window_seconds)
                .then_with(|| left.window_id.cmp(&right.window_id))
        });
        Ok(UsageHistoryQuery {
            request,
            generated_at: now,
            applied_start_at: resolved.start_at,
            applied_end_at_exclusive: resolved.end_at_exclusive,
            available_start_at: available_summary.oldest_sample_at,
            available_end_at: available_summary.latest_sample_at,
            truncated_by_retention: resolved.truncated_by_retention,
            bucket_seconds: resolved.bucket_seconds,
            sample_count: range_summary.sample_count,
            earliest_sample_at: range_summary.oldest_sample_at,
            latest_sample_at: range_summary.latest_sample_at,
            series,
        })
    }

    pub fn forecast_for(
        &self,
        window_id: &str,
        window_seconds: i64,
        now: DateTime<Utc>,
    ) -> Option<Forecast> {
        let stream_key = window_stream_key(&self.salt, window_id, window_seconds)?;
        self.streams
            .iter()
            .find(|stream| {
                stream.window_id == stream_key && stream.window_seconds == window_seconds
            })
            .map(|stream| stream.forecast(now))
    }

    pub fn summary(&self) -> UsageHistorySummary {
        let mut oldest_sample_at = None;
        let mut latest_sample_at = None;
        for sample in self.all_samples() {
            oldest_sample_at = Some(
                oldest_sample_at
                    .map(|current: DateTime<Utc>| current.min(sample.sampled_at))
                    .unwrap_or(sample.sampled_at),
            );
            latest_sample_at = Some(
                latest_sample_at
                    .map(|current: DateTime<Utc>| current.max(sample.sampled_at))
                    .unwrap_or(sample.sampled_at),
            );
        }
        UsageHistorySummary {
            stream_count: self.streams.len().min(u32::MAX as usize) as u32,
            sample_count: self.sample_count().min(u32::MAX as usize) as u32,
            oldest_sample_at,
            latest_sample_at,
        }
    }

    fn summary_between(
        &self,
        start_at: DateTime<Utc>,
        end_at_exclusive: DateTime<Utc>,
    ) -> UsageHistorySummary {
        let mut stream_count = 0_u32;
        let mut sample_count = 0_u32;
        let mut oldest_sample_at = None;
        let mut latest_sample_at = None;

        for stream in &self.streams {
            let mut stream_has_samples = false;
            for sample in stream
                .cycles
                .iter()
                .flat_map(|cycle| &cycle.samples)
                .filter(|sample| {
                    sample.sampled_at >= start_at && sample.sampled_at < end_at_exclusive
                })
            {
                stream_has_samples = true;
                sample_count = sample_count.saturating_add(1);
                oldest_sample_at = Some(
                    oldest_sample_at
                        .map(|current: DateTime<Utc>| current.min(sample.sampled_at))
                        .unwrap_or(sample.sampled_at),
                );
                latest_sample_at = Some(
                    latest_sample_at
                        .map(|current: DateTime<Utc>| current.max(sample.sampled_at))
                        .unwrap_or(sample.sampled_at),
                );
            }
            if stream_has_samples {
                stream_count = stream_count.saturating_add(1);
            }
        }

        UsageHistorySummary {
            stream_count,
            sample_count,
            oldest_sample_at,
            latest_sample_at,
        }
    }

    fn summary_available_at(&self, now: DateTime<Utc>) -> UsageHistorySummary {
        let mut stream_count = 0_u32;
        let mut sample_count = 0_u32;
        let mut oldest_sample_at = None;
        let mut latest_sample_at = None;
        for stream in &self.streams {
            let mut stream_has_samples = false;
            for sample in stream
                .cycles
                .iter()
                .flat_map(|cycle| &cycle.samples)
                .filter(|sample| sample.sampled_at <= now)
            {
                stream_has_samples = true;
                sample_count = sample_count.saturating_add(1);
                oldest_sample_at = Some(
                    oldest_sample_at
                        .map(|current: DateTime<Utc>| current.min(sample.sampled_at))
                        .unwrap_or(sample.sampled_at),
                );
                latest_sample_at = Some(
                    latest_sample_at
                        .map(|current: DateTime<Utc>| current.max(sample.sampled_at))
                        .unwrap_or(sample.sampled_at),
                );
            }
            if stream_has_samples {
                stream_count = stream_count.saturating_add(1);
            }
        }
        UsageHistorySummary {
            stream_count,
            sample_count,
            oldest_sample_at,
            latest_sample_at,
        }
    }

    pub fn sample_count(&self) -> usize {
        self.streams
            .iter()
            .flat_map(|stream| &stream.cycles)
            .map(|cycle| cycle.samples.len())
            .sum()
    }

    #[cfg(test)]
    pub fn stream_count(&self) -> usize {
        self.streams.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }

    fn all_samples(&self) -> impl Iterator<Item = &StoredUsageSample> {
        self.streams
            .iter()
            .flat_map(|stream| &stream.cycles)
            .flat_map(|cycle| &cycle.samples)
    }

    fn validate(&self) -> bool {
        if self.schema_version != USAGE_HISTORY_SCHEMA_VERSION
            || decode_hex_exact(&self.salt, SALT_BYTES).is_none()
            || self
                .account_fingerprint
                .as_deref()
                .is_some_and(|value| decode_hex_exact(value, FINGERPRINT_BYTES).is_none())
            || (self.account_fingerprint.is_none()
                && (!self.streams.is_empty() || !self.accounts.is_empty()))
        {
            return false;
        }

        let mut fingerprints = HashSet::new();
        if let Some(current) = self.account_fingerprint.as_deref() {
            fingerprints.insert(current);
        }
        validate_streams(&self.streams)
            && self.accounts.iter().all(|account| {
                decode_hex_exact(&account.account_fingerprint, FINGERPRINT_BYTES).is_some()
                    && fingerprints.insert(account.account_fingerprint.as_str())
                    && validate_streams(&account.streams)
            })
    }
}

fn validate_streams(streams: &[StoredUsageStream]) -> bool {
    let mut keys = HashSet::new();
    streams.iter().all(|stream| {
        valid_stored_window_key(&stream.window_id, stream.window_seconds)
            && keys.insert((stream.window_id.as_str(), stream.window_seconds))
            && stream.validate()
    })
}

impl StoredUsageStream {
    fn from_first_sample(
        sampled_at: DateTime<Utc>,
        window: &HistoryWindowInput,
        generation_id: Option<&str>,
    ) -> Self {
        Self {
            window_id: window.window_id.clone(),
            window_seconds: window.window_seconds,
            cycles: vec![StoredUsageCycle {
                generation_id: generation_id.map(ToOwned::to_owned),
                reset_at: window.cycle_reset_at,
                samples: vec![StoredUsageSample {
                    sampled_at,
                    remaining_percent: window.remaining_percent,
                }],
            }],
        }
    }

    fn matches(&self, window: &HistoryWindowInput) -> bool {
        self.window_id == window.window_id && self.window_seconds == window.window_seconds
    }

    fn record(
        &mut self,
        sampled_at: DateTime<Utc>,
        window: &HistoryWindowInput,
        generation_id: Option<&str>,
    ) -> (bool, bool) {
        let Some(latest_at) = self.latest_sample_at() else {
            *self = Self::from_first_sample(sampled_at, window, generation_id);
            return (true, false);
        };
        // 系统时间倒退时跳过该点，避免把当前周期写成非单调时间序列。
        if sampled_at < latest_at {
            return (false, false);
        }

        let cycle = self
            .cycles
            .last_mut()
            .expect("non-empty stream must contain a cycle");
        let last = cycle
            .samples
            .last()
            .expect("non-empty cycle must contain a sample");
        let generation_changed = cycle
            .generation_id
            .as_deref()
            .zip(generation_id)
            .is_some_and(|(previous, current)| previous != current);
        // 一旦调用层提供统一代次，它就是唯一的分段依据；继续叠加旧启发式会让
        // 同一代次内的 reset_at 校正或 99↔100 抖动制造重复断点。
        let reset_advanced = generation_id.is_none()
            && match (cycle.reset_at, window.cycle_reset_at) {
                (Some(previous), Some(next)) => next > previous && sampled_at >= previous,
                _ => false,
            };
        let inferred_reset = generation_id.is_none()
            && window.remaining_percent > last.remaining_percent
            && (cycle.reset_at.is_none()
                || cycle
                    .reset_at
                    .is_some_and(|known_reset| sampled_at >= known_reset));

        if generation_changed || reset_advanced || inferred_reset {
            self.cycles.push(StoredUsageCycle {
                generation_id: generation_id.map(ToOwned::to_owned),
                reset_at: window.cycle_reset_at,
                samples: vec![StoredUsageSample {
                    sampled_at,
                    remaining_percent: window.remaining_percent,
                }],
            });
            return (true, false);
        }

        let generation_metadata_updated = cycle.generation_id.is_none() && generation_id.is_some();
        if generation_metadata_updated {
            // 从 v0.4.0 历史平滑迁移：首次获知代次只补元数据，不把升级时刻伪装成重置。
            cycle.generation_id = generation_id.map(ToOwned::to_owned);
        }

        if let Some(next_reset) = window.cycle_reset_at {
            // 截止时间尚未到时，向后移动只视为服务端校正，不应断成新周期；
            // 记住最大值也能避免临时回拨后恢复时制造假重置。
            cycle.reset_at = Some(
                cycle
                    .reset_at
                    .map_or(next_reset, |known| known.max(next_reset)),
            );
        }
        let last = cycle
            .samples
            .last_mut()
            .expect("non-empty cycle must contain a sample");
        if sampled_at == last.sampled_at {
            if last.remaining_percent != window.remaining_percent {
                last.remaining_percent = window.remaining_percent;
                return (true, generation_metadata_updated);
            }
            return (false, generation_metadata_updated);
        }

        let interval_elapsed =
            sampled_at - last.sampled_at >= Duration::minutes(SAMPLE_INTERVAL_MINUTES);
        let percentage_changed = last.remaining_percent != window.remaining_percent;
        if !interval_elapsed && !percentage_changed {
            return (false, generation_metadata_updated);
        }
        cycle.samples.push(StoredUsageSample {
            sampled_at,
            remaining_percent: window.remaining_percent,
        });
        (true, generation_metadata_updated)
    }

    fn compact_for_age(&mut self, now: DateTime<Utc>) -> usize {
        let recent_cutoff = safe_subtract(now, Duration::hours(24));
        let medium_cutoff = safe_subtract(now, Duration::days(7));
        let daily_cutoff = safe_subtract(now, Duration::days(32));
        let mut removed = 0;
        for cycle in &mut self.cycles {
            let compacted =
                compact_cycle_for_age(&cycle.samples, daily_cutoff, medium_cutoff, recent_cutoff);
            removed += cycle.samples.len().saturating_sub(compacted.len());
            cycle.samples = compacted;
        }
        removed
    }

    fn latest_sample_at(&self) -> Option<DateTime<Utc>> {
        self.cycles
            .last()
            .and_then(|cycle| cycle.samples.last())
            .map(|sample| sample.sampled_at)
    }

    fn query_points(
        &self,
        start_at: DateTime<Utc>,
        end_at_exclusive: DateTime<Utc>,
        bucket_seconds: Option<i64>,
    ) -> Vec<UsageHistoryPoint> {
        let mut points = Vec::new();
        for cycle in &self.cycles {
            let samples = cycle
                .samples
                .iter()
                .filter(|sample| {
                    sample.sampled_at >= start_at && sample.sampled_at < end_at_exclusive
                })
                .collect::<Vec<_>>();
            let selected = if let Some(bucket_seconds) = bucket_seconds {
                downsample_cycle(&samples, bucket_seconds)
            } else {
                samples
            };
            for (index, sample) in selected.into_iter().enumerate() {
                points.push(UsageHistoryPoint {
                    sampled_at: sample.sampled_at,
                    remaining_percent: sample.remaining_percent,
                    break_before: index == 0,
                });
            }
        }
        bound_query_points(points, MAX_QUERY_POINTS_PER_SERIES)
    }

    fn current_remaining_percent(&self, now: DateTime<Utc>) -> Option<u8> {
        self.cycles
            .iter()
            .flat_map(|cycle| &cycle.samples)
            .rev()
            .find(|sample| sample.sampled_at <= now)
            .map(|sample| sample.remaining_percent)
    }

    /// 以自然日边界附近最后一个已知额度作为基线。每个额度周期单独计算，避免重置
    /// 造成剩余百分比回升时把当天已经发生的消耗抵消掉。
    fn consumed_between(&self, started_at: DateTime<Utc>, now: DateTime<Utc>) -> u32 {
        if started_at > now {
            return 0;
        }

        self.cycles.iter().fold(0_u32, |total, cycle| {
            let Some(latest) = cycle
                .samples
                .iter()
                .rev()
                .find(|sample| sample.sampled_at >= started_at && sample.sampled_at <= now)
            else {
                return total;
            };
            let baseline = cycle
                .samples
                .iter()
                .rev()
                .find(|sample| sample.sampled_at <= started_at)
                .or_else(|| {
                    cycle
                        .samples
                        .iter()
                        .find(|sample| sample.sampled_at >= started_at && sample.sampled_at <= now)
                })
                .expect("a cycle with a latest in-range sample must have a baseline");
            total.saturating_add(u32::from(
                baseline
                    .remaining_percent
                    .saturating_sub(latest.remaining_percent),
            ))
        })
    }

    fn forecast(&self, now: DateTime<Utc>) -> Forecast {
        let Some(cycle) = self.cycles.last() else {
            return Forecast::collecting(&[]);
        };
        let cutoff = safe_subtract(now, Duration::hours(FORECAST_LOOKBACK_HOURS));
        let samples = cycle
            .samples
            .iter()
            .filter(|sample| sample.sampled_at >= cutoff && sample.sampled_at <= now)
            .collect::<Vec<_>>();
        if samples.len() < FORECAST_MIN_SAMPLES {
            return Forecast::collecting(&samples);
        }

        let span_seconds = (samples.last().unwrap().sampled_at - samples[0].sampled_at)
            .num_seconds()
            .max(0);
        if span_seconds < Duration::minutes(FORECAST_MIN_SPAN_MINUTES).num_seconds() {
            return Forecast::collecting(&samples);
        }
        let consumed = samples[0]
            .remaining_percent
            .saturating_sub(samples.last().unwrap().remaining_percent);
        if consumed < FORECAST_MIN_CONSUMPTION_PERCENT {
            return Forecast::from_observations(ForecastStatus::Stable, None, &samples);
        }

        let Some(slope_per_second) = regression_slope(&samples) else {
            return Forecast::from_observations(ForecastStatus::Stable, None, &samples);
        };
        if slope_per_second >= 0.0 {
            return Forecast::from_observations(ForecastStatus::Stable, None, &samples);
        }

        let latest = samples.last().unwrap();
        let Some(reset_at) = cycle.reset_at.filter(|reset| *reset > latest.sampled_at) else {
            return Forecast::collecting(&samples);
        };
        let seconds_until_empty = latest.remaining_percent as f64 / -slope_per_second;
        if !seconds_until_empty.is_finite() || seconds_until_empty < 0.0 {
            return Forecast::from_observations(ForecastStatus::Stable, None, &samples);
        }
        let whole_seconds = seconds_until_empty.round().min(i64::MAX as f64) as i64;
        let Some(raw_exhausts_at) = latest
            .sampled_at
            .checked_add_signed(Duration::seconds(whole_seconds))
        else {
            return Forecast::from_observations(ForecastStatus::LastsUntilReset, None, &samples);
        };
        let Some(rounded_exhausts_at) = round_to_quarter_hour(raw_exhausts_at) else {
            return Forecast::from_observations(ForecastStatus::LastsUntilReset, None, &samples);
        };
        // 最近刻度若落到最新观测之前，优先提升到下一刻度；只有该刻度会越过
        // resetAt 时才保留已知的最新样本时间。预测绝不能显示为发生在过去。
        let rounded_exhausts_at = if rounded_exhausts_at < latest.sampled_at {
            ceil_to_quarter_hour(latest.sampled_at)
                .filter(|candidate| *candidate < reset_at)
                .unwrap_or(latest.sampled_at)
        } else {
            rounded_exhausts_at
        };

        if rounded_exhausts_at < reset_at {
            Forecast::from_observations(
                ForecastStatus::ExhaustsBeforeReset,
                Some(rounded_exhausts_at),
                &samples,
            )
        } else {
            // 不能把耗尽时间外推到 resetAt 之后。
            Forecast::from_observations(ForecastStatus::LastsUntilReset, None, &samples)
        }
    }

    fn validate(&self) -> bool {
        if self.cycles.is_empty() {
            return false;
        }
        let mut previous_cycle_last = None;
        self.cycles.iter().all(|cycle| {
            if cycle.samples.is_empty()
                || cycle
                    .generation_id
                    .as_deref()
                    .is_some_and(|generation| !valid_generation_id(generation))
            {
                return false;
            }
            let begins_after_previous =
                previous_cycle_last.is_none_or(|previous| cycle.samples[0].sampled_at >= previous);
            let samples_are_valid = cycle
                .samples
                .iter()
                .all(|sample| sample.remaining_percent <= 100)
                && cycle
                    .samples
                    .windows(2)
                    .all(|pair| pair[1].sampled_at > pair[0].sampled_at);
            previous_cycle_last = cycle.samples.last().map(|sample| sample.sampled_at);
            begins_after_previous && samples_are_valid
        })
    }
}

/// 生成 256-bit 本机随机盐。随机值可落盘，但不会通过 IPC 返回。
pub fn generate_local_salt() -> String {
    let bytes: [u8; SALT_BYTES] = rand::random();
    encode_hex(&bytes)
}

/// 计算带域分隔的 SHA-256 指纹。账号 ID 与 Token 即使文本相同，也不会得到同一指纹。
pub fn account_fingerprint(
    salt_hex: &str,
    identity: AccountIdentity<'_>,
) -> Result<String, FingerprintError> {
    let salt = decode_hex_exact(salt_hex, SALT_BYTES).ok_or(FingerprintError::InvalidSalt)?;
    let (kind, value) = identity.kind_and_value();
    let value = value.trim();
    if value.is_empty() {
        return Err(FingerprintError::EmptyIdentity);
    }
    let mut hasher = Sha256::new();
    hasher.update(b"codex-usage-bar/account-fingerprint/v1\0");
    hasher.update(kind);
    hasher.update(b"\0");
    hasher.update(salt);
    hasher.update(b"\0");
    hasher.update(value.as_bytes());
    Ok(encode_hex(&hasher.finalize()))
}

pub fn load_history(path: &Path) -> LoadedUsageHistory {
    load_history_at(path, Utc::now())
}

/// 所有文件错误都降级为空历史，不影响应用启动，也不把路径或底层错误带入返回值。
pub fn load_history_at(path: &Path, now: DateTime<Utc>) -> LoadedUsageHistory {
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return recovered_history(HistoryStorageStatus::Missing)
        }
        Err(_) => return recovered_history(HistoryStorageStatus::Unavailable),
    };
    let value: serde_json::Value = match serde_json::from_slice(&contents) {
        Ok(value) => value,
        Err(_) => return recovered_history(HistoryStorageStatus::RecoveredCorrupt),
    };
    let Some(version) = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
    else {
        return recovered_history(HistoryStorageStatus::RecoveredCorrupt);
    };
    let (mut history, migrated) = match version {
        1 => {
            let legacy = match serde_json::from_value::<UsageHistoryV1>(value) {
                Ok(legacy) if legacy.schema_version == 1 => legacy,
                _ => return recovered_history(HistoryStorageStatus::RecoveredCorrupt),
            };
            let history = UsageHistory {
                schema_version: USAGE_HISTORY_SCHEMA_VERSION,
                salt: legacy.salt,
                account_fingerprint: legacy.account_fingerprint,
                streams: legacy.streams,
                accounts: Vec::new(),
            };
            if !history.validate() {
                return recovered_history(HistoryStorageStatus::RecoveredCorrupt);
            }
            log::info!(
                "本地趋势历史已加载旧版待迁移数据：from_schema=1，to_schema=2，samples={}。",
                history.sample_count()
            );
            (history, true)
        }
        version if version == USAGE_HISTORY_SCHEMA_VERSION as u64 => {
            let history = match serde_json::from_value::<UsageHistory>(value) {
                Ok(history) if history.validate() => history,
                _ => return recovered_history(HistoryStorageStatus::RecoveredCorrupt),
            };
            (history, false)
        }
        _ => return recovered_history(HistoryStorageStatus::RecoveredUnsupported),
    };
    let removed = history.compact_at(now);
    LoadedUsageHistory {
        history,
        status: HistoryStorageStatus::Ready,
        needs_rewrite: migrated || removed > 0,
    }
}

pub fn save_history(path: &Path, history: &UsageHistory) -> Result<(), HistoryPersistenceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| HistoryPersistenceError::Storage)?;
    }
    let content =
        serde_json::to_vec_pretty(history).map_err(|_| HistoryPersistenceError::Serialize)?;
    let temporary_path = path.with_extension("json.tmp");
    let legacy_backup = legacy_backup_for(path)?;
    fs::write(&temporary_path, content).map_err(|_| HistoryPersistenceError::Storage)?;
    if path.exists() {
        fs::remove_file(path).map_err(|_| HistoryPersistenceError::Storage)?;
    }
    if fs::rename(&temporary_path, path).is_err() {
        let _ = fs::remove_file(&temporary_path);
        if !path.exists() {
            if let Some(backup) = legacy_backup {
                let _ = fs::copy(backup, path);
            }
        }
        return Err(HistoryPersistenceError::Storage);
    }
    Ok(())
}

/// 损坏恢复必须先丢弃未知目标；删除失败只返回稳定存储类别，不暴露路径。
pub fn clear_history_storage(path: &Path) -> Result<(), HistoryPersistenceError> {
    let temporary_path = path.with_extension("json.tmp");
    let mut failed = false;
    for candidate in [&temporary_path, path] {
        if let Err(error) = fs::remove_file(candidate) {
            if error.kind() != io::ErrorKind::NotFound {
                failed = true;
            }
        }
    }
    if failed {
        Err(HistoryPersistenceError::Storage)
    } else {
        Ok(())
    }
}

/// 清除当前账号时同步移除同账号的 v1 迁移备份，避免“永久删除”后仍可恢复旧样本。
pub fn clear_legacy_backup_for_current_account(
    path: &Path,
    history: &UsageHistory,
) -> Result<(), HistoryPersistenceError> {
    let Some(current) = history.account_fingerprint.as_deref() else {
        return Ok(());
    };
    let backup = path.with_extension("v1.backup.json");
    let contents = match fs::read(&backup) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(HistoryPersistenceError::Storage),
    };
    let legacy = serde_json::from_slice::<UsageHistoryV1>(&contents)
        .map_err(|_| HistoryPersistenceError::Storage)?;
    if legacy.account_fingerprint.as_deref() == Some(current) {
        fs::remove_file(backup).map_err(|_| HistoryPersistenceError::Storage)?;
    }
    Ok(())
}

/// schema v1 首次改写前保留一次原文件；后续保存失败时可恢复旧目标。
fn legacy_backup_for(path: &Path) -> Result<Option<std::path::PathBuf>, HistoryPersistenceError> {
    let Ok(contents) = fs::read(path) else {
        return Ok(None);
    };
    let is_v1 = serde_json::from_slice::<serde_json::Value>(&contents)
        .ok()
        .and_then(|value| {
            value
                .get("schemaVersion")
                .and_then(serde_json::Value::as_u64)
        })
        == Some(1);
    if !is_v1 {
        return Ok(None);
    }
    let backup = path.with_extension("v1.backup.json");
    if !backup.exists() {
        fs::write(&backup, contents).map_err(|_| HistoryPersistenceError::Storage)?;
        log::info!("本地趋势历史迁移备份已创建：schema=1。");
    }
    Ok(Some(backup))
}

fn recovered_history(status: HistoryStorageStatus) -> LoadedUsageHistory {
    LoadedUsageHistory {
        history: UsageHistory::new_random(),
        status,
        needs_rewrite: status != HistoryStorageStatus::Missing,
    }
}

fn valid_window_input(window: &HistoryWindowInput) -> bool {
    window.remaining_percent <= 100
        && !window.window_id.trim().is_empty()
        && window.window_id.len() <= MAX_WINDOW_ID_BYTES
        && window.window_seconds > 0
}

fn valid_generation_input(generation: &HistoryGenerationInput) -> bool {
    !generation.window_id.trim().is_empty()
        && generation.window_id.len() <= MAX_WINDOW_ID_BYTES
        && generation.window_seconds > 0
        && valid_generation_id(&generation.generation_id)
}

fn valid_generation_id(generation_id: &str) -> bool {
    decode_hex_exact(generation_id, GENERATION_ID_BYTES).is_some()
}

fn valid_stored_window_key(window_id: &str, window_seconds: i64) -> bool {
    decode_hex_exact(window_id, STREAM_KEY_BYTES).is_some() && window_seconds > 0
}

fn window_stream_key(salt_hex: &str, window_id: &str, window_seconds: i64) -> Option<String> {
    if window_id.trim().is_empty() || window_id.len() > MAX_WINDOW_ID_BYTES || window_seconds <= 0 {
        return None;
    }
    let salt = decode_hex_exact(salt_hex, SALT_BYTES)?;
    let mut hasher = Sha256::new();
    hasher.update(b"codex-usage-bar/window-stream/v1\0");
    hasher.update(salt);
    hasher.update(b"\0");
    hasher.update(window_seconds.to_be_bytes());
    hasher.update(b"\0");
    hasher.update(window_id.as_bytes());
    Some(encode_hex(&hasher.finalize()))
}

fn regression_slope(samples: &[&StoredUsageSample]) -> Option<f64> {
    let first_at = samples.first()?.sampled_at;
    let count = samples.len() as f64;
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut sum_xy = 0.0;
    let mut sum_x_squared = 0.0;
    for sample in samples {
        let x = (sample.sampled_at - first_at).num_milliseconds() as f64 / 1_000.0;
        let y = sample.remaining_percent as f64;
        sum_x += x;
        sum_y += y;
        sum_xy += x * y;
        sum_x_squared += x * x;
    }
    let denominator = count * sum_x_squared - sum_x * sum_x;
    if !denominator.is_finite() || denominator <= f64::EPSILON {
        return None;
    }
    let slope = (count * sum_xy - sum_x * sum_y) / denominator;
    slope.is_finite().then_some(slope)
}

fn round_to_quarter_hour(value: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let seconds = value.timestamp();
    let quotient = seconds.div_euclid(FORECAST_ROUND_SECONDS);
    let remainder = seconds.rem_euclid(FORECAST_ROUND_SECONDS);
    let rounded_quotient = if remainder >= FORECAST_ROUND_SECONDS / 2 {
        quotient.checked_add(1)?
    } else {
        quotient
    };
    let rounded_seconds = rounded_quotient.checked_mul(FORECAST_ROUND_SECONDS)?;
    Utc.timestamp_opt(rounded_seconds, 0).single()
}

fn ceil_to_quarter_hour(value: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let seconds = value.timestamp();
    let quotient = seconds.div_euclid(FORECAST_ROUND_SECONDS);
    let remainder = seconds.rem_euclid(FORECAST_ROUND_SECONDS);
    let ceiled_quotient = if remainder == 0 {
        quotient
    } else {
        quotient.checked_add(1)?
    };
    let ceiled_seconds = ceiled_quotient.checked_mul(FORECAST_ROUND_SECONDS)?;
    Utc.timestamp_opt(ceiled_seconds, 0).single()
}

fn downsample_cycle<'a>(
    samples: &[&'a StoredUsageSample],
    bucket_seconds: i64,
) -> Vec<&'a StoredUsageSample> {
    if samples.len() <= 2 {
        return samples.to_vec();
    }
    debug_assert!(bucket_seconds > 0);
    let first = samples[0];
    let mut selected = vec![first];
    let mut bucket = first.sampled_at.timestamp().div_euclid(bucket_seconds);
    let mut bucket_last = first;
    for sample in samples.iter().copied().skip(1) {
        let sample_bucket = sample.sampled_at.timestamp().div_euclid(bucket_seconds);
        if sample_bucket != bucket {
            if bucket_last.sampled_at != selected.last().unwrap().sampled_at {
                selected.push(bucket_last);
            }
            bucket = sample_bucket;
        }
        bucket_last = sample;
    }
    if bucket_last.sampled_at != selected.last().unwrap().sampled_at {
        selected.push(bucket_last);
    }
    selected
}

fn compact_cycle_for_age(
    samples: &[StoredUsageSample],
    daily_cutoff: DateTime<Utc>,
    medium_cutoff: DateTime<Utc>,
    recent_cutoff: DateTime<Utc>,
) -> Vec<StoredUsageSample> {
    if samples.len() <= 2 {
        return samples.to_vec();
    }

    let first_at = samples[0].sampled_at;
    let last_at = samples[samples.len() - 1].sampled_at;
    let mut selected = Vec::with_capacity(samples.len());
    let mut tier_start = 0;
    while tier_start < samples.len() {
        let sampled_at = samples[tier_start].sampled_at;
        let (tier_end, bucket_seconds) = if sampled_at < daily_cutoff {
            let tier_end = samples[tier_start..]
                .iter()
                .position(|sample| sample.sampled_at >= daily_cutoff)
                .map(|offset| tier_start + offset)
                .unwrap_or(samples.len());
            (tier_end, Some(DAILY_BUCKET_SECONDS))
        } else if sampled_at < medium_cutoff {
            let tier_end = samples[tier_start..]
                .iter()
                .position(|sample| sample.sampled_at >= medium_cutoff)
                .map(|offset| tier_start + offset)
                .unwrap_or(samples.len());
            (tier_end, Some(MONTH_BUCKET_SECONDS))
        } else if sampled_at < recent_cutoff {
            let tier_end = samples[tier_start..]
                .iter()
                .position(|sample| sample.sampled_at >= recent_cutoff)
                .map(|offset| tier_start + offset)
                .unwrap_or(samples.len());
            (tier_end, Some(SEVEN_DAY_BUCKET_SECONDS))
        } else {
            // 最新层不需要哨兵。直接消费余下样本，避免合法 MAX_UTC 样本让
            // tier_end == tier_start 而无法推进循环。
            (samples.len(), None)
        };
        if let Some(bucket_seconds) = bucket_seconds {
            let references = samples[tier_start..tier_end].iter().collect::<Vec<_>>();
            selected.extend(
                downsample_cycle(&references, bucket_seconds)
                    .into_iter()
                    .cloned(),
            );
        } else {
            selected.extend_from_slice(&samples[tier_start..tier_end]);
        }
        tier_start = tier_end;
    }

    // Tier 边界可能让同一点被相邻分段同时选中。排序去重后再次固定保留 cycle 首尾，
    // 这样任何压缩都不会丢掉重置前后的边界点。
    selected.sort_by_key(|sample| sample.sampled_at);
    selected.dedup_by_key(|sample| sample.sampled_at);
    if selected
        .first()
        .is_none_or(|sample| sample.sampled_at != first_at)
    {
        selected.insert(0, samples[0].clone());
    }
    if selected
        .last()
        .is_none_or(|sample| sample.sampled_at != last_at)
    {
        selected.push(samples[samples.len() - 1].clone());
    }
    selected
}

fn bound_query_points(points: Vec<UsageHistoryPoint>, maximum: usize) -> Vec<UsageHistoryPoint> {
    if points.len() <= maximum || maximum == 0 {
        return points;
    }

    let mut required = vec![false; points.len()];
    for (index, point) in points.iter().enumerate() {
        if point.break_before {
            required[index] = true;
            if index > 0 {
                required[index - 1] = true;
            }
        }
    }
    required[points.len() - 1] = true;
    let required_count = required.iter().filter(|required| **required).count();
    if required_count >= maximum {
        // 极端情况下 cycle 边界本身超过上限，优先保留最新 cycle。若该 cycle
        // 自身也超过上限，则在 cycle 内均匀采样，但固定保留首点、末点和断点。
        let mut newest_cycles = Vec::new();
        let mut index = points.len();
        let mut selected_len = 0;
        while index > 0 && selected_len < maximum {
            let cycle_start = points[..index]
                .iter()
                .rposition(|point| point.break_before)
                .unwrap_or(0);
            let cycle_len = index - cycle_start;
            if cycle_len > maximum && newest_cycles.is_empty() {
                return sample_cycle_points(&points[cycle_start..index], maximum);
            }
            if selected_len + cycle_len > maximum {
                break;
            }
            newest_cycles.push((cycle_start, index));
            selected_len += cycle_len;
            index = cycle_start;
        }
        let mut selected = Vec::with_capacity(selected_len);
        for (start, end) in newest_cycles.into_iter().rev() {
            selected.extend_from_slice(&points[start..end]);
        }
        selected.sort_by_key(|point| point.sampled_at);
        return selected;
    }

    let optional_budget = maximum - required_count;
    let optional_count = points.len() - required_count;
    let mut selected = Vec::with_capacity(maximum);
    let mut optional_seen = 0_usize;
    let mut optional_kept = 0_usize;
    for (index, point) in points.into_iter().enumerate() {
        if required[index] {
            selected.push(point);
            continue;
        }
        optional_seen += 1;
        let target_kept = optional_seen.saturating_mul(optional_budget) / optional_count;
        if target_kept > optional_kept {
            selected.push(point);
            optional_kept += 1;
        }
    }
    selected
}

fn sample_cycle_points(points: &[UsageHistoryPoint], maximum: usize) -> Vec<UsageHistoryPoint> {
    if maximum == 0 || points.is_empty() {
        return Vec::new();
    }
    if points.len() <= maximum {
        return points.to_vec();
    }
    if maximum == 1 {
        let mut last = points[points.len() - 1].clone();
        last.break_before = true;
        return vec![last];
    }

    let last_index = points.len() - 1;
    let mut selected = Vec::with_capacity(maximum);
    for output_index in 0..maximum {
        let source_index = output_index.saturating_mul(last_index) / (maximum - 1);
        let mut point = points[source_index].clone();
        point.break_before = output_index == 0;
        selected.push(point);
    }
    selected
}

fn safe_subtract(value: DateTime<Utc>, duration: Duration) -> DateTime<Utc> {
    value
        .checked_sub_signed(duration)
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_hex_exact(value: &str, expected_bytes: usize) -> Option<Vec<u8>> {
    if value.len() != expected_bytes * 2 || !value.is_ascii() {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_nibble(pair[0])?;
            let low = hex_nibble(pair[1])?;
            Some((high << 4) | low)
        })
        .collect()
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, time::SystemTime};

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 6, hour, minute, 0)
            .single()
            .unwrap()
    }

    fn salt() -> String {
        "11".repeat(SALT_BYTES)
    }

    fn history() -> UsageHistory {
        UsageHistory::with_salt(salt()).unwrap()
    }

    fn identity() -> AccountIdentity<'static> {
        AccountIdentity::AccountId("account-private")
    }

    fn window(
        id: &str,
        seconds: i64,
        remaining: u8,
        reset_at: Option<DateTime<Utc>>,
    ) -> HistoryWindowInput {
        HistoryWindowInput {
            window_id: id.to_owned(),
            window_seconds: seconds,
            cycle_reset_at: reset_at,
            remaining_percent: remaining,
        }
    }

    fn record(
        history: &mut UsageHistory,
        time: DateTime<Utc>,
        remaining: u8,
        reset_at: DateTime<Utc>,
    ) -> HistoryMutation {
        history
            .record_successful_snapshot(
                identity(),
                time,
                &[window("weekly", 604_800, remaining, Some(reset_at))],
            )
            .unwrap()
    }

    fn generation(id: &str) -> HistoryGenerationInput {
        HistoryGenerationInput {
            window_id: "weekly".to_owned(),
            window_seconds: 604_800,
            generation_id: id.to_owned(),
        }
    }

    fn record_with_generation(
        history: &mut UsageHistory,
        time: DateTime<Utc>,
        remaining: u8,
        reset_at: DateTime<Utc>,
        generation_id: &str,
    ) -> HistoryMutation {
        history
            .record_successful_snapshot_with_generations(
                identity(),
                time,
                &[window("weekly", 604_800, remaining, Some(reset_at))],
                &[generation(generation_id)],
            )
            .unwrap()
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("codex-usage-history-{name}-{nonce}.json"))
    }

    #[test]
    fn fingerprints_are_salted_domain_separated_and_redacted_in_debug() {
        let first = account_fingerprint(&salt(), AccountIdentity::AccountId("same")).unwrap();
        let second =
            account_fingerprint(&"22".repeat(SALT_BYTES), AccountIdentity::AccountId("same"))
                .unwrap();
        let fallback = account_fingerprint(&salt(), AccountIdentity::Token("same")).unwrap();
        let selected_account = account_fingerprint(
            &salt(),
            AccountIdentity::from_parts(Some("same"), "ignored-token"),
        )
        .unwrap();
        let selected_fallback =
            account_fingerprint(&salt(), AccountIdentity::from_parts(None, "same")).unwrap();
        assert_eq!(first.len(), FINGERPRINT_BYTES * 2);
        assert_ne!(first, second);
        assert_ne!(first, fallback);
        assert_eq!(selected_account, first);
        assert_eq!(selected_fallback, fallback);
        assert_eq!(
            format!("{:?}", AccountIdentity::Token("secret-token")),
            "Token(\"[redacted]\")"
        );
        assert!(matches!(
            account_fingerprint("bad", identity()),
            Err(FingerprintError::InvalidSalt)
        ));
        assert!(matches!(
            account_fingerprint(&salt(), AccountIdentity::Token("  ")),
            Err(FingerprintError::EmptyIdentity)
        ));
    }

    #[test]
    fn schema_round_trip_contains_only_allowed_history_fields() {
        let mut value = history();
        let reset = at(20, 0);
        value
            .record_successful_snapshot(
                identity(),
                at(8, 0),
                &[window(
                    "private-account@example.com",
                    604_800,
                    91,
                    Some(reset),
                )],
            )
            .unwrap();
        let path = temp_path("roundtrip");
        save_history(&path, &value).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("schemaVersion"));
        assert!(text.contains("accountFingerprint"));
        assert!(text.contains("windowId"));
        assert!(text.contains("remainingPercent"));
        for private in [
            "account-private",
            "private-account@example.com",
            "secret-token",
            "email",
            "label",
            "proxy",
            "auth.json",
            "rawResponse",
        ] {
            assert!(!text.contains(private));
        }
        let loaded = load_history_at(&path, at(9, 0));
        assert_eq!(loaded.status, HistoryStorageStatus::Ready);
        assert_eq!(loaded.history, value);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn corrupt_and_unknown_files_recover_empty_without_blocking() {
        let corrupt = temp_path("corrupt");
        fs::write(&corrupt, b"{not json").unwrap();
        let recovered = load_history_at(&corrupt, at(9, 0));
        assert_eq!(recovered.status, HistoryStorageStatus::RecoveredCorrupt);
        assert!(recovered.history.is_empty());
        assert!(recovered.needs_rewrite);
        let _ = fs::remove_file(corrupt);

        let unknown = temp_path("unknown");
        fs::write(
            &unknown,
            br#"{"schemaVersion":999,"token":"must-not-survive"}"#,
        )
        .unwrap();
        let recovered = load_history_at(&unknown, at(9, 0));
        assert_eq!(recovered.status, HistoryStorageStatus::RecoveredUnsupported);
        assert!(recovered.history.is_empty());
        let serialized = serde_json::to_string(&recovered.history).unwrap();
        assert!(!serialized.contains("must-not-survive"));
        let _ = fs::remove_file(unknown);
    }

    #[test]
    fn explicit_storage_clear_removes_target_and_temporary_files() {
        let path = temp_path("clear-storage");
        let temporary_path = path.with_extension("json.tmp");
        fs::write(&path, b"old-account-history").unwrap();
        fs::write(&temporary_path, b"stale-temporary-history").unwrap();

        clear_history_storage(&path).unwrap();
        assert!(!path.exists());
        assert!(!temporary_path.exists());
        // Missing files are already in the desired privacy-safe state.
        clear_history_storage(&path).unwrap();
    }

    #[test]
    fn missing_file_starts_with_a_valid_random_salt() {
        let path = temp_path("missing");
        let first = load_history_at(&path, at(9, 0));
        let second = load_history_at(&path, at(9, 0));
        assert_eq!(first.status, HistoryStorageStatus::Missing);
        assert!(!first.needs_rewrite);
        assert_ne!(first.history.salt, second.history.salt);
        assert!(decode_hex_exact(&first.history.salt, SALT_BYTES).is_some());
    }

    #[test]
    fn account_switch_restores_each_anonymous_partition() {
        let mut value = history();
        record(&mut value, at(8, 0), 80, at(20, 0));
        assert_eq!(value.sample_count(), 1);
        let selection = value
            .select_account(AccountIdentity::AccountId("another-account"))
            .unwrap();
        assert_eq!(selection, AccountSelection::Switched { created: true });
        assert!(value.is_empty());
        value
            .record_successful_snapshot(
                AccountIdentity::AccountId("another-account"),
                at(9, 0),
                &[window("weekly", 604_800, 60, Some(at(20, 0)))],
            )
            .unwrap();

        assert_eq!(
            value.select_account(identity()).unwrap(),
            AccountSelection::Switched { created: false }
        );
        assert_eq!(value.sample_count(), 1);
        assert_eq!(value.summary().latest_sample_at, Some(at(8, 0)));

        let serialized = serde_json::to_string(&value).unwrap();
        assert!(!serialized.contains("account-private"));
        assert!(!serialized.contains("another-account"));
        let mut restored: UsageHistory = serde_json::from_str(&serialized).unwrap();
        restored
            .select_account(AccountIdentity::AccountId("another-account"))
            .unwrap();
        assert_eq!(restored.summary().latest_sample_at, Some(at(9, 0)));
    }

    #[test]
    fn samples_on_interval_change_new_window_and_new_cycle() {
        let mut value = history();
        let reset = at(20, 0);
        assert_eq!(record(&mut value, at(8, 0), 80, reset).samples_recorded, 1);
        // 未变化且不足五分钟。
        assert_eq!(record(&mut value, at(8, 4), 80, reset).samples_recorded, 0);
        // 百分比变化绕过节流。
        assert_eq!(record(&mut value, at(8, 4), 79, reset).samples_recorded, 1);
        // 未变化但满五分钟。
        assert_eq!(record(&mut value, at(8, 9), 79, reset).samples_recorded, 1);

        let second = window("short", 18_000, 60, Some(at(12, 0)));
        assert_eq!(
            value
                .record_successful_snapshot(identity(), at(8, 10), &[second])
                .unwrap()
                .samples_recorded,
            1
        );

        let next_reset = reset + Duration::days(7);
        assert_eq!(
            record(&mut value, reset + Duration::minutes(1), 100, next_reset).samples_recorded,
            1
        );
        let weekly_key = window_stream_key(&value.salt, "weekly", 604_800).unwrap();
        let weekly = value
            .streams
            .iter()
            .find(|stream| stream.window_id == weekly_key)
            .unwrap();
        assert_eq!(weekly.cycles.len(), 2);
    }

    #[test]
    fn generation_change_splits_same_reset_at_recovery_from_98_or_99_to_100() {
        for (index, previous) in [98, 99].into_iter().enumerate() {
            let mut value = history();
            let reset = at(20, 0);
            let previous_generation = "aa".repeat(GENERATION_ID_BYTES);
            let next_generation = if index == 0 {
                "bb".repeat(GENERATION_ID_BYTES)
            } else {
                "cc".repeat(GENERATION_ID_BYTES)
            };
            record_with_generation(&mut value, at(8, 0), previous, reset, &previous_generation);
            record_with_generation(&mut value, at(8, 1), 100, reset, &next_generation);

            assert_eq!(value.streams[0].cycles.len(), 2);
            let query = value
                .query_request(
                    UsageHistoryRequest::Preset {
                        preset: UsageHistoryPreset::Hours24,
                    },
                    at(8, 2),
                )
                .unwrap();
            assert_eq!(query.series[0].points.len(), 2);
            assert!(query.series[0].points[0].break_before);
            assert!(query.series[0].points[1].break_before);
        }
    }

    #[test]
    fn same_generation_ignores_percent_and_reset_at_noise_for_segmentation() {
        let mut value = history();
        let generation_id = "aa".repeat(GENERATION_ID_BYTES);
        let reset = at(20, 0);
        record_with_generation(&mut value, at(8, 0), 99, reset, &generation_id);

        // 同代次中的 99→100 抖动与截止时间校正都只能增加采样点，不能伪造新周期。
        record_with_generation(
            &mut value,
            at(8, 1),
            100,
            reset + Duration::hours(2),
            &generation_id,
        );
        record_with_generation(&mut value, at(8, 2), 99, reset, &generation_id);
        assert_eq!(value.streams[0].cycles.len(), 1);
        assert_eq!(value.streams[0].cycles[0].samples.len(), 3);
    }

    #[test]
    fn first_generation_adopts_legacy_cycle_without_an_upgrade_breakpoint() {
        let mut value = history();
        let reset = at(20, 0);
        record(&mut value, at(8, 0), 80, reset);
        let generation_id = "aa".repeat(GENERATION_ID_BYTES);
        let mutation = record_with_generation(&mut value, at(8, 1), 80, reset, &generation_id);

        assert_eq!(value.streams[0].cycles.len(), 1);
        assert_eq!(value.streams[0].cycles[0].samples.len(), 1);
        assert_eq!(mutation.samples_recorded, 0);
        assert!(mutation.generation_metadata_updated);
        assert!(mutation.changed());
        assert_eq!(
            value.streams[0].cycles[0].generation_id.as_deref(),
            Some(generation_id.as_str())
        );
    }

    #[test]
    fn generation_is_persisted_but_never_exposed_by_trend_query() {
        let mut value = history();
        let generation_id = "aa".repeat(GENERATION_ID_BYTES);
        record_with_generation(&mut value, at(8, 0), 99, at(20, 0), &generation_id);
        let path = temp_path("generation-roundtrip");
        save_history(&path, &value).unwrap();
        let stored_json = fs::read_to_string(&path).unwrap();
        assert!(stored_json.contains("generationId"));
        assert!(stored_json.contains(&generation_id));

        let loaded = load_history_at(&path, at(9, 0));
        assert_eq!(loaded.status, HistoryStorageStatus::Ready);
        assert_eq!(loaded.history, value);
        let query_json = serde_json::to_string(
            &loaded
                .history
                .query_request(
                    UsageHistoryRequest::Preset {
                        preset: UsageHistoryPreset::Hours24,
                    },
                    at(9, 0),
                )
                .unwrap(),
        )
        .unwrap();
        assert!(!query_json.contains("generationId"));
        assert!(!query_json.contains(&generation_id));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn forward_reset_correction_before_expiry_does_not_split_a_cycle() {
        let mut value = history();
        let original_reset = at(20, 0);
        record(&mut value, at(8, 0), 80, original_reset);

        let corrected_reset = original_reset + Duration::hours(2);
        record(&mut value, at(9, 0), 79, corrected_reset);
        assert_eq!(value.streams[0].cycles.len(), 1);
        assert_eq!(value.streams[0].cycles[0].reset_at, Some(corrected_reset));

        let next_reset = corrected_reset + Duration::days(7);
        record(
            &mut value,
            corrected_reset + Duration::minutes(1),
            100,
            next_reset,
        );
        assert_eq!(value.streams[0].cycles.len(), 2);
    }

    #[test]
    fn percentage_rise_segments_a_cycle_when_reset_timestamp_is_missing() {
        let mut value = history();
        value
            .record_successful_snapshot(
                identity(),
                at(8, 0),
                &[window("unknown", 86_400, 10, None)],
            )
            .unwrap();
        value
            .record_successful_snapshot(
                identity(),
                at(8, 1),
                &[window("unknown", 86_400, 99, None)],
            )
            .unwrap();
        assert_eq!(value.streams[0].cycles.len(), 2);
        let query = value
            .query_request(
                UsageHistoryRequest::Preset {
                    preset: UsageHistoryPreset::Hours24,
                },
                at(8, 2),
            )
            .unwrap();
        assert!(query.series[0].points[0].break_before);
        assert!(query.series[0].points[1].break_before);
    }

    #[test]
    fn duplicate_windows_invalid_values_and_clock_rollback_are_ignored() {
        let mut value = history();
        let reset = at(20, 0);
        record(&mut value, at(8, 0), 80, reset);
        let duplicate = window("weekly", 604_800, 79, Some(reset));
        let invalid = window("", 0, 101, None);
        let mutation = value
            .record_successful_snapshot(
                identity(),
                at(8, 1),
                &[duplicate.clone(), duplicate, invalid],
            )
            .unwrap();
        assert_eq!(mutation.samples_recorded, 1);
        assert_eq!(mutation.ignored_windows, 2);
        assert_eq!(record(&mut value, at(7, 59), 70, reset).samples_recorded, 0);
    }
    #[test]
    fn permanent_history_keeps_old_samples_and_unbounded_streams() {
        let now = at(8, 0);
        let mut value = history();
        let reset = now + Duration::days(7);
        record(&mut value, now - Duration::days(400), 80, reset);
        for index in 0..20 {
            value
                .record_successful_snapshot(
                    identity(),
                    now + Duration::minutes(index),
                    &[window(
                        &format!("window-{index}"),
                        60 + index,
                        90,
                        Some(reset),
                    )],
                )
                .unwrap();
        }

        value.compact_at(now);

        assert_eq!(value.stream_count(), 21);
        assert!(value
            .all_samples()
            .any(|sample| sample.sampled_at == now - Duration::days(400)));
        let all = value
            .query_request(
                UsageHistoryRequest::Preset {
                    preset: UsageHistoryPreset::All,
                },
                now,
            )
            .unwrap();
        assert_eq!(all.applied_start_at, now - Duration::days(400));
        assert_eq!(all.bucket_seconds, Some(DAILY_BUCKET_SECONDS));
    }
    #[test]
    fn seven_day_query_downsamples_and_never_exposes_internal_metadata() {
        let mut value = history();
        let reset = at(20, 0) + Duration::days(7);
        for minute in 0..180 {
            record(
                &mut value,
                at(8, 0) + Duration::minutes(minute),
                100 - (minute / 10) as u8,
                reset,
            );
        }
        let full = value
            .query_request(
                UsageHistoryRequest::Preset {
                    preset: UsageHistoryPreset::Hours24,
                },
                at(11, 0),
            )
            .unwrap();
        let sampled = value
            .query_request(
                UsageHistoryRequest::Preset {
                    preset: UsageHistoryPreset::Days7,
                },
                at(11, 0),
            )
            .unwrap();
        assert!(sampled.series[0].points.len() < full.series[0].points.len());
        assert!(sampled.series[0].points[0].break_before);
        let json = serde_json::to_string(&sampled).unwrap();
        assert!(!json.contains("accountFingerprint"));
        assert!(!json.contains("salt"));
        assert!(!json.contains("resetAt"));
        assert!(!json.contains("label"));
    }

    #[test]
    fn query_output_is_bounded_and_preserves_every_included_cycle_boundary() {
        let now = at(12, 0);
        let start = now - Duration::hours(20);
        let mut cycles = Vec::new();
        let mut expected_boundaries = Vec::new();
        for cycle_index in 0..4 {
            let cycle_start = start + Duration::minutes((cycle_index * 300) as i64);
            let samples = (0..300)
                .map(|sample_index| StoredUsageSample {
                    sampled_at: cycle_start + Duration::minutes(sample_index as i64),
                    remaining_percent: (sample_index % 101) as u8,
                })
                .collect::<Vec<_>>();
            expected_boundaries.push((
                samples.first().unwrap().sampled_at,
                samples.last().unwrap().sampled_at,
            ));
            cycles.push(StoredUsageCycle {
                generation_id: None,
                reset_at: None,
                samples,
            });
        }
        let mut value = history();
        value.select_account(identity()).unwrap();
        value.streams.push(StoredUsageStream {
            window_id: window_stream_key(&value.salt, "weekly", 604_800).unwrap(),
            window_seconds: 604_800,
            cycles,
        });

        let query = value
            .query_request(
                UsageHistoryRequest::Preset {
                    preset: UsageHistoryPreset::Hours24,
                },
                now,
            )
            .unwrap();
        let points = &query.series[0].points;
        assert!(points.len() <= MAX_QUERY_POINTS_PER_SERIES);
        for (first_at, last_at) in expected_boundaries {
            let first = points
                .iter()
                .find(|point| point.sampled_at == first_at)
                .expect("cycle first point must survive query bounding");
            assert!(first.break_before);
            assert!(points.iter().any(|point| point.sampled_at == last_at));
        }
    }

    #[test]
    fn query_bound_never_returns_empty_when_required_boundaries_exceed_the_cap() {
        let start = at(8, 0) - Duration::hours(1);
        let points = (0..MAX_QUERY_POINTS_PER_SERIES + 1)
            .flat_map(|cycle_index| {
                let first_at = start + Duration::seconds((cycle_index * 2) as i64);
                [
                    UsageHistoryPoint {
                        sampled_at: first_at,
                        remaining_percent: 100,
                        break_before: true,
                    },
                    UsageHistoryPoint {
                        sampled_at: first_at + Duration::seconds(1),
                        remaining_percent: 99,
                        break_before: false,
                    },
                ]
            })
            .collect::<Vec<_>>();
        let latest_at = points.last().unwrap().sampled_at;

        let bounded = bound_query_points(points, MAX_QUERY_POINTS_PER_SERIES);

        assert!(!bounded.is_empty());
        assert!(bounded.len() <= MAX_QUERY_POINTS_PER_SERIES);
        assert!(bounded[0].break_before);
        assert_eq!(bounded.last().unwrap().sampled_at, latest_at);
    }

    #[test]
    fn query_bound_safely_samples_a_latest_cycle_larger_than_the_cap() {
        let start = at(8, 0) - Duration::hours(1);
        let points = (0..MAX_QUERY_POINTS_PER_SERIES + 500)
            .map(|index| UsageHistoryPoint {
                sampled_at: start + Duration::seconds(index as i64),
                remaining_percent: (index % 101) as u8,
                break_before: index == 0,
            })
            .collect::<Vec<_>>();
        let first_at = points.first().unwrap().sampled_at;
        let latest_at = points.last().unwrap().sampled_at;

        let bounded = bound_query_points(points, MAX_QUERY_POINTS_PER_SERIES);

        assert_eq!(bounded.len(), MAX_QUERY_POINTS_PER_SERIES);
        assert!(bounded[0].break_before);
        assert_eq!(bounded[0].sampled_at, first_at);
        assert_eq!(bounded.last().unwrap().sampled_at, latest_at);
    }

    #[test]
    fn query_bound_samples_latest_large_cycle_when_old_boundaries_reach_the_cap() {
        let start = at(8, 0) - Duration::hours(1);
        let mut points = (0..500)
            .flat_map(|cycle_index| {
                let first_at = start + Duration::seconds((cycle_index * 2) as i64);
                [
                    UsageHistoryPoint {
                        sampled_at: first_at,
                        remaining_percent: 1,
                        break_before: true,
                    },
                    UsageHistoryPoint {
                        sampled_at: first_at + Duration::seconds(1),
                        remaining_percent: 0,
                        break_before: false,
                    },
                ]
            })
            .collect::<Vec<_>>();
        let latest_cycle_start = start + Duration::seconds(1_000);
        points.extend((0..1_500).map(|index| UsageHistoryPoint {
            sampled_at: latest_cycle_start + Duration::seconds(index as i64),
            remaining_percent: (index % 101) as u8,
            break_before: index == 0,
        }));
        let latest_at = points.last().unwrap().sampled_at;

        let bounded = bound_query_points(points, MAX_QUERY_POINTS_PER_SERIES);

        assert_eq!(bounded.len(), MAX_QUERY_POINTS_PER_SERIES);
        assert!(bounded[0].break_before);
        assert_eq!(bounded[0].sampled_at, latest_cycle_start);
        assert_eq!(bounded.last().unwrap().sampled_at, latest_at);
        assert!(bounded
            .windows(2)
            .all(|window| window[0].sampled_at < window[1].sampled_at));
    }

    #[test]
    fn structured_requests_deserialize_exactly_and_presets_use_a_half_open_snapshot() {
        let now = at(12, 0);
        let preset: UsageHistoryRequest =
            serde_json::from_str(r#"{"kind":"preset","preset":"30d"}"#).unwrap();
        assert_eq!(
            preset,
            UsageHistoryRequest::Preset {
                preset: UsageHistoryPreset::Days30,
            }
        );
        let resolved = preset.resolve(now, None).unwrap();
        assert_eq!(resolved.start_at, now - Duration::days(30));
        assert_eq!(resolved.end_at_exclusive, now);
        assert_eq!(resolved.bucket_seconds, Some(60 * 60));
        assert!(!resolved.truncated_by_retention);

        let all: UsageHistoryRequest =
            serde_json::from_str(r#"{"kind":"preset","preset":"all"}"#).unwrap();
        let earliest = now - Duration::days(400);
        let resolved_all = all.resolve(now, Some(earliest)).unwrap();
        assert_eq!(resolved_all.start_at, earliest);
        assert_eq!(resolved_all.end_at_exclusive, now);
        assert_eq!(resolved_all.bucket_seconds, Some(DAILY_BUCKET_SECONDS));

        let custom: UsageHistoryRequest = serde_json::from_str(
            r#"{"kind":"custom","startAt":"2026-08-06T08:00:00Z","endAtExclusive":"2026-08-06T09:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(
            custom,
            UsageHistoryRequest::Custom {
                start_at: at(8, 0),
                end_at_exclusive: at(9, 0),
            }
        );

        let mut value = history();
        let reset = now + Duration::days(7);
        record(&mut value, now - Duration::minutes(1), 80, reset);
        record(&mut value, now, 79, reset);
        let query = value
            .query_request(
                UsageHistoryRequest::Preset {
                    preset: UsageHistoryPreset::Hours24,
                },
                now,
            )
            .unwrap();
        assert_eq!(query.series[0].points.len(), 1);
        assert_eq!(
            query.series[0].points[0].sampled_at,
            now - Duration::minutes(1)
        );
        assert_eq!(query.series[0].current_remaining_percent, 79);
        assert_eq!(query.applied_end_at_exclusive, now);
    }

    #[test]
    fn custom_bucket_selection_changes_only_after_exact_tier_boundaries() {
        let now = at(12, 0);
        let bucket_for = |span| {
            UsageHistoryRequest::Custom {
                start_at: now - span,
                end_at_exclusive: now,
            }
            .resolve(now, None)
            .unwrap()
            .bucket_seconds
        };

        assert_eq!(bucket_for(Duration::hours(24)), None);
        assert_eq!(
            bucket_for(Duration::hours(24) + Duration::nanoseconds(1)),
            Some(SEVEN_DAY_BUCKET_SECONDS)
        );
        assert_eq!(
            bucket_for(Duration::days(7)),
            Some(SEVEN_DAY_BUCKET_SECONDS)
        );
        assert_eq!(
            bucket_for(Duration::days(7) + Duration::nanoseconds(1)),
            Some(MONTH_BUCKET_SECONDS)
        );
        assert_eq!(
            bucket_for(Duration::days(32) + Duration::nanoseconds(1)),
            Some(DAILY_BUCKET_SECONDS)
        );
    }

    #[test]
    fn custom_request_validation_is_defensive_but_allows_a_dst_safe_span() {
        let now = at(12, 0);
        let resolve = |start_at, end_at_exclusive| {
            UsageHistoryRequest::Custom {
                start_at,
                end_at_exclusive,
            }
            .resolve(now, None)
        };

        assert_eq!(
            resolve(now, now),
            Err(UsageHistoryQueryError::EmptyOrReversedRange)
        );
        assert_eq!(
            resolve(now, now - Duration::seconds(1)),
            Err(UsageHistoryQueryError::EmptyOrReversedRange)
        );
        assert_eq!(
            resolve(now - Duration::hours(1), now + Duration::nanoseconds(1)),
            Err(UsageHistoryQueryError::FutureEnd)
        );
        assert!(resolve(now - Duration::days(400), now).is_ok());
    }

    #[test]
    fn old_custom_range_is_not_truncated_by_retention() {
        let now = at(12, 0);
        let end_at_exclusive = now - Duration::days(32) - Duration::hours(1);
        let request = UsageHistoryRequest::Custom {
            start_at: end_at_exclusive - Duration::days(1),
            end_at_exclusive,
        };
        let requested_start = end_at_exclusive - Duration::days(1);
        let resolved = request.resolve(now, None).unwrap();
        assert_eq!(resolved.start_at, requested_start);
        assert_eq!(resolved.end_at_exclusive, end_at_exclusive);
        assert!(!resolved.truncated_by_retention);

        let query = history().query_request(request, now).unwrap();
        assert_eq!(query.applied_start_at, requested_start);
        assert_eq!(query.sample_count, 0);
        assert!(query.series.is_empty());
        assert!(!query.truncated_by_retention);
    }

    #[test]
    fn custom_query_uses_half_open_bounds_and_keeps_current_value_global() {
        let mut value = history();
        let now = at(12, 0);
        let reset = now + Duration::days(7);
        record(&mut value, now - Duration::hours(3), 90, reset);
        record(&mut value, now - Duration::hours(2), 80, reset);
        record(&mut value, now - Duration::hours(1), 70, reset);

        let query = value
            .query_request(
                UsageHistoryRequest::Custom {
                    start_at: now - Duration::hours(3),
                    end_at_exclusive: now - Duration::hours(1),
                },
                now,
            )
            .unwrap();
        assert_eq!(query.series[0].points.len(), 2);
        assert_eq!(
            query.series[0].points[0].sampled_at,
            now - Duration::hours(3)
        );
        assert_eq!(
            query.series[0].points[1].sampled_at,
            now - Duration::hours(2)
        );
        assert_eq!(query.series[0].current_remaining_percent, 70);
        assert_eq!(query.sample_count, 2);
        assert_eq!(query.earliest_sample_at, Some(now - Duration::hours(3)));
        assert_eq!(query.latest_sample_at, Some(now - Duration::hours(2)));
        assert_eq!(query.available_start_at, Some(now - Duration::hours(3)));
        assert_eq!(query.available_end_at, Some(now - Duration::hours(1)));
    }

    #[test]
    fn available_summary_includes_now_but_excludes_future_samples() {
        let mut value = history();
        let now = at(12, 0);
        let reset = now + Duration::days(7);
        record(&mut value, now - Duration::minutes(1), 80, reset);
        record(&mut value, now, 79, reset);
        record(&mut value, now + Duration::minutes(1), 78, reset);

        let query = value
            .query_request(
                UsageHistoryRequest::Preset {
                    preset: UsageHistoryPreset::Hours24,
                },
                now,
            )
            .unwrap();

        assert_eq!(query.available_start_at, Some(now - Duration::minutes(1)));
        assert_eq!(query.available_end_at, Some(now));
        assert_eq!(query.series[0].current_remaining_percent, 79);
        assert_eq!(query.latest_sample_at, Some(now - Duration::minutes(1)));
    }

    #[test]
    fn available_summary_includes_samples_older_than_thirty_two_days() {
        let sampled_at = at(12, 0);
        let now = sampled_at + Duration::days(33);
        let mut value = history();
        record(&mut value, sampled_at, 80, sampled_at + Duration::days(7));

        let query = value
            .query_request(
                UsageHistoryRequest::Preset {
                    preset: UsageHistoryPreset::Hours24,
                },
                now,
            )
            .unwrap();

        assert_eq!(query.sample_count, 0);
        assert!(query.series.is_empty());
        assert_eq!(query.available_start_at, Some(sampled_at));
        assert_eq!(query.available_end_at, Some(sampled_at));
    }

    #[test]
    fn historical_custom_query_keeps_forecast_based_on_the_global_latest_cycle() {
        let mut value = history();
        let now = at(12, 0);
        let old_reset = now - Duration::hours(1);
        for (hours_ago, remaining) in [(10, 90), (9, 80), (8, 70), (7, 60)] {
            record(
                &mut value,
                now - Duration::hours(hours_ago),
                remaining,
                old_reset,
            );
        }
        let current_reset = now + Duration::hours(12);
        for (minutes_ago, remaining) in [(60, 80), (40, 70), (20, 60), (0, 50)] {
            record(
                &mut value,
                now - Duration::minutes(minutes_ago),
                remaining,
                current_reset,
            );
        }
        let expected_forecast = value.forecast_for("weekly", 604_800, now).unwrap();

        let query = value
            .query_request(
                UsageHistoryRequest::Custom {
                    start_at: now - Duration::hours(11),
                    end_at_exclusive: now - Duration::hours(6),
                },
                now,
            )
            .unwrap();

        assert_eq!(query.series[0].current_remaining_percent, 50);
        assert_eq!(query.series[0].forecast, expected_forecast);
        assert!(query.series[0]
            .points
            .iter()
            .all(|point| point.sampled_at < now - Duration::hours(6)));
    }

    #[test]
    fn compacts_four_age_tiers_without_deleting_old_cycle_boundaries() {
        let now = at(12, 0);
        let daily_first = now - Duration::days(400) + Duration::hours(1);
        let hourly_first = now - Duration::days(10) + Duration::minutes(1);
        let medium_first = now - Duration::days(2) + Duration::minutes(1);
        let recent_first = now - Duration::hours(2);
        let samples = |first: DateTime<Utc>, step: Duration| {
            vec![
                StoredUsageSample {
                    sampled_at: first,
                    remaining_percent: 100,
                },
                StoredUsageSample {
                    sampled_at: first + step,
                    remaining_percent: 99,
                },
                StoredUsageSample {
                    sampled_at: first + step + step,
                    remaining_percent: 98,
                },
            ]
        };

        let mut value = history();
        value.select_account(identity()).unwrap();
        value.streams.push(StoredUsageStream {
            window_id: window_stream_key(&value.salt, "weekly", 604_800).unwrap(),
            window_seconds: 604_800,
            cycles: vec![
                StoredUsageCycle {
                    generation_id: None,
                    reset_at: None,
                    samples: samples(daily_first, Duration::hours(1)),
                },
                StoredUsageCycle {
                    generation_id: None,
                    reset_at: None,
                    samples: samples(hourly_first, Duration::minutes(10)),
                },
                StoredUsageCycle {
                    generation_id: None,
                    reset_at: None,
                    samples: samples(medium_first, Duration::minutes(3)),
                },
                StoredUsageCycle {
                    generation_id: None,
                    reset_at: None,
                    samples: samples(recent_first, Duration::minutes(1)),
                },
            ],
        });

        assert_eq!(value.compact_at(now), 3);
        let cycles = &value.streams[0].cycles;
        assert_eq!(cycles[0].samples.len(), 2);
        assert_eq!(cycles[0].samples[0].sampled_at, daily_first);
        assert_eq!(
            cycles[0].samples[1].sampled_at,
            daily_first + Duration::hours(2)
        );
        assert_eq!(cycles[1].samples.len(), 2);
        assert_eq!(cycles[2].samples.len(), 2);
        assert_eq!(cycles[3].samples.len(), 3);
    }

    #[test]
    fn age_compaction_is_idempotent_at_exact_tier_boundaries() {
        let now = at(12, 0);
        let daily_cutoff = now - Duration::days(32);
        let medium_cutoff = now - Duration::days(7);
        let recent_cutoff = now - Duration::hours(24);
        let sample_at = |sampled_at, remaining_percent| StoredUsageSample {
            sampled_at,
            remaining_percent,
        };
        let samples = vec![
            sample_at(medium_cutoff - Duration::minutes(50), 100),
            sample_at(medium_cutoff - Duration::minutes(40), 99),
            sample_at(medium_cutoff - Duration::minutes(30), 98),
            sample_at(medium_cutoff, 97),
            sample_at(medium_cutoff + Duration::minutes(1), 96),
            sample_at(medium_cutoff + Duration::minutes(2), 95),
            sample_at(recent_cutoff - Duration::minutes(10), 94),
            sample_at(recent_cutoff - Duration::minutes(9), 93),
            sample_at(recent_cutoff - Duration::minutes(8), 92),
            sample_at(recent_cutoff, 91),
            sample_at(recent_cutoff + Duration::minutes(1), 90),
            sample_at(recent_cutoff + Duration::minutes(2), 89),
        ];

        let once = compact_cycle_for_age(&samples, daily_cutoff, medium_cutoff, recent_cutoff);
        let twice = compact_cycle_for_age(&once, daily_cutoff, medium_cutoff, recent_cutoff);

        assert!(once.len() < samples.len());
        assert_eq!(once, twice);
        assert!(once.iter().any(|sample| sample.sampled_at == medium_cutoff));
        assert!(once.iter().any(|sample| sample.sampled_at == recent_cutoff));
    }

    #[test]
    fn age_compaction_keeps_reset_cycles_separate_inside_one_bucket() {
        let now = at(12, 0);
        let first_start = now - Duration::days(10) + Duration::minutes(1);
        let second_start = first_start + Duration::minutes(5);
        let mut stream = StoredUsageStream {
            window_id: "bounded".to_owned(),
            window_seconds: 604_800,
            cycles: vec![
                StoredUsageCycle {
                    generation_id: None,
                    reset_at: None,
                    samples: vec![
                        StoredUsageSample {
                            sampled_at: first_start,
                            remaining_percent: 10,
                        },
                        StoredUsageSample {
                            sampled_at: first_start + Duration::minutes(1),
                            remaining_percent: 9,
                        },
                        StoredUsageSample {
                            sampled_at: first_start + Duration::minutes(2),
                            remaining_percent: 0,
                        },
                    ],
                },
                StoredUsageCycle {
                    generation_id: None,
                    reset_at: None,
                    samples: vec![
                        StoredUsageSample {
                            sampled_at: second_start,
                            remaining_percent: 100,
                        },
                        StoredUsageSample {
                            sampled_at: second_start + Duration::minutes(1),
                            remaining_percent: 99,
                        },
                        StoredUsageSample {
                            sampled_at: second_start + Duration::minutes(2),
                            remaining_percent: 98,
                        },
                    ],
                },
            ],
        };

        stream.compact_for_age(now);
        let points = stream.query_points(now - Duration::days(30), now, Some(MONTH_BUCKET_SECONDS));

        assert_eq!(stream.cycles.len(), 2);
        assert_eq!(points.len(), 4);
        assert!(points[0].break_before);
        assert!(points[2].break_before);
        assert_eq!(stream.cycles[0].samples.len(), 2);
        assert_eq!(stream.cycles[1].samples.len(), 2);
    }

    #[test]
    fn recent_compaction_consumes_a_max_utc_sample_without_a_sentinel_loop() {
        let now = at(12, 0);
        let recent_cutoff = now - Duration::hours(24);
        let samples = vec![
            StoredUsageSample {
                sampled_at: recent_cutoff,
                remaining_percent: 100,
            },
            StoredUsageSample {
                sampled_at: recent_cutoff + Duration::seconds(1),
                remaining_percent: 99,
            },
            StoredUsageSample {
                sampled_at: DateTime::<Utc>::MAX_UTC,
                remaining_percent: 98,
            },
        ];

        let compacted = compact_cycle_for_age(
            &samples,
            now - Duration::days(32),
            now - Duration::days(7),
            recent_cutoff,
        );

        assert_eq!(compacted, samples);
    }

    #[test]
    fn schema_v1_migrates_without_data_loss_and_keeps_a_backup() {
        let now = at(12, 0);
        let mut value = history();
        let reset = now + Duration::days(7);
        record(&mut value, now - Duration::days(6), 90, reset);
        record(&mut value, now - Duration::days(1), 80, reset);
        let path = temp_path("schema-v1-upgrade");
        let mut legacy = serde_json::to_value(&value).unwrap();
        let object = legacy.as_object_mut().unwrap();
        object.insert("schemaVersion".to_owned(), serde_json::Value::from(1));
        object.remove("accounts");
        fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();

        let loaded = load_history_at(&path, now);
        assert_eq!(loaded.status, HistoryStorageStatus::Ready);
        assert_eq!(loaded.history.schema_version, USAGE_HISTORY_SCHEMA_VERSION);
        assert_eq!(loaded.history.sample_count(), 2);
        assert!(loaded.needs_rewrite);

        save_history(&path, &loaded.history).unwrap();
        let backup = path.with_extension("v1.backup.json");
        assert!(backup.exists());
        assert_eq!(load_history_at(&path, now).history.sample_count(), 2);
        clear_legacy_backup_for_current_account(&path, &loaded.history).unwrap();
        assert!(!backup.exists());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn range_summary_counts_only_raw_samples_inside_requested_window() {
        let mut value = history();
        let now = at(12, 0);
        let reset = now + Duration::days(14);
        let older = now - Duration::days(2);
        let boundary = now - Duration::hours(24);
        let recent = now - Duration::hours(1);
        let future = now + Duration::hours(1);
        for (sampled_at, remaining) in [(older, 90), (boundary, 80), (recent, 70), (future, 60)] {
            record(&mut value, sampled_at, remaining, reset);
        }

        let hours_24 = value
            .query_request(
                UsageHistoryRequest::Preset {
                    preset: UsageHistoryPreset::Hours24,
                },
                now,
            )
            .unwrap();
        assert_eq!(hours_24.sample_count, 2);
        assert_eq!(hours_24.earliest_sample_at, Some(boundary));
        assert_eq!(hours_24.latest_sample_at, Some(recent));

        let days_7 = value
            .query_request(
                UsageHistoryRequest::Preset {
                    preset: UsageHistoryPreset::Days7,
                },
                now,
            )
            .unwrap();
        assert_eq!(days_7.sample_count, 3);
        assert_eq!(days_7.earliest_sample_at, Some(older));
        assert_eq!(days_7.latest_sample_at, Some(recent));
    }

    #[test]
    fn today_consumption_uses_day_boundary_and_sums_across_resets_for_every_range() {
        let mut value = history();
        let today_started_at = at(0, 0);
        let first_reset = at(2, 0);
        record(
            &mut value,
            today_started_at - Duration::minutes(5),
            90,
            first_reset,
        );
        record(&mut value, at(1, 0), 70, first_reset);

        let next_reset = first_reset + Duration::hours(5);
        record(&mut value, first_reset, 100, next_reset);
        record(&mut value, at(4, 0), 80, next_reset);
        record(&mut value, at(6, 0), 40, next_reset);

        let now = at(6, 0);
        let requests = [
            UsageHistoryRequest::Preset {
                preset: UsageHistoryPreset::Hours24,
            },
            UsageHistoryRequest::Preset {
                preset: UsageHistoryPreset::Days7,
            },
            UsageHistoryRequest::Preset {
                preset: UsageHistoryPreset::Days30,
            },
            UsageHistoryRequest::Custom {
                start_at: today_started_at - Duration::minutes(5),
                end_at_exclusive: at(1, 30),
            },
        ];
        for request in requests {
            let query = value
                .query_request_with_day_start(request, now, today_started_at)
                .unwrap();
            assert_eq!(query.series[0].today_consumed_percent, 80);
        }
    }

    #[test]
    fn today_consumption_ignores_samples_outside_the_natural_day_and_future() {
        let mut value = history();
        let today_started_at = at(0, 0);
        let reset = at(20, 0);
        record(&mut value, today_started_at - Duration::hours(2), 90, reset);
        record(&mut value, at(1, 0), 80, reset);
        record(&mut value, at(6, 0), 70, reset);
        record(&mut value, at(7, 0), 60, reset);

        let query = value
            .query_request_with_day_start(
                UsageHistoryRequest::Preset {
                    preset: UsageHistoryPreset::Hours24,
                },
                at(6, 0),
                today_started_at,
            )
            .unwrap();
        assert_eq!(query.series[0].today_consumed_percent, 20);

        let invalid_boundary = value
            .query_request_with_day_start(
                UsageHistoryRequest::Preset {
                    preset: UsageHistoryPreset::Hours24,
                },
                at(6, 0),
                at(7, 0),
            )
            .unwrap();
        assert_eq!(invalid_boundary.series[0].today_consumed_percent, 0);
    }

    #[test]
    fn forecast_collects_until_sample_count_and_span_are_sufficient() {
        let mut value = history();
        let reset = at(20, 0);
        for (minute, remaining) in [(0, 100), (10, 99), (20, 98)] {
            record(&mut value, at(8, minute), remaining, reset);
        }
        let forecast = value.forecast_for("weekly", 604_800, at(8, 20)).unwrap();
        assert_eq!(forecast.status, ForecastStatus::Collecting);
        assert_eq!(forecast.sample_count, 3);

        record(&mut value, at(8, 29), 97, reset);
        let forecast = value.forecast_for("weekly", 604_800, at(8, 29)).unwrap();
        assert_eq!(forecast.status, ForecastStatus::Collecting);
        assert_eq!(forecast.observed_span_seconds, 29 * 60);
    }

    #[test]
    fn forecast_reports_stable_when_observed_decline_is_below_two_points() {
        let mut value = history();
        let reset = at(20, 0);
        for (minute, remaining) in [(0, 80), (10, 80), (20, 79), (30, 79)] {
            record(&mut value, at(8, minute), remaining, reset);
        }
        let forecast = value.forecast_for("weekly", 604_800, at(8, 30)).unwrap();
        assert_eq!(forecast.status, ForecastStatus::Stable);
        assert_eq!(forecast.consumed_percent, 1);
        assert_eq!(forecast.exhausts_at, None);
    }

    #[test]
    fn forecast_reports_exhaustion_before_reset_rounded_to_quarter_hour() {
        let mut value = history();
        let reset = at(20, 0);
        for (minute, remaining) in [(0, 80), (10, 70), (20, 60), (30, 50)] {
            record(&mut value, at(8, minute), remaining, reset);
        }
        let forecast = value.forecast_for("weekly", 604_800, at(8, 30)).unwrap();
        assert_eq!(forecast.status, ForecastStatus::ExhaustsBeforeReset);
        assert_eq!(forecast.exhausts_at, Some(at(9, 15)));
        assert_eq!(forecast.sample_count, 4);
        assert_eq!(forecast.observed_span_seconds, 30 * 60);
        assert_eq!(forecast.consumed_percent, 30);
    }

    #[test]
    fn forecast_does_not_extrapolate_past_reset() {
        let mut value = history();
        let reset = at(10, 0);
        for (minute, remaining) in [(0, 90), (10, 88), (20, 86), (30, 84)] {
            record(&mut value, at(8, minute), remaining, reset);
        }
        let forecast = value.forecast_for("weekly", 604_800, at(8, 30)).unwrap();
        assert_eq!(forecast.status, ForecastStatus::LastsUntilReset);
        assert_eq!(forecast.exhausts_at, None);
    }

    #[test]
    fn forecast_uses_only_latest_cycle_and_last_six_hours() {
        let mut value = history();
        let old_reset = at(10, 0);
        for (minute, remaining) in [(0, 100), (10, 70), (20, 40), (30, 10)] {
            record(&mut value, at(8, minute), remaining, old_reset);
        }
        let new_reset = old_reset + Duration::days(7);
        // 在旧截止之后进入新周期，并放入一个超过六小时回看范围的旧点。
        record(&mut value, old_reset, 80, new_reset);
        for (minutes, remaining) in [(390, 100), (400, 100), (410, 99), (420, 99)] {
            record(
                &mut value,
                old_reset + Duration::minutes(minutes),
                remaining,
                new_reset,
            );
        }
        let forecast = value
            .forecast_for("weekly", 604_800, old_reset + Duration::minutes(420))
            .unwrap();
        assert_eq!(forecast.status, ForecastStatus::Stable);
        assert_eq!(forecast.sample_count, 4);
        assert_eq!(forecast.consumed_percent, 1);
    }

    #[test]
    fn zero_percent_can_produce_an_immediate_reliable_forecast() {
        let mut value = history();
        let reset = at(20, 0);
        for (minute, remaining) in [(0, 9), (10, 6), (20, 3), (30, 0)] {
            record(&mut value, at(8, minute), remaining, reset);
        }
        let forecast = value.forecast_for("weekly", 604_800, at(8, 30)).unwrap();
        assert_eq!(forecast.status, ForecastStatus::ExhaustsBeforeReset);
        assert_eq!(forecast.exhausts_at, Some(at(8, 30)));
    }

    #[test]
    fn zero_percent_at_non_quarter_hour_never_rounds_before_latest_sample() {
        let mut value = history();
        let reset = at(20, 0);
        for (minute, remaining) in [(7, 9), (17, 6), (27, 3), (37, 0)] {
            record(&mut value, at(8, minute), remaining, reset);
        }

        let latest_sample_at = at(8, 37);
        let forecast = value
            .forecast_for("weekly", 604_800, latest_sample_at)
            .unwrap();
        assert_eq!(forecast.status, ForecastStatus::ExhaustsBeforeReset);
        assert_eq!(forecast.exhausts_at, Some(at(8, 45)));
        assert!(forecast.exhausts_at.unwrap() >= latest_sample_at);
    }

    #[test]
    fn zero_percent_prefers_latest_sample_when_next_quarter_crosses_reset() {
        let mut value = history();
        let reset = at(8, 40);
        for (minute, remaining) in [(7, 9), (17, 6), (27, 3), (37, 0)] {
            record(&mut value, at(8, minute), remaining, reset);
        }

        let latest_sample_at = at(8, 37);
        let forecast = value
            .forecast_for("weekly", 604_800, latest_sample_at)
            .unwrap();
        assert_eq!(forecast.status, ForecastStatus::ExhaustsBeforeReset);
        assert_eq!(forecast.exhausts_at, Some(latest_sample_at));
        assert!(forecast.exhausts_at.unwrap() < reset);
    }

    #[test]
    fn system_clock_rollback_never_uses_future_samples() {
        let mut value = history();
        let reset = at(20, 0);
        for (minute, remaining) in [(0, 80), (10, 70), (20, 60), (30, 50)] {
            record(&mut value, at(8, minute), remaining, reset);
        }
        let forecast = value.forecast_for("weekly", 604_800, at(7, 59)).unwrap();
        assert_eq!(forecast.status, ForecastStatus::Collecting);
        assert_eq!(forecast.sample_count, 0);
        assert_eq!(forecast.exhausts_at, None);
    }

    #[test]
    fn clear_keeps_account_partition_but_removes_all_points() {
        let mut value = history();
        record(&mut value, at(8, 0), 80, at(20, 0));
        let fingerprint = value.account_fingerprint.clone();
        value
            .select_account(AccountIdentity::AccountId("another-account"))
            .unwrap();
        value
            .record_successful_snapshot(
                AccountIdentity::AccountId("another-account"),
                at(9, 0),
                &[window("weekly", 604_800, 60, Some(at(20, 0)))],
            )
            .unwrap();
        assert_eq!(value.clear_samples(), 1);
        assert!(value.is_empty());
        assert_eq!(value.clear_samples(), 0);
        value.select_account(identity()).unwrap();
        assert_eq!(value.account_fingerprint, fingerprint);
        assert_eq!(value.sample_count(), 1);
    }
}
