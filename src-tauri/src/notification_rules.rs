//! Pure notification-decision state machine.
//!
//! This module intentionally knows nothing about Tauri or platform notification
//! APIs. The caller converts a successful dashboard snapshot into
//! [`NotificationSnapshot`], passes the current policy to
//! [`NotificationTracker::evaluate`], and localizes/sends the returned batch.

use crate::quota_reset::{NotificationDisposition, QuotaResetEvent};
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
mod tests {
    use super::*;
    use crate::quota_reset::{confirmed_event, generation_id, QuotaResetReason};
    use chrono::{DateTime, Utc};

    fn at(unix_seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(unix_seconds, 0).unwrap()
    }

    fn policy() -> NotificationPolicy {
        NotificationPolicy {
            enabled: true,
            ..NotificationPolicy::default()
        }
    }

    fn window(key: &str, remaining_percent: u8) -> NotificationWindow {
        NotificationWindow {
            key: key.to_owned(),
            remaining_percent,
            reset_at_unix_seconds: Some(1_000),
            start_at_unix_seconds: Some(0),
            window_seconds: 1_000,
            is_long_period: false,
        }
    }

    fn evaluate(
        tracker: &mut NotificationTracker,
        windows: &[NotificationWindow],
        observed_at_unix_seconds: i64,
        local_minute_of_day: u16,
        policy: &NotificationPolicy,
    ) -> Option<NotificationBatch> {
        tracker.evaluate(
            &NotificationSnapshot {
                observed_at_unix_seconds,
                local_minute_of_day,
                windows,
            },
            policy,
            |_| false,
        )
    }

