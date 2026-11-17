mod collection;
use query::downsample_cycle;
mod codec;
mod forecast;
mod query;
use chrono::{DateTime, Duration, TimeZone, Utc};
pub(crate) use codec::{decode_history, recovered_history};
mod analytics;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fmt,
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
    /// 首次归零是事实摘要，不能在历史压缩时被桶末点覆盖。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    first_exhausted_at: Option<DateTime<Utc>>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_remaining_percent: Option<u8>,
    /// 前端在此点断线；不需要获得内部保存的 reset 时间。
    pub break_before: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageHistorySeries {
    pub current_reset_at: Option<DateTime<Utc>>,
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
    fn latest_sample_at(&self) -> Option<DateTime<Utc>> {
        self.cycles
            .last()
            .and_then(|cycle| cycle.samples.last())
            .map(|sample| sample.sampled_at)
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
mod tests;
