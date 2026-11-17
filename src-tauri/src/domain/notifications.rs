//! Pure notification-decision state machine.
//!
//! This module intentionally knows nothing about Tauri or platform notification
//! APIs. The caller converts a successful dashboard snapshot into
//! [`NotificationSnapshot`], passes the current policy to
//! [`NotificationTracker::evaluate`], and localizes/sends the returned batch.

use crate::domain::quota_reset::NotificationDisposition;
use crate::domain::quota_reset::QuotaResetEvent;
use std::collections::{HashMap, HashSet};

const MAX_NOTIFICATION_ITEMS: usize = 3;
const MINUTES_PER_DAY: u16 = 24 * 60;

/// User-configurable notification policy.
///
/// Threshold values greater than 100 are clamped to 100 at evaluation time so
/// corrupt settings cannot create surprising comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationPolicy {
    pub enabled: bool,
    pub low_remaining_enabled: bool,
    pub pace_deficit_enabled: bool,
    pub reset_enabled: bool,
    pub low_remaining_threshold: u8,
    pub pace_deficit_threshold: u8,
    pub quiet_hours: QuietHours,
}

impl Default for NotificationPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            low_remaining_enabled: true,
            pace_deficit_enabled: true,
            reset_enabled: true,
            low_remaining_threshold: 20,
            pace_deficit_threshold: 10,
            quiet_hours: QuietHours::default(),
        }
    }
}

/// Quiet interval expressed as local minutes after midnight.
///
/// The interval is `[start_minute, end_minute)`. A start equal to the end is
/// an empty interval, not an all-day quiet period. Invalid minute values make
/// the interval inactive; persisted settings should normalize them separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QuietHours {
    pub enabled: bool,
    pub start_minute: u16,
    pub end_minute: u16,
}

impl QuietHours {
    pub fn contains(self, local_minute: u16) -> bool {
        if !self.enabled
            || self.start_minute >= MINUTES_PER_DAY
            || self.end_minute >= MINUTES_PER_DAY
            || local_minute >= MINUTES_PER_DAY
            || self.start_minute == self.end_minute
        {
            return false;
        }

        if self.start_minute < self.end_minute {
            (self.start_minute..self.end_minute).contains(&local_minute)
        } else {
            local_minute >= self.start_minute || local_minute < self.end_minute
        }
    }
}

/// Minimal, account-free view of a quota window needed by the rule engine.
///
/// `key` is used only inside the tracker to correlate snapshots and is never
/// copied into notification output. This lets callers use an internal stable
/// identifier without risking it being rendered as notification text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationWindow {
    pub key: String,
    pub remaining_percent: u8,
    pub reset_at_unix_seconds: Option<i64>,
    pub start_at_unix_seconds: Option<i64>,
    pub window_seconds: i64,
    pub is_long_period: bool,
}

/// A successful usage observation. The caller supplies local wall-clock time
/// explicitly so timezone conversion remains outside this pure module.
#[derive(Debug, Clone, Copy)]
pub struct NotificationSnapshot<'a> {
    pub observed_at_unix_seconds: i64,
    pub local_minute_of_day: u16,
    pub windows: &'a [NotificationWindow],
}

/// 调用层把统一重置事件与当前安全快照中的窗口索引配对后传入。
/// 适配器只在本次评估调用栈内存在，不复制或持久化上游窗口 ID。
#[derive(Debug, Clone, Copy)]
pub struct MatchedQuotaResetEvent<'a> {
    pub window_index: usize,
    pub event: &'a QuotaResetEvent,
}

/// Stable, non-localized reasons for one window's notification entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationReason {
    LowRemaining { remaining_percent: u8 },
    PaceDeficit { deficit_points: u8 },
    Reset,
}

/// One line/item in a merged notification.
///
/// The index points into the snapshot passed to the same `evaluate` call. The
/// caller can resolve it to already-sanitized/localized window metadata. No
/// account identifier or arbitrary upstream label is returned by this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationItem {
    pub window_index: usize,
    pub reasons: Vec<NotificationReason>,
}

/// A single platform notification payload containing at most three windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationBatch {
    pub items: Vec<NotificationItem>,
    pub omitted_window_count: usize,
}

#[derive(Debug, Clone)]
struct WindowState {
    low_remaining_handled: bool,
    pace_deficit_handled: bool,
}

