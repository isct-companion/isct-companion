import type { ScheduledTask } from "../core/model";

const HOUR_MS = 60 * 60 * 1_000;
const DAY_MS = 24 * HOUR_MS;
const JST_MS = 9 * HOUR_MS;
const WEEKDAYS = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"] as const;

type Frequency = "HOURLY" | "DAILY" | "WEEKLY";

export type ParsedTaskSchedule =
  | { kind: "launch" }
  | {
      kind: "rrule";
      frequency: Frequency;
      interval: number;
      hour?: number;
      minute: number;
      weekdays?: ReadonlySet<string>;
    };

function integer(
  fields: Map<string, string>,
  name: string,
  fallback: number,
  minimum: number,
  maximum: number,
) {
  const raw = fields.get(name);
  if (raw === undefined) return fallback;
  if (!/^\d+$/.test(raw)) throw new Error(`${name} must be an integer.`);
  const value = Number(raw);
  if (value < minimum || value > maximum)
    throw new Error(`${name} must be between ${minimum} and ${maximum}.`);
  return value;
}

/** Parse the deliberately small task RRULE surface: hourly, daily, and weekly. */
export function parseTaskSchedule(raw: string): ParsedTaskSchedule {
  const schedule = raw.trim();
  if (schedule === "launch") return { kind: "launch" };
  const body = schedule.replace(/^RRULE:/i, "");
  if (!body || /[\r\n]/.test(body))
    throw new Error("Enter one RRULE without additional calendar properties.");
  const fields = new Map<string, string>();
  for (const part of body.split(";")) {
    const [key, value, ...extra] = part.split("=");
    const normalized = key?.toUpperCase();
    if (!normalized || !value || extra.length || fields.has(normalized))
      throw new Error("The RRULE contains an invalid or duplicate field.");
    fields.set(normalized, value.toUpperCase());
  }
  const supported = new Set([
    "FREQ",
    "INTERVAL",
    "BYHOUR",
    "BYMINUTE",
    "BYDAY",
  ]);
  for (const name of fields.keys())
    if (!supported.has(name))
      throw new Error(`${name} is not supported for scheduled tasks.`);
  const frequency = fields.get("FREQ") as Frequency | undefined;
  if (!frequency || !["HOURLY", "DAILY", "WEEKLY"].includes(frequency))
    throw new Error("Scheduled tasks support hourly, daily, or weekly RRULEs.");
  const interval = integer(fields, "INTERVAL", 1, 1, 8_760);
  const minute = integer(fields, "BYMINUTE", 0, 0, 59);
  const hour = fields.has("BYHOUR")
    ? integer(fields, "BYHOUR", 0, 0, 23)
    : undefined;
  if (frequency === "HOURLY" && hour !== undefined)
    throw new Error("Hourly tasks cannot include BYHOUR.");
  if (frequency !== "HOURLY" && hour === undefined)
    throw new Error("Daily and weekly tasks require BYHOUR.");
  const dayValues = fields.get("BYDAY")?.split(",");
  if (frequency === "WEEKLY" && !dayValues?.length)
    throw new Error("Weekly tasks require BYDAY.");
  if (frequency !== "WEEKLY" && dayValues)
    throw new Error("BYDAY is only supported for weekly tasks.");
  if (
    dayValues?.some(
      (day) => !WEEKDAYS.includes(day as (typeof WEEKDAYS)[number]),
    )
  )
    throw new Error("BYDAY contains an unsupported weekday.");
  return {
    kind: "rrule",
    frequency,
    interval,
    hour,
    minute,
    weekdays: dayValues ? new Set(dayValues) : undefined,
  };
}

function jstDay(timestamp: number) {
  return Math.floor((timestamp + JST_MS) / DAY_MS);
}

function jstCandidate(day: number, hour: number, minute: number) {
  return day * DAY_MS - JST_MS + hour * HOUR_MS + minute * 60_000;
}

function latestRruleOccurrence(
  schedule: Extract<ParsedTaskSchedule, { kind: "rrule" }>,
  anchor: number,
  now: number,
): number | undefined {
  if (now < anchor) return undefined;
  if (schedule.frequency === "HOURLY") {
    const anchorHour = Math.floor(anchor / HOUR_MS);
    let hour = Math.floor(now / HOUR_MS);
    hour -=
      (((hour - anchorHour) % schedule.interval) + schedule.interval) %
      schedule.interval;
    let candidate = hour * HOUR_MS + schedule.minute * 60_000;
    if (candidate > now) candidate -= schedule.interval * HOUR_MS;
    return candidate >= anchor ? candidate : undefined;
  }

  const anchorDay = jstDay(anchor);
  const nowDay = jstDay(now);
  const searchDays =
    schedule.frequency === "DAILY" ? schedule.interval : schedule.interval * 7;
  for (let offset = 0; offset <= searchDays; offset += 1) {
    const day = nowDay - offset;
    const candidate = jstCandidate(day, schedule.hour!, schedule.minute);
    if (candidate > now || candidate < anchor) continue;
    if (schedule.frequency === "DAILY") {
      if ((day - anchorDay) % schedule.interval === 0) return candidate;
      continue;
    }
    const weekday = WEEKDAYS[new Date(candidate + JST_MS).getUTCDay()];
    if (!schedule.weekdays?.has(weekday)) continue;
    const anchorWeek = Math.floor((anchorDay + 3) / 7);
    const week = Math.floor((day + 3) / 7);
    if ((week - anchorWeek) % schedule.interval === 0) return candidate;
  }
  return undefined;
}

