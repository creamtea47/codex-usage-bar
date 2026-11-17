use super::*;
impl UsageHistory {
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
                    current_reset_at: stream.cycles.last().and_then(|c| c.reset_at),
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

    pub(super) fn summary_between(
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

    pub(super) fn summary_available_at(&self, now: DateTime<Utc>) -> UsageHistorySummary {
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
}
impl StoredUsageStream {
    pub(super) fn suggested_remaining(
        &self,
        reset: Option<DateTime<Utc>>,
        sampled_at: DateTime<Utc>,
    ) -> Option<u8> {
        if !crate::domain::usage::supports_pace(self.window_seconds) {
            return None;
        }
        let remaining = (reset? - sampled_at).num_milliseconds() as f64;
        Some(
            (remaining / (self.window_seconds as f64 * 1000.0) * 100.0)
                .clamp(0.0, 100.0)
                .round() as u8,
        )
    }

    pub(super) fn query_points(
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
                    suggested_remaining_percent: self
                        .suggested_remaining(cycle.reset_at, sample.sampled_at),
                    break_before: index == 0,
                });
            }
        }
        bound_query_points(points, MAX_QUERY_POINTS_PER_SERIES)
    }

    pub(super) fn current_remaining_percent(&self, now: DateTime<Utc>) -> Option<u8> {
        self.cycles
            .iter()
            .flat_map(|cycle| &cycle.samples)
            .rev()
            .find(|sample| sample.sampled_at <= now)
            .map(|sample| sample.remaining_percent)
    }

    /// 以自然日边界附近最后一个已知额度作为基线。每个额度周期单独计算，避免重置
    /// 造成剩余百分比回升时把当天已经发生的消耗抵消掉。
    pub(super) fn consumed_between(&self, started_at: DateTime<Utc>, now: DateTime<Utc>) -> u32 {
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
}
pub(super) fn downsample_cycle<'a>(
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

pub(super) fn bound_query_points(
    points: Vec<UsageHistoryPoint>,
    maximum: usize,
) -> Vec<UsageHistoryPoint> {
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