/// Stateful rule evaluator. One tracker should be kept for the active account.
#[derive(Debug)]
pub struct NotificationTracker {
    baseline_pending: bool,
    windows: HashMap<String, WindowState>,
    /// 事件 ID 和 generation ID 都来自统一识别器且不含账号明文。分别去重可防止
    /// 同一事件重复广播，以及异常调用方为同一代次构造不同 event ID。
    handled_reset_event_ids: HashSet<String>,
    handled_reset_generation_ids: HashSet<String>,
}

impl Default for NotificationTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationTracker {
    pub fn new() -> Self {
        Self {
            baseline_pending: true,
            windows: HashMap::new(),
            handled_reset_event_ids: HashSet::new(),
            handled_reset_generation_ids: HashSet::new(),
        }
    }

    /// Forget all prior threshold state. Call this after notification thresholds,
    /// switches, quiet hours, or language change. The next successful snapshot
    /// establishes low/pace baselines; a reset newly confirmed on that same refresh
    /// is still eligible because the persistent recognizer filters historical events.
    pub fn reset_baseline(&mut self) {
        self.windows.clear();
        // 去重集合刻意跨设置基线保留；否则语言或阈值变化时，调用方同批重试会
        // 让同一 reset event 再次进入 Windows 通知。跨进程去重由持久化识别器负责。
        self.baseline_pending = true;
    }

    /// Evaluate one successful snapshot and return at most one merged batch.
    ///
    /// Events reached during quiet hours are still marked handled, so they are
    /// never replayed when quiet hours end. Newly appearing windows establish
    /// their own baseline and do not alert immediately.
    #[allow(dead_code)] // 保留无统一事件的兼容入口和低额/进度规则回归测试。
    pub fn evaluate<F>(
        &mut self,
        snapshot: &NotificationSnapshot<'_>,
        policy: &NotificationPolicy,
        reset_event_is_quiet: F,
    ) -> Option<NotificationBatch>
    where
        F: Fn(i64) -> bool,
    {
        self.evaluate_with_confirmed_resets(snapshot, policy, &[], reset_event_is_quiet)
    }

