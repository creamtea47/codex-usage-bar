import { historyApi } from './historyApi';
import type { RangeSummary } from '@/contracts/history';
import { useEffect, useState } from 'react';

import type { UsageHistorySeries } from '@/contracts/history';

import { validHistoryPoints } from './chartData';
export function useRangeSelection(series: UsageHistorySeries, accountId: string | null) {
  const [selection, setSelection] = useState<number[]>([]);
  const [cursor, setCursor] = useState(0);
  const [rangeSummary, setRangeSummary] = useState<RangeSummary | null>(null);
  const [rangeError, setRangeError] = useState(false);
  const start = selection.length === 2 ? Math.min(...selection) : null;
  const end = selection.length === 2 ? Math.max(...selection) : null;
  useEffect(() => {
    if (start === null || end === null) return;
    let disposed = false;
    // Date.parse 仅用于图表坐标；IPC 保留 Rust 原始亚毫秒时间，避免选中的终点被排除。
    void historyApi
      .analytics<RangeSummary>(
        {
          kind: 'range',
          windowId: series.windowId,
          startAt:
            series.points.find((p) => Date.parse(p.sampledAt) === start)?.sampledAt ??
            new Date(start).toISOString(),
          endAt:
            series.points.find((p) => Date.parse(p.sampledAt) === end)?.sampledAt ??
            new Date(end).toISOString(),
        },
        accountId,
      )
      .then((result) => {
        if (!disposed) {
          setRangeSummary(result);
          setRangeError(false);
        }
      })
      .catch(() => {
        if (!disposed) setRangeError(true);
      });
    return () => {
      disposed = true;
    };
  }, [start, end, series.windowId, series.points, accountId]);
  const selectPoint = (timestamp: number) => {
    const points = validHistoryPoints(series.points);
    if (!points.length || !Number.isFinite(timestamp)) return;
    const closest = points.reduce((best, point) =>
      Math.abs(point.timestamp - timestamp) < Math.abs(best.timestamp - timestamp) ? point : best,
    );
    setRangeSummary(null);
    setRangeError(false);
    setSelection((previous) =>
      previous.length === 1 ? [previous[0], closest.timestamp] : [closest.timestamp],
    );
  };
  return { selection, setSelection, cursor, setCursor, rangeSummary, rangeError, start, end, selectPoint };
}
