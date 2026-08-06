//! Pure notification-decision state machine.
//!
//! This module intentionally knows nothing about Tauri or platform notification
//! APIs. The caller converts a successful dashboard snapshot into
//! [`NotificationSnapshot`], passes the current policy to
//! [`NotificationTracker::evaluate`], and localizes/sends the returned batch.

use std::collections::HashMap;

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
    /// Greatest valid reset deadline observed for this key. Keeping the maximum
    /// avoids treating a temporary backwards correction followed by recovery as
    /// a real quota reset.
    latest_reset_at_unix_seconds: Option<i64>,
    low_remaining_handled: bool,
    pace_deficit_handled: bool,
}

/// Stateful rule evaluator. One tracker should be kept for the active account.
#[derive(Debug)]
pub struct NotificationTracker {
    baseline_pending: bool,
    windows: HashMap<String, WindowState>,
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
        }
    }

    /// Forget all prior generations. Call this after notification thresholds,
    /// switches, quiet hours, or language change. The next successful snapshot
    /// becomes a baseline and never emits historical notifications.
    pub fn reset_baseline(&mut self) {
        self.windows.clear();
        self.baseline_pending = true;
    }

    /// Evaluate one successful snapshot and return at most one merged batch.
    ///
    /// Events reached during quiet hours are still marked handled, so they are
    /// never replayed when quiet hours end. Newly appearing windows establish
    /// their own baseline and do not alert immediately.
    pub fn evaluate<F>(
        &mut self,
        snapshot: &NotificationSnapshot<'_>,
        policy: &NotificationPolicy,
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

        if self.baseline_pending {
            self.windows.clear();
            for window in snapshot.windows {
                self.windows.insert(
                    window.key.clone(),
                    baseline_state(window, snapshot.observed_at_unix_seconds, policy),
                );
            }
            self.baseline_pending = false;
            return None;
        }

        let low_threshold = policy.low_remaining_threshold.min(100);
        let pace_threshold = policy.pace_deficit_threshold.min(100);
        let mut triggered = Vec::new();

        for (window_index, window) in snapshot.windows.iter().enumerate() {
            let remaining_percent = window.remaining_percent.min(100);
            let Some(state) = self.windows.get_mut(&window.key) else {
                self.windows.insert(
                    window.key.clone(),
                    baseline_state(window, snapshot.observed_at_unix_seconds, policy),
                );
                continue;
            };

            let mut reasons = Vec::new();
            let previous_reset_at = state.latest_reset_at_unix_seconds;
            if advances_reset_deadline(
                previous_reset_at,
                window.reset_at_unix_seconds,
                snapshot.observed_at_unix_seconds,
            ) {
                state.latest_reset_at_unix_seconds = window.reset_at_unix_seconds;
                state.low_remaining_handled = false;
                state.pace_deficit_handled = false;
                if policy.reset_enabled && !previous_reset_at.is_some_and(&reset_event_is_quiet) {
                    reasons.push(NotificationReason::Reset);
                }
            } else if let Some(reset_at) = window.reset_at_unix_seconds {
                state.latest_reset_at_unix_seconds = Some(
                    state
                        .latest_reset_at_unix_seconds
                        .map_or(reset_at, |known| known.max(reset_at)),
                );
            }

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

            if !reasons.is_empty() {
                triggered.push(NotificationItem {
                    window_index,
                    reasons,
                });
            }
        }

        // A removed stream is treated as new if it later returns, preventing a
        // stale generation from producing an immediate alert after API changes.
        self.windows
            .retain(|key, _| snapshot.windows.iter().any(|window| &window.key == key));

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
        latest_reset_at_unix_seconds: window.reset_at_unix_seconds,
        low_remaining_handled: remaining_percent <= low_threshold,
        pace_deficit_handled: pace_already_reached,
    }
}

fn advances_reset_deadline(
    previous: Option<i64>,
    current: Option<i64>,
    observed_at_unix_seconds: i64,
) -> bool {
    matches!(
        (previous, current),
        (Some(previous), Some(current))
            if current > previous && observed_at_unix_seconds >= previous
    )
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
    fn advancing_reset_deadline_fires_once_and_starts_a_new_cycle() {
        let mut tracker = NotificationTracker::new();
        let mut current = [window("weekly", 80)];
        let rules = policy();
        evaluate(&mut tracker, &current, 100, 0, &rules);

        current[0].reset_at_unix_seconds = Some(2_000);
        current[0].start_at_unix_seconds = Some(1_000);
        current[0].remaining_percent = 100;
        let notification = evaluate(&mut tracker, &current, 1_100, 0, &rules).unwrap();
        assert_eq!(
            notification.items[0].reasons,
            vec![NotificationReason::Reset]
        );
        assert_eq!(evaluate(&mut tracker, &current, 1_200, 0, &rules), None);

        current[0].remaining_percent = 20;
        assert!(evaluate(&mut tracker, &current, 1_300, 0, &rules).is_some());
    }

    #[test]
    fn forward_deadline_correction_before_expiry_is_not_a_reset() {
        let mut tracker = NotificationTracker::new();
        let mut current = [window("weekly", 80)];
        let rules = policy();
        evaluate(&mut tracker, &current, 100, 0, &rules);

        current[0].reset_at_unix_seconds = Some(1_100);
        current[0].start_at_unix_seconds = Some(100);
        current[0].remaining_percent = 100;
        assert_eq!(evaluate(&mut tracker, &current, 900, 0, &rules), None);

        // 修正后的截止时间真正过去、服务端进入下一周期后才提醒。
        current[0].reset_at_unix_seconds = Some(2_100);
        current[0].start_at_unix_seconds = Some(1_100);
        assert_eq!(
            evaluate(&mut tracker, &current, 1_100, 0, &rules)
                .unwrap()
                .items[0]
                .reasons,
            vec![NotificationReason::Reset]
        );
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
        let after_quiet_hours = NotificationSnapshot {
            observed_at_unix_seconds: 1_100,
            local_minute_of_day: 8 * 60 + 10,
            windows: &current,
        };
        assert_eq!(
            tracker.evaluate(&after_quiet_hours, &rules, |event_at| event_at == 1_000),
            None
        );
        assert_eq!(
            tracker.evaluate(&after_quiet_hours, &rules, |_| false),
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
