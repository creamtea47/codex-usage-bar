import { describe, expect, it } from 'vitest';

import { defaultSettings } from './types';
import { applySettingsViewSnapshot } from './trendsOperationState';

describe('applySettingsViewSnapshot', () => {
  it('advances only for a matching authoritative trend-toggle result', () => {
    const initial = { settings: defaultSettings, trendsOperationRevision: 0 };
    const paused = applySettingsViewSnapshot(
      initial,
      { ...defaultSettings, historyEnabled: false },
      false,
    );
    expect(paused).toEqual({
      settings: { ...defaultSettings, historyEnabled: false },
      trendsOperationRevision: 1,
    });

    const failedOrUnrelated = applySettingsViewSnapshot(paused, {
      ...defaultSettings,
      historyEnabled: false,
    });
    expect(failedOrUnrelated.trendsOperationRevision).toBe(1);

    const mismatched = applySettingsViewSnapshot(
      failedOrUnrelated,
      { ...defaultSettings, historyEnabled: false },
      true,
    );
    expect(mismatched.settings.historyEnabled).toBe(false);
    expect(mismatched.trendsOperationRevision).toBe(1);

    const enabled = applySettingsViewSnapshot(
      mismatched,
      { ...defaultSettings, historyEnabled: true },
      true,
    );
    expect(enabled.trendsOperationRevision).toBe(2);
  });

  it('does not advance for a mismatched authoritative readback', () => {
    const initial = {
      settings: { ...defaultSettings, historyEnabled: false },
      trendsOperationRevision: 3,
    };

    const eventOnly = applySettingsViewSnapshot(initial, {
      ...defaultSettings,
      historyEnabled: true,
    });

    expect(eventOnly.settings.historyEnabled).toBe(true);
    expect(eventOnly.trendsOperationRevision).toBe(3);
  });
});
