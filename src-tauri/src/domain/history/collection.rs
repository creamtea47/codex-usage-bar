use super::*;

impl UsageHistory {
    /// 只应在成功获取用量快照后调用。该方法先完成账号隔离，再依据采样策略写入。
    #[allow(dead_code)] // 保留 v0.4.0 调用兼容与旧历史启发式回归测试。
    pub fn record_successful_snapshot(
        &mut self,
        identity: AccountIdentity<'_>,
        sampled_at: DateTime<Utc>,
        windows: &[HistoryWindowInput],
    ) -> Result<HistoryMutation, FingerprintError> {
        self.record_successful_snapshot_with_generations(identity, sampled_at, windows, &[])
    }

    /// 使用统一重置识别器的代次记录成功快照。只要同一额度窗口的代次发生变化，
    /// 即使 `reset_at` 没变、额度只从 98/99% 恢复到 100%，趋势也会可靠断开。
    ///
    /// `generations` 是可选旁路输入，保留旧调用方的兼容行为；无代次时仍使用
    /// 原有的 `reset_at`/额度回升推断，便于升级期间连续采样。
    pub fn record_successful_snapshot_with_generations(
        &mut self,
        identity: AccountIdentity<'_>,
        sampled_at: DateTime<Utc>,
        windows: &[HistoryWindowInput],
        generations: &[HistoryGenerationInput],
    ) -> Result<HistoryMutation, FingerprintError> {
        let mut mutation = HistoryMutation::default();
        match self.select_account(identity)? {
            AccountSelection::Initialized => mutation.account_changed = true,
            AccountSelection::Unchanged => {}
            AccountSelection::Switched { .. } => mutation.account_changed = true,
        }

        mutation.samples_pruned = self.compact_at(sampled_at);
        let generation_by_window = generations
            .iter()
            .filter(|generation| valid_generation_input(generation))
            .map(|generation| {
                (
                    (generation.window_id.as_str(), generation.window_seconds),
                    generation.generation_id.as_str(),
                )
            })
            .collect::<HashMap<_, _>>();
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
            let generation_id = generation_by_window
                .get(&(window.window_id.as_str(), window.window_seconds))
                .copied();

            let stream_index = self
                .streams
                .iter()
                .position(|stream| stream.matches(&stored_window));
            let (recorded, generation_metadata_updated) = if let Some(index) = stream_index {
                self.streams[index].record(sampled_at, &stored_window, generation_id)
            } else {
                self.streams.push(StoredUsageStream::from_first_sample(
                    sampled_at,
                    &stored_window,
                    generation_id,
                ));
                (true, false)
            };
            mutation.generation_metadata_updated |= generation_metadata_updated;

            if recorded {
                mutation.samples_recorded += 1;
            }
        }
        Ok(mutation)
    }

    /// 用户清除历史时只清当前账号，并保留随机盐和账号指纹，避免影响其他分区。
    pub fn clear_samples(&mut self) -> usize {
        let removed = self.sample_count();
        self.streams.clear();
        removed
    }

    pub fn compact_at(&mut self, now: DateTime<Utc>) -> usize {
        let mut removed = 0;
        for stream in &mut self.streams {
            removed += stream.compact_for_age(now);
        }
        for account in &mut self.accounts {
            for stream in &mut account.streams {
                removed += stream.compact_for_age(now);
            }
        }
        removed
    }
}

