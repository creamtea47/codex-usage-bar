import type { Settings } from './types';

export interface SettingsViewState {
  settings: Settings;
  trendsOperationRevision: number;
}

/** Applies a settings snapshot and grants one recovery budget only for a matching toggle result. */
export function applySettingsViewSnapshot(
  current: SettingsViewState,
  settings: Settings,
  confirmedHistoryEnabled?: boolean,
): SettingsViewState {
  return {
    settings,
    trendsOperationRevision:
      current.trendsOperationRevision +
      (confirmedHistoryEnabled !== undefined && settings.historyEnabled === confirmedHistoryEnabled ? 1 : 0),
  };
}