    fn evaluate_with_resets<F>(
        tracker: &mut NotificationTracker,
        windows: &[NotificationWindow],
        observed_at_unix_seconds: i64,
        local_minute_of_day: u16,
        policy: &NotificationPolicy,
        confirmed_resets: &[MatchedQuotaResetEvent<'_>],
        reset_event_is_quiet: F,
    ) -> Option<NotificationBatch>
    where
        F: Fn(i64) -> bool,
    {
        tracker.evaluate_with_confirmed_resets(
            &NotificationSnapshot {
                observed_at_unix_seconds,
                local_minute_of_day,
                windows,
            },
            policy,
            confirmed_resets,
            reset_event_is_quiet,
        )
    }

    fn reset_event(
        sequence: u64,
        detected_at: i64,
        reason: QuotaResetReason,
        previous_remaining_percent: u8,
        current_remaining_percent: u8,
    ) -> QuotaResetEvent {
        let generation_id = generation_id(
            "test-salt",
            "test-account-fingerprint",
            "test-window-fingerprint",
            at(1_000),
            sequence,
        );
        confirmed_event(
            "test-salt",
            generation_id,
            "test-window-fingerprint".to_owned(),
            1_000,
            reason,
            at(detected_at),
            at(1_000),
            Some(at(2_000)),
            previous_remaining_percent,
            current_remaining_percent,
        )
    }

    #[test]
    fn first_success_only_builds_a_baseline() {
        let mut tracker = NotificationTracker::new();
        let initial = [window("weekly", 10)];

        assert_eq!(
            evaluate(&mut tracker, &initial, 100, 12 * 60, &policy()),
            None
        );
        assert_eq!(
            evaluate(&mut tracker, &initial, 200, 12 * 60, &policy()),
            None
        );
    }

    #[test]
    fn first_success_still_notifies_a_reset_confirmed_on_that_refresh() {
        let mut tracker = NotificationTracker::new();
        let current = [window("weekly", 100)];
        let event = reset_event(1, 200, QuotaResetReason::QuotaRecovered, 99, 100);
        let matched = [MatchedQuotaResetEvent {
            window_index: 0,
            event: &event,
        }];

        let notification = evaluate_with_resets(
            &mut tracker,
            &current,
            200,
            12 * 60,
            &policy(),
            &matched,
            |_| false,
        )
        .expect("a reset newly confirmed after restart must not be swallowed by baseline");
        assert_eq!(
            notification.items[0].reasons,
            vec![NotificationReason::Reset]
        );

        // 识别器即使在下一刷新重复携带同一事件，通知侧也只能消费一次。
        assert_eq!(
            evaluate_with_resets(
                &mut tracker,
                &current,
                201,
                12 * 60,
                &policy(),
                &matched,
                |_| false,
            ),
            None
        );
    }

    #[test]
    fn low_remaining_fires_only_once_per_cycle_and_honors_boundaries() {
        let mut tracker = NotificationTracker::new();
        let mut current = [window("weekly", 21)];
        let mut rules = policy();
        rules.low_remaining_threshold = 20;
        evaluate(&mut tracker, &current, 100, 0, &rules);

        current[0].remaining_percent = 20;
        let notification = evaluate(&mut tracker, &current, 200, 0, &rules).unwrap();
        assert_eq!(
            notification.items[0].reasons,
            vec![NotificationReason::LowRemaining {
                remaining_percent: 20
            }]
        );

        current[0].remaining_percent = 0;
        assert_eq!(evaluate(&mut tracker, &current, 300, 0, &rules), None);

        let mut zero_tracker = NotificationTracker::new();
        let mut zero_window = [window("short", 1)];
        rules.low_remaining_threshold = 0;
        evaluate(&mut zero_tracker, &zero_window, 100, 0, &rules);
        zero_window[0].remaining_percent = 0;
        assert!(evaluate(&mut zero_tracker, &zero_window, 200, 0, &rules).is_some());
    }

    #[test]
    fn pace_deficit_fires_once_when_a_long_window_falls_behind() {
        let mut tracker = NotificationTracker::new();
        let mut current = [window("weekly", 80)];
        current[0].is_long_period = true;
        let rules = policy();
        evaluate(&mut tracker, &current, 200, 0, &rules); // suggested: 80

        current[0].remaining_percent = 49;
        let notification = evaluate(&mut tracker, &current, 400, 0, &rules).unwrap();
        assert_eq!(
            notification.items[0].reasons,
            vec![NotificationReason::PaceDeficit { deficit_points: 11 }]
        );

        current[0].remaining_percent = 25;
        assert_eq!(evaluate(&mut tracker, &current, 500, 0, &rules), None);
    }

    #[test]
    fn confirmed_reset_fires_once_and_rearms_thresholds_for_the_new_cycle() {
        let mut tracker = NotificationTracker::new();
        let mut current = [window("weekly", 80)];
        let rules = policy();
        evaluate(&mut tracker, &current, 100, 0, &rules);

        current[0].reset_at_unix_seconds = Some(2_000);
        current[0].start_at_unix_seconds = Some(1_000);
        current[0].remaining_percent = 100;
        let event = reset_event(1, 1_100, QuotaResetReason::DeadlineReached, 80, 100);
        let matched = [MatchedQuotaResetEvent {
            window_index: 0,
            event: &event,
        }];
        let notification =
            evaluate_with_resets(&mut tracker, &current, 1_100, 0, &rules, &matched, |_| {
                false
            })
            .unwrap();
        assert_eq!(
            notification.items[0].reasons,
            vec![NotificationReason::Reset]
        );
        assert_eq!(
            evaluate_with_resets(&mut tracker, &current, 1_200, 0, &rules, &matched, |_| {
                false
            },),
            None
        );

        current[0].remaining_percent = 20;
        assert!(evaluate(&mut tracker, &current, 1_300, 0, &rules).is_some());
    }

    #[test]
    fn reset_at_changes_alone_never_bypass_the_unified_recognizer() {
        let mut tracker = NotificationTracker::new();
        let mut current = [window("weekly", 80)];
        let rules = policy();
        evaluate(&mut tracker, &current, 100, 0, &rules);

        current[0].reset_at_unix_seconds = Some(1_100);
        current[0].start_at_unix_seconds = Some(100);
        current[0].remaining_percent = 100;
        assert_eq!(evaluate(&mut tracker, &current, 900, 0, &rules), None);

        // 即使截止时间真正过去，通知模块也等待统一识别器给出确认事件。
        current[0].reset_at_unix_seconds = Some(2_100);
        current[0].start_at_unix_seconds = Some(1_100);
        assert_eq!(evaluate(&mut tracker, &current, 1_100, 0, &rules), None);

        let event = reset_event(1, 1_100, QuotaResetReason::DeadlineReached, 80, 100);
        let matched = [MatchedQuotaResetEvent {
            window_index: 0,
            event: &event,
        }];
        assert_eq!(
            evaluate_with_resets(&mut tracker, &current, 1_100, 0, &rules, &matched, |_| {
                false
            },)
            .unwrap()
            .items[0]
                .reasons,
            vec![NotificationReason::Reset]
        );
    }

    #[test]
    fn same_reset_at_quota_recovery_from_98_or_99_to_100_is_notified() {
        for (sequence, previous) in [(1, 98), (2, 99)] {
            let mut tracker = NotificationTracker::new();
            let mut current = [window("weekly", previous)];
            let rules = policy();
            evaluate(&mut tracker, &current, 100, 0, &rules);

            // reset_at 保持不变；是否重置完全由统一事件决定。
            current[0].remaining_percent = 100;
            let event = reset_event(
                sequence,
                200,
                QuotaResetReason::QuotaRecovered,
                previous,
                100,
            );
            let matched = [MatchedQuotaResetEvent {
                window_index: 0,
                event: &event,
            }];
            let notification =
                evaluate_with_resets(&mut tracker, &current, 200, 0, &rules, &matched, |_| false)
                    .unwrap();
            assert_eq!(
                notification.items[0].reasons,
                vec![NotificationReason::Reset]
            );
        }
    }

    #[test]
    fn generation_dedupe_blocks_a_second_event_id_for_the_same_cycle() {
        let mut tracker = NotificationTracker::new();
        let mut current = [window("weekly", 99)];
        let rules = policy();
        evaluate(&mut tracker, &current, 100, 0, &rules);
        current[0].remaining_percent = 100;

        let first = reset_event(1, 200, QuotaResetReason::QuotaRecovered, 99, 100);
        let mut duplicate_generation = first.clone();
        duplicate_generation.event_id = "different-event-id".to_owned();
        duplicate_generation.detected_at = at(201);
        let first_match = [MatchedQuotaResetEvent {
            window_index: 0,
            event: &first,
        }];
        assert!(
            evaluate_with_resets(&mut tracker, &current, 200, 0, &rules, &first_match, |_| {
                false
            },)
            .is_some()
        );

        let duplicate_match = [MatchedQuotaResetEvent {
            window_index: 0,
            event: &duplicate_generation,
        }];
        assert_eq!(
            evaluate_with_resets(
                &mut tracker,
                &current,
                201,
                0,
                &rules,
                &duplicate_match,
                |_| false,
            ),
            None
        );
    }

    #[test]
    fn settings_baseline_does_not_forget_consumed_reset_identity() {
        let mut tracker = NotificationTracker::new();
        let current = [window("weekly", 100)];
        let first = reset_event(1, 200, QuotaResetReason::QuotaRecovered, 99, 100);
        let first_match = [MatchedQuotaResetEvent {
            window_index: 0,
            event: &first,
        }];
        assert!(evaluate_with_resets(
            &mut tracker,
            &current,
            200,
            0,
            &policy(),
            &first_match,
            |_| false,
        )
        .is_some());

        tracker.reset_baseline();
        assert_eq!(
            evaluate_with_resets(
                &mut tracker,
                &current,
                201,
                0,
                &policy(),
                &first_match,
                |_| false,
            ),
            None
        );

        // 新一轮基线刷新中的新事件仍必须透传，不能把所有 reset 都当成历史。
        tracker.reset_baseline();
        let next = reset_event(2, 202, QuotaResetReason::QuotaRecovered, 99, 100);
        let next_match = [MatchedQuotaResetEvent {
            window_index: 0,
            event: &next,
        }];
        assert!(evaluate_with_resets(
            &mut tracker,
            &current,
            202,
            0,
            &policy(),
            &next_match,
            |_| false,
        )
        .is_some());
    }

    #[test]
    fn multiple_windows_are_merged_and_limited_to_three_items() {
        let mut tracker = NotificationTracker::new();
        let mut current = [
            window("one", 21),
            window("two", 21),
            window("three", 21),
            window("four", 21),
        ];
        let rules = policy();
        evaluate(&mut tracker, &current, 100, 0, &rules);
        for item in &mut current {
            item.remaining_percent = 20;
        }

        let notification = evaluate(&mut tracker, &current, 200, 0, &rules).unwrap();
        assert_eq!(
            notification
                .items
                .iter()
                .map(|item| item.window_index)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(notification.omitted_window_count, 1);
    }

    #[test]
    fn cross_midnight_quiet_hours_consume_events_without_replay() {
        let quiet_hours = QuietHours {
            enabled: true,
            start_minute: 22 * 60,
            end_minute: 8 * 60,
        };
        assert!(quiet_hours.contains(22 * 60));
        assert!(quiet_hours.contains(23 * 60));
        assert!(quiet_hours.contains(7 * 60 + 59));
        assert!(!quiet_hours.contains(8 * 60));
        assert!(!quiet_hours.contains(12 * 60));

        let mut rules = policy();
        rules.quiet_hours = quiet_hours;
        let mut tracker = NotificationTracker::new();
        let mut current = [window("weekly", 21)];
        evaluate(&mut tracker, &current, 100, 21 * 60, &rules);
        current[0].remaining_percent = 20;

        assert_eq!(evaluate(&mut tracker, &current, 200, 23 * 60, &rules), None);
        assert_eq!(evaluate(&mut tracker, &current, 300, 9 * 60, &rules), None);
    }

    #[test]
    fn reset_that_occurred_during_quiet_hours_is_not_replayed_after_outage() {
        let mut tracker = NotificationTracker::new();
        let mut current = [window("weekly", 80)];
        let rules = policy();
        evaluate(&mut tracker, &current, 100, 12 * 60, &rules);

        current[0].reset_at_unix_seconds = Some(2_000);
        current[0].start_at_unix_seconds = Some(1_000);
        current[0].remaining_percent = 100;
        let event = reset_event(1, 1_000, QuotaResetReason::DeadlineReached, 80, 100);
        let matched = [MatchedQuotaResetEvent {
            window_index: 0,
            event: &event,
        }];
        assert_eq!(
            evaluate_with_resets(
                &mut tracker,
                &current,
                1_100,
                8 * 60 + 10,
                &rules,
                &matched,
                |event_at| event_at == 1_000,
            ),
            None
        );
        assert_eq!(
            evaluate_with_resets(
                &mut tracker,
                &current,
                1_101,
                8 * 60 + 11,
                &rules,
                &matched,
                |_| false,
            ),
            None
        );
    }

    #[test]
    fn reset_observed_while_current_quiet_hours_are_active_is_consumed() {
        let mut rules = policy();
        rules.quiet_hours = QuietHours {
            enabled: true,
            start_minute: 22 * 60,
            end_minute: 8 * 60,
        };
        let mut tracker = NotificationTracker::new();
        let mut current = [window("weekly", 99)];
        evaluate(&mut tracker, &current, 100, 21 * 60, &rules);
        current[0].remaining_percent = 100;
        let event = reset_event(1, 200, QuotaResetReason::QuotaRecovered, 99, 100);
        let matched = [MatchedQuotaResetEvent {
            window_index: 0,
            event: &event,
        }];

        assert_eq!(
            evaluate_with_resets(
                &mut tracker,
                &current,
                200,
                23 * 60,
                &rules,
                &matched,
                |_| false,
            ),
            None
        );
        assert_eq!(
            evaluate_with_resets(
                &mut tracker,
                &current,
                201,
                9 * 60,
                &rules,
                &matched,
                |_| false,
            ),
            None
        );
    }

    #[test]
    fn equal_quiet_hour_endpoints_mean_an_empty_interval() {
        let mut rules = policy();
        rules.quiet_hours = QuietHours {
            enabled: true,
            start_minute: 8 * 60,
            end_minute: 8 * 60,
        };
        assert!(!rules.quiet_hours.contains(8 * 60));

        let mut tracker = NotificationTracker::new();
        let mut current = [window("weekly", 21)];
        evaluate(&mut tracker, &current, 100, 8 * 60, &rules);
        current[0].remaining_percent = 20;
        assert!(evaluate(&mut tracker, &current, 200, 8 * 60, &rules).is_some());
    }

    #[test]
    fn reset_baseline_suppresses_events_already_reached_under_new_settings() {
        let mut tracker = NotificationTracker::new();
        let mut current = [window("weekly", 30)];
        let mut rules = policy();
        rules.low_remaining_threshold = 20;
        evaluate(&mut tracker, &current, 100, 0, &rules);

        current[0].remaining_percent = 25;
        rules.low_remaining_threshold = 30;
        tracker.reset_baseline();
        assert_eq!(evaluate(&mut tracker, &current, 200, 0, &rules), None);
        current[0].remaining_percent = 20;
        assert_eq!(evaluate(&mut tracker, &current, 300, 0, &rules), None);
    }

    #[test]
    fn disabled_policy_always_requires_a_fresh_baseline_when_reenabled() {
        let mut tracker = NotificationTracker::new();
        let mut current = [window("weekly", 30)];
        let enabled = policy();
        evaluate(&mut tracker, &current, 100, 0, &enabled);
        current[0].remaining_percent = 20;

        let disabled = NotificationPolicy::default();
        assert_eq!(evaluate(&mut tracker, &current, 200, 0, &disabled), None);
        assert_eq!(evaluate(&mut tracker, &current, 300, 0, &enabled), None);
    }
}