impl StoredUsageStream {
    fn from_first_sample(
        sampled_at: DateTime<Utc>,
        window: &HistoryWindowInput,
        generation_id: Option<&str>,
    ) -> Self {
        Self {
            window_id: window.window_id.clone(),
            window_seconds: window.window_seconds,
            cycles: vec![StoredUsageCycle {
                first_exhausted_at: None,
                generation_id: generation_id.map(ToOwned::to_owned),
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

    fn record(
        &mut self,
        sampled_at: DateTime<Utc>,
        window: &HistoryWindowInput,
        generation_id: Option<&str>,
    ) -> (bool, bool) {
        let Some(latest_at) = self.latest_sample_at() else {
            *self = Self::from_first_sample(sampled_at, window, generation_id);
            return (true, false);
        };
        // 系统时间倒退时跳过该点，避免把当前周期写成非单调时间序列。
        if sampled_at < latest_at {
            return (false, false);
        }

        let cycle = self
            .cycles
            .last_mut()
            .expect("non-empty stream must contain a cycle");
        let last = cycle
            .samples
            .last()
            .expect("non-empty cycle must contain a sample");
        let generation_changed = cycle
            .generation_id
            .as_deref()
            .zip(generation_id)
            .is_some_and(|(previous, current)| previous != current);
        // 一旦调用层提供统一代次，它就是唯一的分段依据；继续叠加旧启发式会让
        // 同一代次内的 reset_at 校正或 99↔100 抖动制造重复断点。
        let reset_advanced = generation_id.is_none()
            && match (cycle.reset_at, window.cycle_reset_at) {
                (Some(previous), Some(next)) => next > previous && sampled_at >= previous,
                _ => false,
            };
        let inferred_reset = generation_id.is_none()
            && window.remaining_percent > last.remaining_percent
            && (cycle.reset_at.is_none()
                || cycle
                    .reset_at
                    .is_some_and(|known_reset| sampled_at >= known_reset));

        if generation_changed || reset_advanced || inferred_reset {
            self.cycles.push(StoredUsageCycle {
                first_exhausted_at: None,
                generation_id: generation_id.map(ToOwned::to_owned),
                reset_at: window.cycle_reset_at,
                samples: vec![StoredUsageSample {
                    sampled_at,
                    remaining_percent: window.remaining_percent,
                }],
            });
            return (true, false);
        }

        let generation_metadata_updated = cycle.generation_id.is_none() && generation_id.is_some();
        if generation_metadata_updated {
            // 从 v0.4.0 历史平滑迁移：首次获知代次只补元数据，不把升级时刻伪装成重置。
            cycle.generation_id = generation_id.map(ToOwned::to_owned);
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
                return (true, generation_metadata_updated);
            }
            return (false, generation_metadata_updated);
        }

        let interval_elapsed =
            sampled_at - last.sampled_at >= Duration::minutes(SAMPLE_INTERVAL_MINUTES);
        let percentage_changed = last.remaining_percent != window.remaining_percent;
        if !interval_elapsed && !percentage_changed {
            return (false, generation_metadata_updated);
        }
        cycle.samples.push(StoredUsageSample {
            sampled_at,
            remaining_percent: window.remaining_percent,
        });
        (true, generation_metadata_updated)
    }

    pub(super) fn compact_for_age(&mut self, now: DateTime<Utc>) -> usize {
        let recent_cutoff = safe_subtract(now, Duration::hours(24));
        let medium_cutoff = safe_subtract(now, Duration::days(7));
        let daily_cutoff = safe_subtract(now, Duration::days(32));
        let mut removed = 0;
        for cycle in &mut self.cycles {
            if cycle.first_exhausted_at.is_none() {
                cycle.first_exhausted_at = cycle
                    .samples
                    .iter()
                    .find(|s| s.remaining_percent == 0)
                    .map(|s| s.sampled_at);
            }
            let compacted =
                compact_cycle_for_age(&cycle.samples, daily_cutoff, medium_cutoff, recent_cutoff);
            removed += cycle.samples.len().saturating_sub(compacted.len());
            cycle.samples = compacted;
        }
        removed
    }
}
pub(super) fn compact_cycle_for_age(
    samples: &[StoredUsageSample],
    daily_cutoff: DateTime<Utc>,
    medium_cutoff: DateTime<Utc>,
    recent_cutoff: DateTime<Utc>,
) -> Vec<StoredUsageSample> {
    if samples.len() <= 2 {
        return samples.to_vec();
    }

    let first_at = samples[0].sampled_at;
    let last_at = samples[samples.len() - 1].sampled_at;
    let mut selected = Vec::with_capacity(samples.len());
    let mut tier_start = 0;
    while tier_start < samples.len() {
        let sampled_at = samples[tier_start].sampled_at;
        let (tier_end, bucket_seconds) = if sampled_at < daily_cutoff {
            let tier_end = samples[tier_start..]
                .iter()
                .position(|sample| sample.sampled_at >= daily_cutoff)
                .map(|offset| tier_start + offset)
                .unwrap_or(samples.len());
            (tier_end, Some(DAILY_BUCKET_SECONDS))
        } else if sampled_at < medium_cutoff {
            let tier_end = samples[tier_start..]
                .iter()
                .position(|sample| sample.sampled_at >= medium_cutoff)
                .map(|offset| tier_start + offset)
                .unwrap_or(samples.len());
            (tier_end, Some(MONTH_BUCKET_SECONDS))
        } else if sampled_at < recent_cutoff {
            let tier_end = samples[tier_start..]
                .iter()
                .position(|sample| sample.sampled_at >= recent_cutoff)
                .map(|offset| tier_start + offset)
                .unwrap_or(samples.len());
            (tier_end, Some(SEVEN_DAY_BUCKET_SECONDS))
        } else {
            // 最新层不需要哨兵。直接消费余下样本，避免合法 MAX_UTC 样本让
            // tier_end == tier_start 而无法推进循环。
            (samples.len(), None)
        };
        if let Some(bucket_seconds) = bucket_seconds {
            let references = samples[tier_start..tier_end].iter().collect::<Vec<_>>();
            selected.extend(
                downsample_cycle(&references, bucket_seconds)
                    .into_iter()
                    .cloned(),
            );
        } else {
            selected.extend_from_slice(&samples[tier_start..tier_end]);
        }
        tier_start = tier_end;
    }

    // Tier 边界可能让同一点被相邻分段同时选中。排序去重后再次固定保留 cycle 首尾，
    // 这样任何压缩都不会丢掉重置前后的边界点。
    selected.sort_by_key(|sample| sample.sampled_at);
    selected.dedup_by_key(|sample| sample.sampled_at);
    if selected
        .first()
        .is_none_or(|sample| sample.sampled_at != first_at)
    {
        selected.insert(0, samples[0].clone());
    }
    if selected
        .last()
        .is_none_or(|sample| sample.sampled_at != last_at)
    {
        selected.push(samples[samples.len() - 1].clone());
    }
    selected
}
