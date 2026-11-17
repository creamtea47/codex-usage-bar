import { useCallback, useEffect, useRef, useState } from 'react';

import type { UsageHistoryRequest, UsageHistoryResponse } from '@/contracts/history';

type Unlisten = () => void;
export interface HistoryQuerySource {
  getUsageHistory: (request: UsageHistoryRequest) => Promise<UsageHistoryResponse>;
  listenForUsageHistory?: (handler: () => void) => Promise<Unlisten>;
}
export function useHistoryQuery(
  request: UsageHistoryRequest,
  { getUsageHistory, listenForUsageHistory }: HistoryQuerySource,
  onListenerError: () => void,
) {
  const [history, setHistory] = useState<UsageHistoryResponse | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [loadFailed, setLoadFailed] = useState(false);
  const [reloadVersion, setReloadVersion] = useState(0);
  const [historyListenerState, setHistoryListenerState] = useState<{
    source: HistoryQuerySource['listenForUsageHistory'];
    ready: boolean;
  }>(() => ({ source: listenForUsageHistory, ready: !listenForUsageHistory }));
  const isHistoryListenerReady =
    !listenForUsageHistory ||
    (historyListenerState.source === listenForUsageHistory && historyListenerState.ready);
  const requestGenerationRef = useRef(0);
  const requestReload = useCallback(() => {
    // 先同步作废所有在途读取，再触发下一次读取，避免旧响应覆盖更新事件后的快照。
    requestGenerationRef.current += 1;
    setIsLoading(true);
    setLoadFailed(false);
    setReloadVersion((version) => version + 1);
  }, []);

  useEffect(() => {
    if (!listenForUsageHistory) {
      return undefined;
    }
    let disposed = false;
    let unlisten: Unlisten | undefined;
    void listenForUsageHistory(() => {
      if (!disposed) {
        requestReload();
      }
    })
      .then((removeListener) => {
        if (disposed) removeListener();
        else {
          unlisten = removeListener;
          setHistoryListenerState({ source: listenForUsageHistory, ready: true });
        }
      })
      .catch(() => {
        if (!disposed) {
          onListenerError();
          // Event delivery is optional. A failed subscription must not block
          // the settings window from reading the current history snapshot.
          setHistoryListenerState({ source: listenForUsageHistory, ready: true });
        }
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [listenForUsageHistory, requestReload, onListenerError]);

  useEffect(() => {
    if (!isHistoryListenerReady) return undefined;
    let disposed = false;
    const requestGeneration = ++requestGenerationRef.current;
    void getUsageHistory(request)
      .then((response) => {
        if (!disposed && requestGeneration === requestGenerationRef.current) {
          setHistory(response);
        }
      })
      .catch(() => {
        if (!disposed && requestGeneration === requestGenerationRef.current) {
          setLoadFailed(true);
        }
      })
      .finally(() => {
        if (!disposed && requestGeneration === requestGenerationRef.current) {
          setIsLoading(false);
        }
      });
    return () => {
      disposed = true;
    };
  }, [getUsageHistory, isHistoryListenerReady, request, reloadVersion]);
  function clearSnapshot() {
    // 即使历史事件监听失败，也要同步作废清除前的在途读取，避免旧快照让已删除数据重新出现。
    requestGenerationRef.current += 1;
    setIsLoading(false);
    setLoadFailed(false);
    setHistory((current) =>
      current
        ? {
            ...current,
            storageStatus: 'empty',
            sampleCount: 0,
            earliestSampleAt: null,
            latestSampleAt: null,
            availableStartAt: null,
            availableEndAt: null,
            truncatedByRetention: false,
            bucketSeconds: null,
            series: [],
          }
        : current,
    );
  }
  function changeRange() {
    setHistory(null);
    requestReload();
  }
  return { history, isLoading, loadFailed, requestReload, changeRange, clearSnapshot };
}
