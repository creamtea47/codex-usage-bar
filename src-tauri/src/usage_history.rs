use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fmt, fs, io, path::Path};

pub const USAGE_HISTORY_FILE_NAME: &str = "usage-history.json";
pub const USAGE_HISTORY_SCHEMA_VERSION: u32 = 1;

const SALT_BYTES: usize = 32;
const FINGERPRINT_BYTES: usize = 32;
const STREAM_KEY_BYTES: usize = 32;
const RETENTION_DAYS: i64 = 7;
const SAMPLE_INTERVAL_MINUTES: i64 = 5;
const FORECAST_LOOKBACK_HOURS: i64 = 6;
const FORECAST_MIN_SAMPLES: usize = 4;
const FORECAST_MIN_SPAN_MINUTES: i64 = 30;
const FORECAST_MIN_CONSUMPTION_PERCENT: u8 = 2;
const FORECAST_ROUND_SECONDS: i64 = 15 * 60;
const SEVEN_DAY_BUCKET_SECONDS: i64 = 15 * 60;
const MAX_POINTS_PER_STREAM: usize = 2_500;
const MAX_STREAMS: usize = 16;
const MAX_WINDOW_ID_BYTES: usize = 256;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountSelection {
    Initialized,
    Unchanged,
    ChangedAndCleared { cleared_samples: usize },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistoryMutation {
    pub account_changed: bool,
    pub samples_recorded: usize,
    pub samples_pruned: usize,
    pub samples_cleared: usize,
    pub streams_evicted: usize,
    pub ignored_windows: usize,
}

impl HistoryMutation {
    pub fn changed(self) -> bool {
        self.account_changed
            || self.samples_recorded > 0
            || self.samples_pruned > 0
            || self.samples_cleared > 0
            || self.streams_evicted > 0
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum UsageHistoryRange {
    #[serde(rename = "24h")]
    Hours24,
    #[serde(rename = "7d")]
    Days7,
}

impl UsageHistoryRange {
    fn duration(self) -> Duration {
        match self {
            Self::Hours24 => Duration::hours(24),
            Self::Days7 => Duration::days(7),
        }
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
    pub points: Vec<UsageHistoryPoint>,
    pub forecast: Forecast,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageHistoryQuery {
    pub range: UsageHistoryRange,
    pub generated_at: DateTime<Utc>,
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
                Ok(AccountSelection::Initialized)
            }
            Some(current) if current == fingerprint => Ok(AccountSelection::Unchanged),
            Some(_) => {
                let cleared_samples = self.sample_count();
                self.streams.clear();
                self.account_fingerprint = Some(fingerprint);
                Ok(AccountSelection::ChangedAndCleared { cleared_samples })
            }
        }
    }

    /// 只应在成功获取用量快照后调用。该方法先完成账号隔离，再依据采样策略写入。
    pub fn record_successful_snapshot(
        &mut self,
        identity: AccountIdentity<'_>,
        sampled_at: DateTime<Utc>,
        windows: &[HistoryWindowInput],
    ) -> Result<HistoryMutation, FingerprintError> {
        let mut mutation = HistoryMutation::default();
        match self.select_account(identity)? {
            AccountSelection::Initialized => mutation.account_changed = true,
            AccountSelection::Unchanged => {}
            AccountSelection::ChangedAndCleared { cleared_samples } => {
                mutation.account_changed = true;
                mutation.samples_cleared = cleared_samples;
            }
        }

        mutation.samples_pruned = self.prune_at(sampled_at);
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

            let stream_index = self
                .streams
                .iter()
                .position(|stream| stream.matches(&stored_window));
            let recorded = if let Some(index) = stream_index {
                self.streams[index].record(sampled_at, &stored_window)
            } else {
                if self.streams.len() >= MAX_STREAMS {
                    mutation.samples_pruned += self.evict_oldest_stream();
                    mutation.streams_evicted += 1;
                }
                self.streams.push(StoredUsageStream::from_first_sample(
                    sampled_at,
                    &stored_window,
                ));
                true
            };

            if recorded {
                mutation.samples_recorded += 1;
                let index = self
                    .streams
                    .iter()
                    .position(|stream| stream.matches(&stored_window))
                    .expect("recorded stream must remain present");
                mutation.samples_pruned += self.streams[index].trim_to_point_limit();
            }
        }
        Ok(mutation)
    }

    /// 用户清除历史时保留随机盐和当前账号指纹，避免清除后产生可关联的新指纹。
    #[cfg(test)]
    pub fn clear_samples(&mut self) -> usize {
        let removed = self.sample_count();
        self.streams.clear();
        removed
    }

    pub fn prune_at(&mut self, now: DateTime<Utc>) -> usize {
        let cutoff = safe_subtract(now, Duration::days(RETENTION_DAYS));
        let mut removed = 0;
        for stream in &mut self.streams {
            removed += stream.prune_before(cutoff);
            removed += stream.trim_to_point_limit();
        }
        self.streams.retain(|stream| !stream.cycles.is_empty());
        while self.streams.len() > MAX_STREAMS {
            removed += self.evict_oldest_stream();
        }
        removed
    }

    pub fn query(&self, range: UsageHistoryRange, now: DateTime<Utc>) -> UsageHistoryQuery {
        let cutoff = safe_subtract(now, range.duration());
        let downsample = range == UsageHistoryRange::Days7;
        let mut series = self
            .streams
            .iter()
            .filter_map(|stream| {
                let points = stream.query_points(cutoff, now, downsample);
                let current_remaining_percent = points.last()?.remaining_percent;
                Some(UsageHistorySeries {
                    window_id: stream.window_id.clone(),
                    window_seconds: stream.window_seconds,
                    current_remaining_percent,
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
        UsageHistoryQuery {
            range,
            generated_at: now,
            series,
        }
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

    pub fn summary_for_range(
        &self,
        range: UsageHistoryRange,
        now: DateTime<Utc>,
    ) -> UsageHistorySummary {
        let cutoff = safe_subtract(now, range.duration());
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
                .filter(|sample| sample.sampled_at >= cutoff && sample.sampled_at <= now)
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

    fn evict_oldest_stream(&mut self) -> usize {
        let Some((index, _)) = self
            .streams
            .iter()
            .enumerate()
            .min_by_key(|(_, stream)| stream.latest_sample_at())
        else {
            return 0;
        };
        self.streams.remove(index).sample_count()
    }

    fn validate(&self) -> bool {
        if self.schema_version != USAGE_HISTORY_SCHEMA_VERSION
            || decode_hex_exact(&self.salt, SALT_BYTES).is_none()
            || self
                .account_fingerprint
                .as_deref()
                .is_some_and(|value| decode_hex_exact(value, FINGERPRINT_BYTES).is_none())
            || (self.account_fingerprint.is_none() && !self.streams.is_empty())
        {
            return false;
        }

        let mut keys = HashSet::new();
        self.streams.iter().all(|stream| {
            valid_stored_window_key(&stream.window_id, stream.window_seconds)
                && keys.insert((stream.window_id.as_str(), stream.window_seconds))
                && stream.validate()
        })
    }
}

impl StoredUsageStream {
    fn from_first_sample(sampled_at: DateTime<Utc>, window: &HistoryWindowInput) -> Self {
        Self {
            window_id: window.window_id.clone(),
            window_seconds: window.window_seconds,
            cycles: vec![StoredUsageCycle {
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

    fn record(&mut self, sampled_at: DateTime<Utc>, window: &HistoryWindowInput) -> bool {
        let Some(latest_at) = self.latest_sample_at() else {
            *self = Self::from_first_sample(sampled_at, window);
            return true;
        };
        // 系统时间倒退时跳过该点，避免把当前周期写成非单调时间序列。
        if sampled_at < latest_at {
            return false;
        }

        let cycle = self
            .cycles
            .last_mut()
            .expect("non-empty stream must contain a cycle");
        let last = cycle
            .samples
            .last()
            .expect("non-empty cycle must contain a sample");
        let reset_advanced = match (cycle.reset_at, window.cycle_reset_at) {
            (Some(previous), Some(next)) => next > previous && sampled_at >= previous,
            _ => false,
        };
        let inferred_reset = window.remaining_percent > last.remaining_percent
            && (cycle.reset_at.is_none()
                || cycle
                    .reset_at
                    .is_some_and(|known_reset| sampled_at >= known_reset));

        if reset_advanced || inferred_reset {
            self.cycles.push(StoredUsageCycle {
                reset_at: window.cycle_reset_at,
                samples: vec![StoredUsageSample {
                    sampled_at,
                    remaining_percent: window.remaining_percent,
                }],
            });
            return true;
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
                return true;
            }
            return false;
        }

        let interval_elapsed =
            sampled_at - last.sampled_at >= Duration::minutes(SAMPLE_INTERVAL_MINUTES);
        let percentage_changed = last.remaining_percent != window.remaining_percent;
        if !interval_elapsed && !percentage_changed {
            return false;
        }
        cycle.samples.push(StoredUsageSample {
            sampled_at,
            remaining_percent: window.remaining_percent,
        });
        true
    }

    fn prune_before(&mut self, cutoff: DateTime<Utc>) -> usize {
        let mut removed = 0;
        for cycle in &mut self.cycles {
            let before = cycle.samples.len();
            cycle.samples.retain(|sample| sample.sampled_at >= cutoff);
            removed += before - cycle.samples.len();
        }
        self.cycles.retain(|cycle| !cycle.samples.is_empty());
        removed
    }

    fn trim_to_point_limit(&mut self) -> usize {
        let mut excess = self.sample_count().saturating_sub(MAX_POINTS_PER_STREAM);
        let original_excess = excess;
        for cycle in &mut self.cycles {
            if excess == 0 {
                break;
            }
            let remove = excess.min(cycle.samples.len());
            cycle.samples.drain(..remove);
            excess -= remove;
        }
        self.cycles.retain(|cycle| !cycle.samples.is_empty());
        original_excess
    }

    fn sample_count(&self) -> usize {
        self.cycles.iter().map(|cycle| cycle.samples.len()).sum()
    }

    fn latest_sample_at(&self) -> Option<DateTime<Utc>> {
        self.cycles
            .last()
            .and_then(|cycle| cycle.samples.last())
            .map(|sample| sample.sampled_at)
    }

    fn query_points(
        &self,
        cutoff: DateTime<Utc>,
        now: DateTime<Utc>,
        downsample: bool,
    ) -> Vec<UsageHistoryPoint> {
        let mut points = Vec::new();
        for cycle in &self.cycles {
            let samples = cycle
                .samples
                .iter()
                .filter(|sample| sample.sampled_at >= cutoff && sample.sampled_at <= now)
                .collect::<Vec<_>>();
            let selected = if downsample {
                downsample_cycle(&samples)
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
        points
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
            if cycle.samples.is_empty() {
                return false;
            }
            let begins_after_previous = previous_cycle_last
                .map_or(true, |previous| cycle.samples[0].sampled_at >= previous);
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
    if version != USAGE_HISTORY_SCHEMA_VERSION as u64 {
        return recovered_history(HistoryStorageStatus::RecoveredUnsupported);
    }
    let mut history: UsageHistory = match serde_json::from_value::<UsageHistory>(value) {
        Ok(history) if history.validate() => history,
        _ => return recovered_history(HistoryStorageStatus::RecoveredCorrupt),
    };
    let removed = history.prune_at(now);
    LoadedUsageHistory {
        history,
        status: HistoryStorageStatus::Ready,
        needs_rewrite: removed > 0,
    }
}

pub fn save_history(path: &Path, history: &UsageHistory) -> Result<(), HistoryPersistenceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| HistoryPersistenceError::Storage)?;
    }
    let content =
        serde_json::to_vec_pretty(history).map_err(|_| HistoryPersistenceError::Serialize)?;
    let temporary_path = path.with_extension("json.tmp");
    fs::write(&temporary_path, content).map_err(|_| HistoryPersistenceError::Storage)?;
    if path.exists() {
        fs::remove_file(path).map_err(|_| HistoryPersistenceError::Storage)?;
    }
    if fs::rename(&temporary_path, path).is_err() {
        let _ = fs::remove_file(&temporary_path);
        return Err(HistoryPersistenceError::Storage);
    }
    Ok(())
}

/// 账号切换、用户清除和损坏恢复必须先丢弃旧目标，避免安全新文件写入失败时
/// 旧账号样本或未知内容仍留在磁盘。删除失败只返回稳定存储类别，不暴露路径。
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

fn downsample_cycle<'a>(samples: &[&'a StoredUsageSample]) -> Vec<&'a StoredUsageSample> {
    if samples.len() <= 2 {
        return samples.to_vec();
    }
    let first = samples[0];
    let mut selected = vec![first];
    let mut bucket = first
        .sampled_at
        .timestamp()
        .div_euclid(SEVEN_DAY_BUCKET_SECONDS);
    let mut bucket_last = first;
    for sample in samples.iter().copied().skip(1) {
        let sample_bucket = sample
            .sampled_at
            .timestamp()
            .div_euclid(SEVEN_DAY_BUCKET_SECONDS);
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
    fn account_switch_clears_every_old_stream_immediately() {
        let mut value = history();
        record(&mut value, at(8, 0), 80, at(20, 0));
        assert_eq!(value.sample_count(), 1);
        let selection = value
            .select_account(AccountIdentity::AccountId("another-account"))
            .unwrap();
        assert_eq!(
            selection,
            AccountSelection::ChangedAndCleared { cleared_samples: 1 }
        );
        assert!(value.is_empty());
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
        let query = value.query(UsageHistoryRange::Hours24, at(8, 2));
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
    fn prunes_seven_days_and_enforces_per_stream_point_cap() {
        let mut value = history();
        let start = at(8, 0) - Duration::days(2);
        let reset = at(20, 0) + Duration::days(30);
        for index in 0..=MAX_POINTS_PER_STREAM {
            let time = start + Duration::minutes(index as i64);
            record(&mut value, time, 80 - (index % 2) as u8, reset);
        }
        assert_eq!(value.sample_count(), MAX_POINTS_PER_STREAM);

        let mut stale = history();
        record(&mut stale, at(8, 0) - Duration::days(8), 80, reset);
        assert_eq!(stale.prune_at(at(8, 0)), 1);
        assert!(stale.is_empty());
    }

    #[test]
    fn evicts_oldest_stream_at_sixteen_stream_limit() {
        let mut value = history();
        let reset = at(20, 0);
        for index in 0..MAX_STREAMS {
            value
                .record_successful_snapshot(
                    identity(),
                    at(8, 0) + Duration::minutes(index as i64),
                    &[window(
                        &format!("window-{index}"),
                        60 + index as i64,
                        90,
                        Some(reset),
                    )],
                )
                .unwrap();
        }
        let mutation = value
            .record_successful_snapshot(
                identity(),
                at(9, 0),
                &[window("newest", 999, 90, Some(reset))],
            )
            .unwrap();
        assert_eq!(mutation.streams_evicted, 1);
        assert_eq!(value.stream_count(), MAX_STREAMS);
        let oldest_key = window_stream_key(&value.salt, "window-0", 60).unwrap();
        let newest_key = window_stream_key(&value.salt, "newest", 999).unwrap();
        assert!(!value
            .streams
            .iter()
            .any(|stream| stream.window_id == oldest_key));
        assert!(value
            .streams
            .iter()
            .any(|stream| stream.window_id == newest_key));
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
        let full = value.query(UsageHistoryRange::Hours24, at(11, 0));
        let sampled = value.query(UsageHistoryRange::Days7, at(11, 0));
        assert!(sampled.series[0].points.len() < full.series[0].points.len());
        assert!(sampled.series[0].points[0].break_before);
        let json = serde_json::to_string(&sampled).unwrap();
        assert!(!json.contains("accountFingerprint"));
        assert!(!json.contains("salt"));
        assert!(!json.contains("resetAt"));
        assert!(!json.contains("label"));
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

        let hours_24 = value.summary_for_range(UsageHistoryRange::Hours24, now);
        assert_eq!(hours_24.stream_count, 1);
        assert_eq!(hours_24.sample_count, 2);
        assert_eq!(hours_24.oldest_sample_at, Some(boundary));
        assert_eq!(hours_24.latest_sample_at, Some(recent));

        let days_7 = value.summary_for_range(UsageHistoryRange::Days7, now);
        assert_eq!(days_7.stream_count, 1);
        assert_eq!(days_7.sample_count, 3);
        assert_eq!(days_7.oldest_sample_at, Some(older));
        assert_eq!(days_7.latest_sample_at, Some(recent));
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
        assert_eq!(value.clear_samples(), 1);
        assert_eq!(value.account_fingerprint, fingerprint);
        assert!(value.is_empty());
        assert_eq!(value.clear_samples(), 0);
    }
}
