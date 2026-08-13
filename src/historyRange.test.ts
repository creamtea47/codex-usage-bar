import { afterEach, describe, expect, it } from 'vitest';

import {
  buildCustomHistoryRequest,
  countInclusiveLocalDays,
  hasPartialHistoryCoverage,
  validateCustomHistoryDates,
} from './historyRange';

const runtimeProcess = (
  globalThis as typeof globalThis & { process: { env: Record<string, string | undefined> } }
).process;
const originalTimezone = runtimeProcess.env.TZ;

afterEach(() => {
  if (originalTimezone === undefined) delete runtimeProcess.env.TZ;
  else runtimeProcess.env.TZ = originalTimezone;
});

describe('custom history local-date boundaries', () => {
  it('accepts at most 30 inclusive local calendar days regardless of elapsed hours', () => {
    runtimeProcess.env.TZ = 'America/Los_Angeles';
    const now = new Date(2030, 3, 30, 12, 0);

    expect(countInclusiveLocalDays(new Date(2030, 3, 1), new Date(2030, 3, 30))).toBe(30);
    expect(validateCustomHistoryDates(new Date(2030, 3, 1), new Date(2030, 3, 30), now)).toEqual({
      valid: true,
      issue: null,
    });
    expect(validateCustomHistoryDates(new Date(2030, 2, 31), new Date(2030, 3, 30), now)).toEqual({
      valid: false,
      issue: 'tooLong',
    });
  });

  it('rejects missing, reversed, and future local dates', () => {
    const now = new Date(2030, 0, 15, 8, 0);

    expect(validateCustomHistoryDates(null, new Date(2030, 0, 15), now).issue).toBe('missingStart');
    expect(validateCustomHistoryDates(new Date(2030, 0, 15), null, now).issue).toBe('missingEnd');
    expect(validateCustomHistoryDates(new Date(2030, 0, 15), new Date(2030, 0, 14), now).issue).toBe(
      'reversed',
    );
    expect(validateCustomHistoryDates(new Date(2030, 0, 15), new Date(2030, 0, 16), now).issue).toBe(
      'future',
    );
  });

  it('turns past local dates across DST into UTC half-open boundaries', () => {
    runtimeProcess.env.TZ = 'America/Los_Angeles';

    const request = buildCustomHistoryRequest(
      new Date(2030, 2, 9),
      new Date(2030, 2, 10),
      new Date(2030, 2, 11, 12, 0),
    );

    expect(request).toEqual({
      kind: 'custom',
      startAt: '2030-03-09T08:00:00.000Z',
      endAtExclusive: '2030-03-11T07:00:00.000Z',
    });
    expect(Date.parse(request.endAtExclusive) - Date.parse(request.startAt)).toBe(47 * 60 * 60 * 1_000);
  });

  it('keeps an inclusive fall-back DST range to local midnights', () => {
    runtimeProcess.env.TZ = 'America/Los_Angeles';

    const request = buildCustomHistoryRequest(
      new Date(2030, 10, 2),
      new Date(2030, 10, 3),
      new Date(2030, 10, 4, 12, 0),
    );

    expect(request).toEqual({
      kind: 'custom',
      startAt: '2030-11-02T07:00:00.000Z',
      endAtExclusive: '2030-11-04T08:00:00.000Z',
    });
    expect(Date.parse(request.endAtExclusive) - Date.parse(request.startAt)).toBe(49 * 60 * 60 * 1_000);
  });

  it('caps a range ending today at the injected Apply instant instead of tomorrow', () => {
    runtimeProcess.env.TZ = 'America/Los_Angeles';
    const applyInstant = new Date(2030, 2, 10, 15, 30);

    const request = buildCustomHistoryRequest(
      new Date(2030, 2, 9),
      new Date(2030, 2, 10),
      applyInstant,
    );

    expect(request.startAt).toBe('2030-03-09T08:00:00.000Z');
    expect(request.endAtExclusive).toBe(applyInstant.toISOString());
  });

  it('uses the exact local-midnight Apply instant when today is selected', () => {
    runtimeProcess.env.TZ = 'America/Los_Angeles';
    const applyInstant = new Date(2030, 4, 20, 0, 0, 0, 0);

    const request = buildCustomHistoryRequest(
      new Date(2030, 4, 19),
      new Date(2030, 4, 20),
      applyInstant,
    );

    expect(request.endAtExclusive).toBe(applyInstant.toISOString());
    expect(Date.parse(request.endAtExclusive)).toBeGreaterThan(Date.parse(request.startAt));
  });

  it('rejects a today-only range at exact local midnight because no interval has elapsed', () => {
    runtimeProcess.env.TZ = 'America/Los_Angeles';
    const localMidnight = new Date(2030, 4, 20, 0, 0, 0, 0);

    expect(validateCustomHistoryDates(localMidnight, localMidnight, localMidnight)).toEqual({
      valid: false,
      issue: 'emptyToday',
    });
  });

  it('recognizes explicit retention truncation and an applied-start coverage gap', () => {
    expect(
      hasPartialHistoryCoverage({
        truncatedByRetention: true,
        appliedStartAt: '2030-01-01T00:00:00.000Z',
        availableStartAt: null,
      }),
    ).toBe(true);
    expect(
      hasPartialHistoryCoverage({
        truncatedByRetention: false,
        appliedStartAt: '2030-01-01T00:00:00.000Z',
        availableStartAt: '2030-01-02T00:00:00.000Z',
      }),
    ).toBe(true);
    expect(
      hasPartialHistoryCoverage({
        truncatedByRetention: false,
        appliedStartAt: '2030-01-01T00:00:00.000Z',
        availableStartAt: '2030-01-01T00:00:00.000Z',
      }),
    ).toBe(false);
  });
});
