//! Deterministic refresh scheduling primitives shared by automatic and manual refreshes.

const FAILURE_BACKOFF_SECONDS: [u64; 5] = [60, 180, 300, 600, 1_800];

/// `consecutive_failures` is one-based. After the fifth failure the retry delay remains 30 min.
pub fn failure_retry_seconds(consecutive_failures: u32) -> u64 {
    let index = consecutive_failures.saturating_sub(1) as usize;
    FAILURE_BACKOFF_SECONDS[index.min(FAILURE_BACKOFF_SECONDS.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_locked_failure_backoff_sequence() {
        let actual: Vec<u64> = (1..=8).map(failure_retry_seconds).collect();
        assert_eq!(actual, vec![60, 180, 300, 600, 1_800, 1_800, 1_800, 1_800]);
    }

    #[test]
    fn zero_is_safe_for_diagnostics_and_startup_edges() {
        assert_eq!(failure_retry_seconds(0), 60);
    }
}
