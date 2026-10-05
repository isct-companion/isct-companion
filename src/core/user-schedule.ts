import { RRule } from "rrule";
import type { IntensiveSchedule, Plan } from "./model";
import { classSlots } from "./timetable-layout";

const iso = (date: Date) => date.toISOString().slice(0, 10);
const localDate = (value: string) => new Date(`${value}T12:00:00+09:00`);

export function intensiveDays(
  from: string,
  to: string,
  periods: [number, number],
): NonNullable<IntensiveSchedule["days"]> {
  clockForPeriods(periods);
  const start = localDate(from);
  const end = localDate(to);
  if (
    !Number.isFinite(start.valueOf()) ||
    !Number.isFinite(end.valueOf()) ||
    start > end
  )
    throw new Error("Choose a valid intensive-course date range.");
  const result: NonNullable<IntensiveSchedule["days"]> = [];
  for (
    const cursor = new Date(start);
    cursor <= end;
    cursor.setUTCDate(cursor.getUTCDate() + 1)
  )
    result.push({ date: iso(cursor), periods });
  return result;
}

export function clockForPeriods(periods: [number, number]) {
  const first = classSlots.find((slot) =>
    slot.periods.includes(periods[0] as never),
  );
  const last = classSlots.find((slot) =>
    slot.periods.includes(periods[1] as never),
  );
  if (
    !first ||
    !last ||
    !Number.isInteger(periods[0]) ||
    !Number.isInteger(periods[1]) ||
    periods[0] > periods[1]
  )
    throw new Error("Choose periods from 1 to 10 in ascending order.");
  return { start: first.start, end: last.end };
}

export function planCivilStart(plan: Plan) {
  if (!plan.start.includes("T")) return { date: plan.start, time: "00:00" };
  if (!/(?:Z|[+-]\d{2}:\d{2})$/.test(plan.start))
    return { date: plan.start.slice(0, 10), time: plan.start.slice(11, 16) };
  const value = new Date(plan.start);
  return {
    date: new Intl.DateTimeFormat("en-CA", { timeZone: "Asia/Tokyo" }).format(
      value,
    ),
    time: new Intl.DateTimeFormat("en-GB", {
      timeZone: "Asia/Tokyo",
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    }).format(value),
  };
}

const rules = new Map<string, RRule>();
function recurrence(plan: Plan): RRule {
  const key = `${plan.start}:${plan.rrule}`;
  const cached = rules.get(key);
  if (cached) return cached;
  const start = planCivilStart(plan);
  const raw = plan
    .rrule!.trim()
    .replace(/^RRULE:/i, "")
    .toUpperCase();
  if (/\r|\n/.test(raw) || !/FREQ=(DAILY|WEEKLY|MONTHLY|YEARLY)(;|$)/.test(raw))
    throw new Error("Invalid plan recurrence.");
  if (/(?:^|;)(BYHOUR|BYMINUTE|BYSECOND)=/.test(raw))
    throw new Error("Plan recurrence has a minimum unit of one day.");
  const options = RRule.parseString(raw);
  if (options.until) {
    const untilField = raw.match(/(?:^|;)UNTIL=([^;]+)/)?.[1];
    if (untilField?.endsWith("Z"))
      options.until = new Date(options.until.getTime() + 9 * 3600000);
    else if (untilField?.length === 8)
      options.until.setUTCHours(23, 59, 59, 999);
  }
  const rule = new RRule({
    ...options,
    dtstart: new Date(`${start.date}T${start.time}:00Z`),
  });
  if (rules.size > 100) rules.clear();
  rules.set(key, rule);
  return rule;
}

function excludedOn(plan: Plan, date: string) {
  return plan.exdates.some(
    (value) => planCivilStart({ ...plan, start: value }).date === date,
  );
}

/** Floating UTC values represent Japan civil time, independent of the host timezone. */
export function planOccursOn(plan: Plan, date: string): boolean {
  if (excludedOn(plan, date) || date < planCivilStart(plan).date) return false;
  if (!plan.rrule) return date === planCivilStart(plan).date;
  return (
    recurrence(plan).between(
      new Date(`${date}T00:00:00Z`),
      new Date(`${date}T23:59:59Z`),
      true,
    ).length > 0
  );
}

export function nextPlanOccurrences(plan: Plan, limit = 5): string[] {
  if (limit <= 0) return [];
  const start = planCivilStart(plan);
  if (!plan.rrule)
    return excludedOn(plan, start.date)
      ? []
      : [`${start.date}T${start.time}:00+09:00`];
  const values: string[] = [];
  const horizon = new Date(`${start.date}T00:00:00Z`);
  horizon.setUTCFullYear(horizon.getUTCFullYear() + 10);
  recurrence(plan).all((date) => {
    if (date > horizon || values.length >= limit) return false;
    if (!excludedOn(plan, date.toISOString().slice(0, 10)))
      values.push(`${date.toISOString().slice(0, 19)}+09:00`);
    return values.length < limit;
  });
  return values;
}
