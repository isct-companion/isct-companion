import type { IntensiveSchedule, Plan } from "./model";
import { classSlots } from "./timetable-layout";

const iso = (date: Date) => date.toISOString().slice(0, 10);
const localDate = (value: string) => new Date(`${value}T12:00:00+09:00`);

export function intensiveDays(
  from: string,
  to: string,
  periods: [number, number],
): NonNullable<IntensiveSchedule["days"]> {
  const start = localDate(from);
  const end = localDate(to);
  if (!Number.isFinite(start.valueOf()) || !Number.isFinite(end.valueOf()) || start > end)
    throw new Error("Choose a valid intensive-course date range.");
  const result: NonNullable<IntensiveSchedule["days"]> = [];
  for (const cursor = new Date(start); cursor <= end; cursor.setDate(cursor.getDate() + 1))
    result.push({ date: iso(cursor), periods });
  return result;
}

export function clockForPeriods(periods: [number, number]) {
  const covered = classSlots.filter(
    (slot) => slot.periods[0]! >= periods[0] && slot.periods.at(-1)! <= periods[1],
  );
  return {
    start: covered[0]?.start ?? "08:50",
    end: covered.at(-1)?.end ?? "18:55",
  };
}

type Rule = {
  freq: "DAILY" | "WEEKLY" | "MONTHLY" | "YEARLY";
  interval: number;
  count?: number;
  until?: string;
  byday?: number[];
};

const weekday = new Map([
  ["MO", 1], ["TU", 2], ["WE", 3], ["TH", 4], ["FR", 5], ["SA", 6], ["SU", 0],
]);

function parseRule(raw?: string): Rule | undefined {
  if (!raw) return undefined;
  const fields = Object.fromEntries(raw.replace(/^RRULE:/i, "").split(";").map((part) => part.split("=", 2)));
  const freq = fields.FREQ as Rule["freq"];
  if (!(["DAILY", "WEEKLY", "MONTHLY", "YEARLY"] as string[]).includes(freq)) return undefined;
  return {
    freq,
    interval: Math.max(1, Number(fields.INTERVAL) || 1),
    count: fields.COUNT ? Number(fields.COUNT) : undefined,
    until: fields.UNTIL?.slice(0, 8).replace(/^(\d{4})(\d{2})(\d{2})$/, "$1-$2-$3"),
    byday: fields.BYDAY?.split(",").map((day: string) => weekday.get(day.slice(-2))).filter((v: number | undefined): v is number => v !== undefined),
  };
}

/** Expand only the bounded week requested by Schedule; Rust remains authoritative. */
export function planOccursOn(plan: Plan, date: string): boolean {
  const startDate = plan.start.slice(0, 10);
  if (date < startDate || plan.exdates.includes(date)) return false;
  const rule = parseRule(plan.rrule);
  if (!rule) return date === startDate;
  if (rule.until && date > rule.until) return false;
  const start = localDate(startDate);
  const target = localDate(date);
  const days = Math.round((target.valueOf() - start.valueOf()) / 86_400_000);
  let matches = false;
  if (rule.freq === "DAILY") matches = days % rule.interval === 0;
  if (rule.freq === "WEEKLY") {
    const weeks = Math.floor(days / 7);
    matches = weeks % rule.interval === 0 && (rule.byday?.includes(target.getDay()) ?? target.getDay() === start.getDay());
  }
  if (rule.freq === "MONTHLY") {
    const months = (target.getFullYear() - start.getFullYear()) * 12 + target.getMonth() - start.getMonth();
    matches = months >= 0 && months % rule.interval === 0 && target.getDate() === start.getDate();
  }
  if (rule.freq === "YEARLY")
    matches = (target.getFullYear() - start.getFullYear()) % rule.interval === 0 && target.getMonth() === start.getMonth() && target.getDate() === start.getDate();
  if (!matches) return false;
  if (!rule.count) return true;
  let seen = 0;
  for (const cursor = new Date(start); cursor <= target; cursor.setDate(cursor.getDate() + 1)) {
    const candidate = iso(cursor);
    if (candidate === date) break;
    if (!plan.exdates.includes(candidate) && planOccursOn({ ...plan, rrule: plan.rrule?.replace(/;COUNT=\d+/i, "") }, candidate)) seen++;
  }
  return seen < rule.count;
}

export function nextPlanOccurrences(plan: Plan, limit = 5): string[] {
  const start = localDate(plan.start.slice(0, 10));
  const values: string[] = [];
  for (let offset = 0; offset < 3660 && values.length < limit; offset++) {
    const cursor = new Date(start);
    cursor.setDate(cursor.getDate() + offset);
    const date = iso(cursor);
    if (planOccursOn(plan, date)) values.push(`${date}T${plan.start.slice(11, 19) || "00:00:00"}+09:00`);
  }
  return values;
}