    /// 消费统一识别器已经确认的重置事件，并与低额度/进度提醒合并成一个批次。
    ///
    /// 识别器负责判断 `reset_at` 提前推进、98/99→100 与 generation；本模块只负责
    /// 通知策略和 at-most-once 消费，避免通知与自动接续各自推断出不同的周期。
    pub fn evaluate_with_confirmed_resets<F>(
        &mut self,
        snapshot: &NotificationSnapshot<'_>,
        policy: &NotificationPolicy,
        confirmed_resets: &[MatchedQuotaResetEvent<'_>],
        reset_event_is_quiet: F,
    ) -> Option<NotificationBatch>
    where
        F: Fn(i64) -> bool,
    {
        if !policy.enabled {
            // This also makes enabling safe if integration code forgets to call
            // reset_baseline explicitly: the first enabled snapshot is quiet.
            self.reset_baseline();
            return None;
        }

        let establishes_baseline = self.baseline_pending;
        if establishes_baseline {
            self.windows.clear();
            for window in snapshot.windows {
                self.windows.insert(
                    window.key.clone(),
                    baseline_state(window, snapshot.observed_at_unix_seconds, policy),
                );
            }
            self.baseline_pending = false;
        }

        let low_threshold = policy.low_remaining_threshold.min(100);
        let pace_threshold = policy.pace_deficit_threshold.min(100);
        let mut newly_seen_windows = HashSet::new();
        for (window_index, window) in snapshot.windows.iter().enumerate() {
            if !self.windows.contains_key(&window.key) {
                self.windows.insert(
                    window.key.clone(),
                    baseline_state(window, snapshot.observed_at_unix_seconds, policy),
                );
                newly_seen_windows.insert(window_index);
            }
        }

        let mut reasons_by_window = HashMap::<usize, Vec<NotificationReason>>::new();
        for matched in confirmed_resets {
            let Some(window) = snapshot.windows.get(matched.window_index) else {
                continue;
            };
            let event = matched.event;
            if event.window_seconds != window.window_seconds
                || event.event_id.is_empty()
                || event.generation_id.is_empty()
                || event.notification_disposition != NotificationDisposition::Pending
            {
                continue;
            }
            if self.handled_reset_event_ids.contains(&event.event_id)
                || self
                    .handled_reset_generation_ids
                    .contains(&event.generation_id)
            {
                continue;
            }

            // 先消费再应用通知策略。静默、关闭重置提醒或同批次后续分支都不能重放事件。
            self.handled_reset_event_ids.insert(event.event_id.clone());
            self.handled_reset_generation_ids
                .insert(event.generation_id.clone());
            if newly_seen_windows.contains(&matched.window_index) {
                continue;
            }
            let Some(state) = self.windows.get_mut(&window.key) else {
                continue;
            };
            // 首次刷新已经按重置后的快照建立低额/进度基线。只允许新确认的 Reset
            // 穿过该基线，避免在重启恢复时把当前事件误当历史，也避免顺带补发低额提醒。
            if !establishes_baseline {
                state.low_remaining_handled = false;
                state.pace_deficit_handled = false;
            }
            if policy.reset_enabled && !reset_event_is_quiet(event.detected_at.timestamp()) {
                reasons_by_window
                    .entry(matched.window_index)
                    .or_default()
                    .push(NotificationReason::Reset);
            }
        }

        for (window_index, window) in snapshot.windows.iter().enumerate() {
            let remaining_percent = window.remaining_percent.min(100);
            if establishes_baseline || newly_seen_windows.contains(&window_index) {
                continue;
            }
            let state = self
                .windows
                .get_mut(&window.key)
                .expect("current windows were synchronized before evaluation");
            let reasons = reasons_by_window.entry(window_index).or_default();

            if remaining_percent <= low_threshold && !state.low_remaining_handled {
                state.low_remaining_handled = true;
                if policy.low_remaining_enabled {
                    reasons.push(NotificationReason::LowRemaining { remaining_percent });
                }
            }

            if let Some(deficit) = pace_deficit_points(window, snapshot.observed_at_unix_seconds) {
                let reaches_threshold =
                    deficit > 0.0 && deficit + f64::EPSILON >= pace_threshold.into();
                if reaches_threshold && !state.pace_deficit_handled {
                    state.pace_deficit_handled = true;
                    if policy.pace_deficit_enabled {
                        reasons.push(NotificationReason::PaceDeficit {
                            deficit_points: deficit.ceil().clamp(0.0, 100.0) as u8,
                        });
                    }
                }
            }
        }

        // A removed stream is treated as new if it later returns, preventing a
        // stale generation from producing an immediate alert after API changes.
        self.windows
            .retain(|key, _| snapshot.windows.iter().any(|window| &window.key == key));

        let mut triggered = reasons_by_window
            .into_iter()
            .filter(|(_, reasons)| !reasons.is_empty())
            .map(|(window_index, reasons)| NotificationItem {
                window_index,
                reasons,
            })
            .collect::<Vec<_>>();
        triggered.sort_by_key(|item| item.window_index);

        if triggered.is_empty() || policy.quiet_hours.contains(snapshot.local_minute_of_day) {
            return None;
        }

        let omitted_window_count = triggered.len().saturating_sub(MAX_NOTIFICATION_ITEMS);
        triggered.truncate(MAX_NOTIFICATION_ITEMS);
        Some(NotificationBatch {
            items: triggered,
            omitted_window_count,
        })
    }
}

fn baseline_state(
    window: &NotificationWindow,
    observed_at_unix_seconds: i64,
    policy: &NotificationPolicy,
) -> WindowState {
    let remaining_percent = window.remaining_percent.min(100);
    let low_threshold = policy.low_remaining_threshold.min(100);
    let pace_threshold = policy.pace_deficit_threshold.min(100);
    let pace_already_reached = pace_deficit_points(window, observed_at_unix_seconds)
        .is_some_and(|deficit| deficit > 0.0 && deficit + f64::EPSILON >= pace_threshold.into());

    WindowState {
        low_remaining_handled: remaining_percent <= low_threshold,
        pace_deficit_handled: pace_already_reached,
    }
}

fn pace_deficit_points(window: &NotificationWindow, observed_at_unix_seconds: i64) -> Option<f64> {
    if !window.is_long_period {
        return None;
    }

    let reset_at = window.reset_at_unix_seconds?;
    let start_at = window.start_at_unix_seconds.or_else(|| {
        (window.window_seconds > 0).then(|| reset_at.saturating_sub(window.window_seconds))
    })?;
    if start_at >= reset_at
        || observed_at_unix_seconds < start_at
        || observed_at_unix_seconds >= reset_at
    {
        return None;
    }

    let total_seconds = reset_at.saturating_sub(start_at) as f64;
    let remaining_seconds = reset_at.saturating_sub(observed_at_unix_seconds) as f64;
    let suggested_remaining = (remaining_seconds / total_seconds * 100.0).clamp(0.0, 100.0);
    Some(suggested_remaining - f64::from(window.remaining_percent.min(100)))
}

#[cfg(test)]
mod tests;
