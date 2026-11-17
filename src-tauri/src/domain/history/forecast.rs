use super::*;
impl UsageHistory {
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
}
impl StoredUsageStream {
    pub(super) fn forecast(&self, now: DateTime<Utc>) -> Forecast {
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
