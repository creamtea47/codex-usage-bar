use super::*;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RangeSummary {
    pub start_at: Option<DateTime<Utc>>,
    pub end_at: Option<DateTime<Utc>>,
    pub duration_seconds: i64,
    pub first_remaining_percent: Option<u8>,
    pub last_remaining_percent: Option<u8>,
    pub consumed_percent: u32,
    pub cycle_count: usize,
    pub sample_count: usize,
    pub partial: bool,
    pub bucket_seconds: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CycleSummary {
    pub cycle_id: String,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub reset_at: Option<DateTime<Utc>>,
    pub first_remaining_percent: u8,
    pub last_remaining_percent: u8,
    pub consumed_percent: u8,
    pub duration_seconds: i64,
    pub sample_count: usize,
    pub partial: bool,
    pub outside_chart_range: bool,
    pub current: bool,
    pub first_exhausted_at: Option<DateTime<Utc>>,
    pub bucket_seconds: i64,
    pub forecast: Option<Forecast>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CycleSamples {
    pub points: Vec<UsageHistoryPoint>,
    pub total: usize,
    pub offset: usize,
    pub bucket_seconds: i64,
}

fn precision(start: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    if start < now - Duration::days(32) {
        86400
    } else if start < now - Duration::days(7) {
        3600
    } else if start < now - Duration::hours(24) {
        900
    } else {
        0
    }
}

fn cycle_id(cycle: &StoredUsageCycle) -> String {
    cycle
        .generation_id
        .clone()
        .unwrap_or_else(|| format!("legacy-{}", cycle.samples[0].sampled_at.timestamp_millis()))
}

impl UsageHistory {
    /// 查询前由集成层选定账号。窗口 ID 是已有匿名流键，只能匹配该账号自己的流。
    fn analytics_stream(&self, window_id: &str) -> Result<&StoredUsageStream, String> {
        self.streams
            .iter()
            .find(|v| v.window_id == window_id)
            .ok_or_else(|| "historyWindowMissing".into())
    }

    pub fn range_summary(
        &self,
        window_id: &str,
        a: DateTime<Utc>,
        b: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<RangeSummary, String> {
        let stream = self.analytics_stream(window_id)?;
        let (start, end) = (a.min(b), a.max(b).min(now));
        if start > end {
            return Err("invalidRange".into());
        }
        let mut result = RangeSummary {
            start_at: None,
            end_at: None,
            duration_seconds: 0,
            first_remaining_percent: None,
            last_remaining_percent: None,
            consumed_percent: 0,
            cycle_count: 0,
            sample_count: 0,
            partial: false,
            bucket_seconds: precision(start, now),
        };
        for cycle in &stream.cycles {
            let samples: Vec<_> = cycle
                .samples
                .iter()
                .filter(|s| s.sampled_at >= start && s.sampled_at <= end)
                .collect();
            let Some((first, last)) = samples.first().zip(samples.last()) else {
                continue;
            };
            if result.start_at.is_none() {
                result.start_at = Some(first.sampled_at);
                result.first_remaining_percent = Some(first.remaining_percent);
            }
            result.end_at = Some(last.sampled_at);
            result.last_remaining_percent = Some(last.remaining_percent);
            // 每个真实周期单独相减，不能让补满额度抵消旧周期的消耗。
            result.consumed_percent += u32::from(
                first
                    .remaining_percent
                    .saturating_sub(last.remaining_percent),
            );
            result.cycle_count += 1;
            result.sample_count += samples.len();
        }
        result.duration_seconds = result
            .end_at
            .zip(result.start_at)
            .map_or(0, |(e, s)| (e - s).num_seconds());
        result.partial = result.start_at != Some(start)
            || result.end_at != Some(end)
            || result.bucket_seconds > 0;
        Ok(result)
    }

    pub fn cycle_summaries(
        &self,
        window_id: &str,
        request: UsageHistoryRequest,
        now: DateTime<Utc>,
    ) -> Result<Vec<CycleSummary>, String> {
        let stream = self.analytics_stream(window_id)?;
        let range = request
            .resolve(now, self.summary_available_at(now).oldest_sample_at)
            .map_err(|_| "invalidRange")?;
        Ok(stream
            .cycles
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(index, cycle)| {
                let (first, last) = cycle.samples.first().zip(cycle.samples.last())?;
                if last.sampled_at < range.start_at || first.sampled_at >= range.end_at_exclusive {
                    return None;
                }
                let current = index + 1 == stream.cycles.len();
                Some(CycleSummary {
                    cycle_id: cycle_id(cycle),
                    start_at: first.sampled_at,
                    end_at: last.sampled_at,
                    reset_at: cycle.reset_at,
                    first_remaining_percent: first.remaining_percent,
                    last_remaining_percent: last.remaining_percent,
                    consumed_percent: first
                        .remaining_percent
                        .saturating_sub(last.remaining_percent),
                    duration_seconds: (last.sampled_at - first.sampled_at).num_seconds(),
                    sample_count: cycle.samples.len(),
                    partial: first.remaining_percent != 100,
                    outside_chart_range: first.sampled_at < range.start_at
                        || last.sampled_at >= range.end_at_exclusive,
                    current,
                    first_exhausted_at: cycle.first_exhausted_at.or_else(|| {
                        cycle
                            .samples
                            .iter()
                            .find(|s| s.remaining_percent == 0)
                            .map(|s| s.sampled_at)
                    }),
                    bucket_seconds: precision(first.sampled_at, now),
                    forecast: current.then(|| stream.forecast(now)),
                })
            })
            .collect())
    }

    pub fn cycle_samples(
        &self,
        window_id: &str,
        id: &str,
        offset: usize,
        now: DateTime<Utc>,
    ) -> Result<CycleSamples, String> {
        let cycle = self
            .analytics_stream(window_id)?
            .cycles
            .iter()
            .find(|v| cycle_id(v) == id)
            .ok_or("historyCycleMissing")?;
        let offset = offset.min(cycle.samples.len());
        Ok(CycleSamples {
            total: cycle.samples.len(),
            offset,
            bucket_seconds: precision(cycle.samples[0].sampled_at, now),
            points: cycle
                .samples
                .iter()
                .enumerate()
                .skip(offset)
                .take(50)
                .map(|(i, s)| UsageHistoryPoint {
                    sampled_at: s.sampled_at,
                    remaining_percent: s.remaining_percent,
                    break_before: i == 0,
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compaction_keeps_first_empty_fact_and_pagination_is_bounded() {
        let at = Utc::now() - Duration::days(40);
        let mut h = UsageHistory::with_salt("ab".repeat(32)).unwrap();
        h.select_account(AccountIdentity::AccountId("one")).unwrap();
        h.streams.push(StoredUsageStream {
            window_id: "ab".repeat(32),
            window_seconds: 604800,
            cycles: vec![StoredUsageCycle {
                generation_id: None,
                first_exhausted_at: None,
                reset_at: Some(at + Duration::days(7)),
                samples: vec![(0, 100), (1, 0), (2, 30), (3, 20)]
                    .into_iter()
                    .map(|(hours, remaining_percent)| StoredUsageSample {
                        sampled_at: at + Duration::hours(hours),
                        remaining_percent,
                    })
                    .collect(),
            }],
        });
        h.compact_at(Utc::now());
        let key = h.streams[0].window_id.clone();
        let row = h
            .cycle_summaries(
                &key,
                UsageHistoryRequest::Preset {
                    preset: UsageHistoryPreset::All,
                },
                Utc::now(),
            )
            .unwrap()
            .remove(0);
        assert_eq!(row.first_exhausted_at, Some(at + Duration::hours(1)));
        assert_eq!(row.consumed_percent, 80);
        assert!(row.sample_count < 4);
        let samples = h
            .cycle_samples(&key, &row.cycle_id, usize::MAX, Utc::now())
            .unwrap();
        assert!(samples.points.is_empty());
        assert_eq!(samples.offset, samples.total);
    }
    #[test]
    fn range_sums_cycles_and_supports_reverse_and_same_point() {
        let now = Utc::now();
        let mut h = UsageHistory::with_salt("ab".repeat(32)).unwrap();
        for (i, percent, generation) in
            [(0, 80, "aa"), (1, 20, "aa"), (2, 100, "bb"), (3, 70, "bb")]
        {
            h.record_successful_snapshot_with_generations(
                AccountIdentity::AccountId("one"),
                now + Duration::minutes(i),
                &[HistoryWindowInput {
                    window_id: "weekly".into(),
                    window_seconds: 604800,
                    cycle_reset_at: Some(now + Duration::days(7)),
                    remaining_percent: percent,
                }],
                &[HistoryGenerationInput {
                    window_id: "weekly".into(),
                    window_seconds: 604800,
                    generation_id: generation.repeat(32),
                }],
            )
            .unwrap();
        }
        let key = h.streams[0].window_id.clone();
        let end = now + Duration::minutes(3);
        let summary = h.range_summary(&key, end, now, end).unwrap();
        assert_eq!((summary.consumed_percent, summary.cycle_count), (90, 2));
        assert_eq!(
            h.range_summary(&key, now, now, end)
                .unwrap()
                .consumed_percent,
            0
        );
        assert_eq!(
            h.cycle_summaries(
                &key,
                UsageHistoryRequest::Preset {
                    preset: UsageHistoryPreset::All
                },
                end
            )
            .unwrap()
            .len(),
            2
        );
    }
}