/** Returns a stable occurrence timestamp, or undefined when this task is not due. */
export function dueTaskOccurrence(
  task: ScheduledTask,
  now: Date,
  schedulerStartedAt: Date,
): string | undefined {
  if (!task.enabled || task.deletedAt) return undefined;
  const schedule = parseTaskSchedule(task.schedule);
  if (schedule.kind === "launch") {
    const occurrence = schedulerStartedAt.toISOString();
    const prior = Date.parse(task.lastOccurrenceAt ?? "");
    return Number.isFinite(prior) && prior >= schedulerStartedAt.getTime()
      ? undefined
      : occurrence;
  }
  const anchor = Date.parse(task.updatedAt);
  if (!Number.isFinite(anchor)) throw new Error("Task updatedAt is invalid.");
  const occurrence = latestRruleOccurrence(schedule, anchor, now.getTime());
  if (occurrence === undefined) return undefined;
  // Tasks run only while this app process is alive. Do not catch up occurrences
  // missed while it was closed; allow one poll interval for startup/timer jitter.
  if (occurrence < schedulerStartedAt.getTime() - 60_000) return undefined;
  const prior = Date.parse(task.lastOccurrenceAt ?? task.lastRunAt ?? "");
  if (Number.isFinite(prior) && prior >= occurrence) return undefined;
  return new Date(occurrence).toISOString();
}

export type ScheduledTaskRuntime = {
  getTasks(): readonly ScheduledTask[];
  markStarted(
    taskId: string,
    occurrenceAt: string,
    runAt: string,
  ): Promise<void>;
  execute(task: ScheduledTask): Promise<string>;
  markFinished(
    taskId: string,
    occurrenceAt: string,
    runAt: string,
    result: string,
  ): Promise<void>;
  now?: () => Date;
  setInterval?: typeof globalThis.setInterval;
  clearInterval?: typeof globalThis.clearInterval;
  pollMilliseconds?: number;
  /** Stable for the whole app process, including React development remounts. */
  startedAt?: Date;
};

/** In-process, at-most-once scheduler. No task runs after stop(). */
export function createScheduledTaskRuntime(runtime: ScheduledTaskRuntime) {
  const now = runtime.now ?? (() => new Date());
  const startedAt = runtime.startedAt ?? now();
  const launched = new Set<string>();
  const inFlight = new Set<string>();
  const claimed = new Set<string>();
  const invalid = new Set<string>();
  let stopped = false;
  let ticking: Promise<void> | undefined;

  const tick = async () => {
    if (stopped || ticking) return ticking;
    ticking = (async () => {
      const tickNow = now();
      for (const task of runtime.getTasks()) {
        if (stopped) break;
        let occurrenceAt: string | undefined;
        try {
          occurrenceAt = dueTaskOccurrence(task, tickNow, startedAt);
        } catch (error) {
          const key = `${task.id}:${task.schedule}`;
          if (!invalid.has(key) && task.enabled && !task.deletedAt) {
            invalid.add(key);
            const runAt = now().toISOString();
            const result = `Failed: ${error instanceof Error ? error.message : "invalid schedule"}`;
            try {
              await runtime.markStarted(task.id, runAt, runAt);
              await runtime.markFinished(
                task.id,
                runAt,
                now().toISOString(),
                result,
              );
            } catch {
              // The workspace store owns persistence diagnostics. Never run a
              // task whose invalid-schedule result could not be recorded.
            }
          }
          continue;
        }
        if (!occurrenceAt) continue;
        if (task.schedule === "launch" && launched.has(task.id)) continue;
        const key = `${task.id}:${occurrenceAt}`;
        if (inFlight.has(key) || claimed.has(key)) continue;
        inFlight.add(key);
        const runAt = now().toISOString();
        try {
          // The occurrence marker is durable before the side effect starts. A
          // crash can leave "Running", but cannot silently repeat an export.
          await runtime.markStarted(task.id, occurrenceAt, runAt);
          claimed.add(key);
          if (task.schedule === "launch") launched.add(task.id);
          let result: string;
          try {
            result = await runtime.execute(task);
          } catch (error) {
            result = `Failed: ${error instanceof Error ? error.message : "unknown error"}`;
          }
          await runtime.markFinished(
            task.id,
            occurrenceAt,
            now().toISOString(),
            result,
          );
        } finally {
          inFlight.delete(key);
        }
      }
    })().finally(() => {
      ticking = undefined;
    });
    return ticking;
  };

  const setTimer = runtime.setInterval ?? globalThis.setInterval;
  const clearTimer = runtime.clearInterval ?? globalThis.clearInterval;
  const timer = setTimer(() => void tick(), runtime.pollMilliseconds ?? 30_000);
  void tick();
  return {
    tick,
    stop() {
      stopped = true;
      clearTimer(timer);
    },
  };
}
