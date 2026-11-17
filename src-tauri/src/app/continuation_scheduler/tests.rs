use crate::app::continuation_scheduler::quota_auto_continue_wait_duration;
use crate::app::continuation_scheduler::QUOTA_AUTO_CONTINUE_IDLE_WAIT;
use chrono::DateTime;
use chrono::Duration as ChronoDuration;
use std::time::Duration;

#[test]
fn quota_auto_continue_wait_distinguishes_idle_future_and_due_tasks() {
    let now = DateTime::from_timestamp(1_000, 500_000_000).unwrap();

    assert_eq!(
        quota_auto_continue_wait_duration(None, now),
        QUOTA_AUTO_CONTINUE_IDLE_WAIT
    );
    assert_eq!(
        quota_auto_continue_wait_duration(Some(now + ChronoDuration::seconds(30)), now),
        Duration::from_secs(30)
    );
    assert_eq!(
        quota_auto_continue_wait_duration(Some(now), now),
        Duration::ZERO
    );
    assert_eq!(
        quota_auto_continue_wait_duration(Some(now - ChronoDuration::nanoseconds(1)), now),
        Duration::ZERO
    );
}
