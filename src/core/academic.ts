import type {
  Assignment,
  CalendarException,
  Course,
  Meeting,
  PersonalEvent,
  Quarter,
  TermId,
} from "./model";
export const JST = "Asia/Tokyo";
export function termForDate(date: Date): TermId {
  const p = new Intl.DateTimeFormat("en-CA", {
    timeZone: JST,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).formatToParts(date);
  const year = Number(p.find((x) => x.type === "year")!.value),
    month = Number(p.find((x) => x.type === "month")!.value);
  const academic = month < 4 ? year - 1 : year;
  const q: Quarter =
    month >= 4 && month < 6
      ? "1Q"
      : month >= 6 && month < 10
        ? "2Q"
        : month >= 10 && month < 12
          ? "3Q"
          : "4Q";
  return `${academic}-${q}`;
}
export function deadlineState(
  a: Assignment,
  now = new Date(),
): { label: string; tone: "warning" | "normal"; ms: number } {
  if (!a.dueAt)
    return { label: "No deadline", tone: "normal", ms: Infinity };
  const ms = new Date(a.dueAt).getTime() - now.getTime();
  if (ms < 0)
    return { label: `${relative(Math.abs(ms))} ago`, tone: "normal", ms };
  if (ms < 864e5)
    return { label: `Due in ${relative(ms)}`, tone: "warning", ms };
  return {
    label: `Due ${new Intl.DateTimeFormat("en-GB", { timeZone: JST, day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }).format(new Date(a.dueAt))}`,
    tone: "normal",
    ms,
  };
}
/**
 * The work's state for display, with the grade once the LMS shows one
 * ("採点済 85.00 / 100.00"). Marking it submitted by hand counts as submitted.
 */
export function submissionLabel(
  a: Assignment,
  translate: (key: string) => string,
): string {
  if (a.submission === "graded")
    return a.grade ? `${translate("graded")} ${a.grade}` : translate("graded");
  if (a.submission === "submitted" || a.submissionStatus === "submitted")
    return translate("submitted");
  return translate(a.submission ?? "unknown");
}
/**
 * Time left before a deadline, for a chip: exact to the hour inside three
 * days ("1d 12h"), whole days beyond. Tone tiers: under a day, under three.
 */
export function dueCountdown(
  dueAt: string,
  now = new Date(),
): { label: string; tone: "urgent" | "soon" | "normal" | "past"; ms: number } {
  const ms = new Date(dueAt).getTime() - now.getTime();
  if (ms < 0) return { label: `${relative(-ms)} ago`, tone: "past", ms };
  const hours = Math.floor(ms / 36e5);
  const days = Math.floor(hours / 24);
  const label =
    hours < 1
      ? `${Math.max(1, Math.floor(ms / 6e4))}m`
      : hours < 24
        ? `${hours}h`
        : hours < 72
          ? hours % 24
            ? `${days}d ${hours % 24}h`
            : `${days}d`
          : `${days}d`;
  return {
    label,
    tone: hours < 24 ? "urgent" : hours < 72 ? "soon" : "normal",
    ms,
  };
}
const relative = (ms: number) => {
  const h = Math.max(1, Math.round(ms / 36e5));
  return h < 24 ? `${h}h` : `${Math.round(h / 24)}d`;
};
export function effectiveWeekday(
  date: Date,
  exceptions: CalendarException[],
): number {
  const iso = new Intl.DateTimeFormat("en-CA", { timeZone: JST }).format(date);
  const e = exceptions.find((x) => x.date === iso && !x.deletedAt);
  if (e?.type === "no-class" || e?.type === "break") return 0;
  if (e?.type === "replacement" && e.replacementDay) return e.replacementDay;
  const day = new Intl.DateTimeFormat("en-US", {
    timeZone: JST,
    weekday: "short",
  }).format(date);
  return (
    (
      { Mon: 1, Tue: 2, Wed: 3, Thu: 4, Fri: 5, Sat: 6, Sun: 7 } as Record<
        string,
        number
      >
    )[day] || 0
  );
}
const overlap = (
  a: { startTime: string; endTime: string },
  b: { startTime: string; endTime: string },
) => a.startTime < b.endTime && b.startTime < a.endTime;
export type Conflict = {
  leftId: string;
  rightId: string;
  day: number;
  startTime: string;
  endTime: string;
};
export function detectConflicts(
  courses: Course[],
  events: PersonalEvent[],
): Conflict[] {
  const slots: {
    id: string;
    days: number[];
    startTime: string;
    endTime: string;
  }[] = [];
  courses
    .filter((x) => !x.deletedAt)
    .forEach((c) => c.meetings.forEach((m) => slots.push({ id: c.id, ...m })));
  events
    .filter((x) => x.blocking && !x.deletedAt && x.days)
    .forEach((e) =>
      slots.push({
        id: e.id,
        days: e.days!,
        startTime: e.startTime,
        endTime: e.endTime,
      }),
    );
  const out: Conflict[] = [];
  for (let i = 0; i < slots.length; i++)
    for (let j = i + 1; j < slots.length; j++)
      for (const day of slots[i].days)
        if (slots[j].days.includes(day) && overlap(slots[i], slots[j]))
          out.push({
            leftId: slots[i].id,
            rightId: slots[j].id,
            day,
            startTime:
              slots[i].startTime > slots[j].startTime
                ? slots[i].startTime
                : slots[j].startTime,
            endTime:
              slots[i].endTime < slots[j].endTime
                ? slots[i].endTime
                : slots[j].endTime,
          });
  return out;
}
export function meetingsOnDay(
  courses: Course[],
  events: PersonalEvent[],
  day: number,
  date?: string,
): (Meeting & { id: string; title: string; color: string })[] {
  return [
    ...courses
      .filter((c) => !c.deletedAt)
      .flatMap((c) =>
        c.meetings
          .filter((m) => m.days.includes(day))
          .map((m) => ({
            ...m,
            id: c.id,
            title: c.overrides?.title || c.title,
            color: c.overrides?.color || c.color,
          })),
      ),
    ...events
      .filter(
        (e) =>
          !e.deletedAt &&
          (e.days?.includes(day) || Boolean(date && e.date === date)),
      )
      .map((e) => ({
        days: e.days!,
        periods: [],
        startTime: e.startTime,
        endTime: e.endTime,
        room: e.room,
        id: e.id,
        title: e.title,
        color: "#8a887f",
      })),
  ].sort((a, b) => a.startTime.localeCompare(b.startTime));
}
