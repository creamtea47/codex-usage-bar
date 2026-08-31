export type CustomHistoryDateIssue =
  | 'missingStart'
  | 'missingEnd'
  | 'invalidStart'
  | 'invalidEnd'
  | 'future'
  | 'reversed'
  | 'emptyToday';

export interface CustomHistoryDateValidation {
  valid: boolean;
  issue: CustomHistoryDateIssue | null;
}

function isValidDate(value: Date): boolean {
  return Number.isFinite(value.getTime());
}

function localCalendarOrdinal(value: Date): number {
  return Date.UTC(value.getFullYear(), value.getMonth(), value.getDate());
}

function startOfLocalDay(value: Date): Date {
  const result = new Date(value);
  result.setHours(0, 0, 0, 0);
  return result;
}

function addLocalCalendarDays(value: Date, amount: number): Date {
  const result = startOfLocalDay(value);
  result.setDate(result.getDate() + amount);
  return result;
}

function isSameLocalCalendarDay(left: Date, right: Date): boolean {
  return (
    left.getFullYear() === right.getFullYear() &&
    left.getMonth() === right.getMonth() &&
    left.getDate() === right.getDate()
  );
}

export function validateCustomHistoryDates(
  start: Date | null,
  end: Date | null,
  now: Date,
): CustomHistoryDateValidation {
  if (!start) return { valid: false, issue: 'missingStart' };
  if (!end) return { valid: false, issue: 'missingEnd' };
  if (!isValidDate(start)) return { valid: false, issue: 'invalidStart' };
  if (!isValidDate(end)) return { valid: false, issue: 'invalidEnd' };

  const today = localCalendarOrdinal(now);
  const startDay = localCalendarOrdinal(start);
  const endDay = localCalendarOrdinal(end);
  if (startDay > today || endDay > today) return { valid: false, issue: 'future' };
  if (startDay > endDay) return { valid: false, issue: 'reversed' };
  if (
    startDay === today
    && endDay === today
    && now.getTime() === startOfLocalDay(now).getTime()
  ) {
    return { valid: false, issue: 'emptyToday' };
  }
  return { valid: true, issue: null };
}

/**
 * Converts inclusive local calendar dates to the backend's UTC half-open interval.
 * A selection ending today is capped at the Apply instant so no future boundary is sent.
 */
export function buildCustomHistoryRequest(start: Date, end: Date, now: Date) {
  const validation = validateCustomHistoryDates(start, end, now);
  if (!validation.valid) {
    throw new RangeError(`Invalid custom history dates: ${validation.issue}`);
  }

  const endAtExclusive = isSameLocalCalendarDay(end, now)
    ? new Date(now)
    : addLocalCalendarDays(end, 1);
  return {
    kind: 'custom' as const,
    startAt: startOfLocalDay(start).toISOString(),
    endAtExclusive: endAtExclusive.toISOString(),
  };
}

export function defaultCustomHistoryDates(now: Date): { start: Date; end: Date } {
  const end = startOfLocalDay(now);
  return { start: addLocalCalendarDays(end, -6), end };
}

export function hasPartialHistoryCoverage(coverage: {
  truncatedByRetention: boolean;
  appliedStartAt: string;
  availableStartAt: string | null;
}): boolean {
  if (coverage.truncatedByRetention) return true;
  if (!coverage.availableStartAt) return false;
  const appliedStart = Date.parse(coverage.appliedStartAt);
  const availableStart = Date.parse(coverage.availableStartAt);
  return Number.isFinite(appliedStart) && Number.isFinite(availableStart) && availableStart > appliedStart;
}
