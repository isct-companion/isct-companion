import { contentLanguage, pickLocalized, type Course, type PersonalEvent, type TermId, type Workspace } from "./model";
import { clockForPeriods, planOccursOn, planCivilStart } from "./user-schedule";

export type Occurrence = {
  key: string;
  id: string;
  kind: "course" | "event" | "plan" | "exam";
  courseId?: string;
  date: string;
  title: string;
  subtitle?: string;
  room: string;
  startTime: string;
  endTime: string;
  endDate?: string;
  color: string;
  conflict?: boolean;
  dim?: boolean;
  cancelled?: boolean;
  timeLabel?: string;
  conflictWith?: string[];
  overridden?: boolean;
};
export const tokyoDate = (date: Date) =>
  new Intl.DateTimeFormat("en-CA", { timeZone: "Asia/Tokyo" }).format(date);
export const dateAtNoon = (date: string) => new Date(`${date}T12:00:00Z`);
export function addDays(date: string, days: number) {
  const value = dateAtNoon(date);
  value.setUTCDate(value.getUTCDate() + days);
  return value.toISOString().slice(0, 10);
}
export function civilWeekday(date: string) {
  return dateAtNoon(date).getUTCDay() || 7;
}
export function calendarState(data: Workspace, date: string) {
  return (
    data.exceptions.find((row) => !row.deletedAt && row.date === date) ??
    data.academicCalendar.find(
      (row) =>
        !row.deletedAt &&
        !row.missingSince &&
        row.date === date &&
        ["holiday", "no-class", "break", "replacement"].includes(row.type),
    )
  );
}
export function quarterRange(data: Workspace, term: TermId) {
  const [rawYear, quarter] = term.split("-");
  const year = Number(rawYear);
  const defaults: Record<string, [string, string]> = {
    "1Q": [`${year}-04-01`, `${year}-05-31`],
    "2Q": [`${year}-06-01`, `${year}-09-30`],
    "3Q": [`${year}-10-01`, `${year}-11-30`],
    "4Q": [`${year}-12-01`, `${year + 1}-03-31`],
  };
  if (!Number.isInteger(year) || !defaults[quarter])
    throw new Error("Invalid quarter.");
  const profile = data.academicProfile.years.find(
    (row) => row.academicYear === year,
  );
  const boundary = (type: "quarter-start" | "quarter-end") =>
    data.academicCalendar.find(
      (row) =>
        !row.deletedAt &&
        !row.missingSince &&
        row.type === type &&
        (row.academicYear === year || row.id.startsWith(`calendar:${year}:`)) &&
        (row.quarter === quarter ||
          row.id === `calendar:${year}:${type}:${quarter}`),
    )?.date;
  const next = { "1Q": "2Q", "2Q": "3Q", "3Q": "4Q" }[quarter] as
    "2Q" | "3Q" | "4Q" | undefined;
  const teachingRanges = data.academicCalendar.find(
    (row) =>
      !row.deletedAt &&
      !row.missingSince &&
      row.id === `calendar:${year}:quarter-start:${quarter}`,
  )?.teachingRanges;
  return {
    teachingRanges,
    from:
      profile?.quarterStarts[quarter as "1Q" | "2Q" | "3Q" | "4Q"] ??
      boundary("quarter-start") ??
      defaults[quarter][0],
    to:
      boundary("quarter-end") ??
      (next && profile
        ? addDays(profile.quarterStarts[next], -1)
        : defaults[quarter][1]),
  };
}
export function scheduledCourses(data: Workspace, term: TermId) {
  const ids = new Set(
    [...data.memberships, ...data.planItems]
      .filter((row) => !row.deletedAt && row.termId === term)
      .map((row) => row.courseId),
  );
  return data.courses.filter(
    (row) => !row.deletedAt && row.termId === term && ids.has(row.id),
  );
}

/** Single actual-date resolver for Today, Next, Schedule, and calendar export.
 * School calendar rules affect weekly courses only; user events/plans keep civil dates.
 * Intensive days are the editor's explicit selected-date list, never implicit overrides.
 */
export function occurrencesOnDate(
  data: Workspace,
  term: TermId,
  date: string,
  options: {
    courses?: Course[];
    events?: PersonalEvent[];
    includePersonal?: boolean;
    /** Timetable context only; cancelled courses never enter Today, reminders or ICS. */
    includeCancelledCourses?: boolean;
    conflicts?: ReadonlySet<string>;
    match?: (course: Course) => boolean;
  } = {},
): Occurrence[] {
  const courses = options.courses ?? scheduledCourses(data, term);
  const state = calendarState(data, date);
  const day = civilWeekday(date);
  const closed = ["holiday", "no-class", "break"].includes(state?.type ?? "");
  const teachingDay =
    state?.type === "replacement"
      ? (state.replacementDay ?? day)
      : closed && !options.includeCancelledCourses
        ? 0
        : day;
  const range = quarterRange(data, term);
  const inQuarter = date >= range.from && date <= range.to;
  const outsideTeaching = Boolean(range.teachingRanges && state?.type !== "replacement" && !range.teachingRanges.some(r => date >= r.start && date <= r.end));
  const blocks: Occurrence[] = [];
  if (inQuarter)
    for (const course of courses.filter((row) => !row.deletedAt)) {
      for (const [index, meeting] of course.meetings.entries()) {
        if (
          (outsideTeaching && !options.includeCancelledCourses) ||
          !meeting.days.includes(teachingDay) ||
          (meeting.validFrom && date < meeting.validFrom) ||
          (meeting.validTo && date > meeting.validTo)
        )
          continue;
        blocks.push({
          key: `${course.id}-weekly-${index}-${date}`,
          cancelled: closed || outsideTeaching || undefined,
          id: course.id,
          courseId: course.id,
          kind: "course",
          date,
          title: course.overrides?.title || course.title,
          subtitle: course.code,
          room: meeting.room,
          startTime: meeting.startTime,
          endTime: meeting.endTime,
          color: course.overrides?.color || course.color,
          conflict: options.conflicts?.has(course.id),
          dim: options.match ? !options.match(course) : false,
        });
      }
      for (const intensive of data.intensiveSchedules.filter(
        (row) =>
          !row.deletedAt &&
          row.courseId === course.id &&
          date >= row.from &&
          date <= row.to,
      )) {
        const selected = intensive.days?.find((row) => row.date === date);
        if (!selected?.periods) continue;
        const clock = clockForPeriods(selected.periods);
        blocks.push({
          key: `${intensive.id}-${date}`,
          id: course.id,
          courseId: course.id,
          kind: "course",
          date,
          title: course.overrides?.title || course.title,
          subtitle: course.code,
          room: intensive.room ?? "",
          startTime: clock.start,
          endTime: clock.end,
          color: course.overrides?.color || course.color,
          conflict: options.conflicts?.has(course.id),
          dim: options.match ? !options.match(course) : false,
        });
      }
    }
  if (options.includePersonal !== false) {
    const events =
      options.events ?? data.events.filter((row) => row.termId === term);
    for (const event of events.filter((row) => !row.deletedAt)) {
      if (event.date ? event.date !== date : !event.days?.includes(day))
        continue;
      blocks.push({
        key: `${event.id}-${date}`,
        id: event.id,
        kind: "event",
        date,
        title: event.title,
        room: event.room,
        startTime: event.startTime,
        endTime: event.endTime,
        color: "#8a887f",
        conflict: options.conflicts?.has(event.id),
      });
    }
    for (const plan of data.plans.filter(
      (row) => !row.deletedAt && planOccursOn(row, date),
    )) {
      const startTime = planCivilStart(plan).time;
      const end = new Date(`${date}T${startTime}:00Z`);
      end.setUTCMinutes(end.getUTCMinutes() + plan.duration);
      blocks.push({
        key: `${plan.id}-${date}`,
        id: plan.id,
        kind: "plan",
        courseId: plan.courseId,
        date,
        title: plan.title,
        room: plan.room ?? "",
        startTime,
        endTime: end.toISOString().slice(11, 16),
        endDate: end.toISOString().slice(0, 10),
        color: "#5f6b8a",
      });
    }
  }
  for (let i = 0; i < blocks.length; i++)
    for (let j = i + 1; j < blocks.length; j++) {
      const a = blocks[i],
        b = blocks[j];
      if (a.cancelled || b.cancelled) continue;
      const endA = `${a.endDate ?? (a.endTime <= a.startTime ? addDays(date, 1) : date)}T${a.endTime}`;
      const endB = `${b.endDate ?? (b.endTime <= b.startTime ? addDays(date, 1) : date)}T${b.endTime}`;
      if (`${date}T${a.startTime}` < endB && `${date}T${b.startTime}` < endA)
        a.conflict = b.conflict = true;
    }
  return blocks.sort(
    (a, b) =>
      a.startTime.localeCompare(b.startTime) || a.key.localeCompare(b.key),
  );
}

const DATE_AT_END = /\d{4}-\d{2}-\d{2}$/;

function overlap(left: Occurrence, right: Occurrence) {
  const leftEnd = `${left.endDate ?? (left.endTime <= left.startTime ? addDays(left.date, 1) : left.date)}T${left.endTime}`;
  const rightEnd = `${right.endDate ?? (right.endTime <= right.startTime ? addDays(right.date, 1) : right.date)}T${right.endTime}`;
  return `${left.date}T${left.startTime}` < rightEnd && `${right.date}T${right.startTime}` < leftEnd;
}

function applyOverride(block: Occurrence, override: Workspace["scheduleOverrides"][number]) {
  const clock = override.periods ? clockForPeriods(override.periods) : undefined;
  return {
    ...block,
    date: override.date ?? block.date,
    title: override.title ?? block.title,
    room: override.room ?? block.room,
    startTime: override.startTime ?? clock?.start ?? block.startTime,
    endTime: override.endTime ?? clock?.end ?? block.endTime,
    courseId: override.courseId ?? block.courseId,
    overridden: true,
  };
}

/** Important all-day dates active on one civil date. */
export function importantDatesOnDate(data: Workspace, date: string) {
  return data.importantDates.filter(
    (row) => !row.deletedAt && !row.missingSince && row.startDate <= date && row.endDate >= date,
  );
}

/**
 * Resolves the dated agenda after one-off overrides and published exams.
 * An exam for the same course replaces that course occurrence. Other overlaps
 * remain visible and are explicitly named as conflicts.
 */
export function resolveDayAgenda(
  data: Workspace,
  term: TermId,
  date: string,
  options: Parameters<typeof occurrencesOnDate>[3] = {},
): Occurrence[] {
  let blocks = occurrencesOnDate(data, term, date, options);
  const overrides = data.scheduleOverrides.filter((row) => !row.deletedAt);

  blocks = blocks.flatMap((block) => {
    const override = overrides.find((row) => row.targetKey === block.key);
    if (!override || override.hidden) return override?.hidden ? [] : [block];
    const changed = applyOverride(block, override);
    return changed.date === date ? [changed] : [];
  });

  // A dated override can move an occurrence into this day. The original date
  // is encoded in the canonical occurrence key, so no second override model is needed.
  for (const override of overrides.filter(
    (row) => !row.hidden && row.targetKind === "course" && row.date === date,
  )) {
    if (blocks.some((block) => block.key === override.targetKey)) continue;
    const sourceDate = override.targetKey.match(DATE_AT_END)?.[0];
    if (!sourceDate || sourceDate === date) continue;
    const original = occurrencesOnDate(data, term, sourceDate, options).find(
      (block) => block.key === override.targetKey,
    );
    if (original) blocks.push(applyOverride(original, override));
  }

  const registeredIds = new Set(
    data.memberships
      .filter(
        (row) =>
          !row.deletedAt &&
          row.termId === term &&
          row.status !== "withdrawn" &&
          row.evidence.provider !== "manual",
      )
      .map((row) => row.courseId),
  );
  const registeredByCode = new Map(
    data.courses
      .filter((course) => !course.deletedAt && registeredIds.has(course.id))
      .map((course) => [course.code.trim().toUpperCase(), course]),
  );

  const exams: Occurrence[] = [];
  for (const exam of data.exams.filter((row) => {
    const key = row.sourceKey ?? row.id;
    const movedDate = overrides.find((override) => override.targetKey === key)?.date;
    return (
      !row.deletedAt &&
      !row.missingSince &&
      row.status === "scheduled" &&
      `${row.academicYear}-${row.quarter}` === term &&
      (movedDate ?? row.date) === date
    );
  })) {
    const targetKey = exam.sourceKey ?? exam.id;
    const override = overrides.find((row) => row.targetKey === targetKey);
    if (override?.hidden) continue;
    const mappedCourseId = override?.courseId ?? exam.courseId;
    const course = mappedCourseId
      ? data.courses.find((row) => row.id === mappedCourseId && !row.deletedAt)
      : registeredByCode.get(exam.courseCode.trim().toUpperCase());
    if (!course) continue;
    const periods = override?.periods ?? exam.periods;
    const clock = periods ? clockForPeriods(periods) : undefined;
    const startTime = override?.startTime ?? clock?.start ?? exam.startTime;
    const endTime = override?.endTime ?? clock?.end ?? exam.endTime;
    const effectiveDate = override?.date ?? exam.date;
    if (!startTime || !endTime || effectiveDate !== date) continue;
    const room =
      override?.room ??
      pickLocalized(exam.room ?? undefined, contentLanguage(data.settings), "");
    exams.push({
      key: targetKey,
      id: course.id,
      courseId: course.id,
      kind: "exam",
      date,
      title:
        override?.title ??
        pickLocalized(exam.label, contentLanguage(data.settings), course.overrides?.title || course.title),
      subtitle: course.code,
      room,
      startTime,
      endTime,
      color: course.overrides?.color || course.color,
      timeLabel: periods ? `${periods[0]}–${periods[1]}限` : `${startTime}–${endTime}`,
      overridden: Boolean(override),
    });
  }

  const examCourseIds = new Set(exams.map((exam) => exam.courseId));
  blocks = blocks.filter(
    (block) => !(block.kind === "course" && examCourseIds.has(block.courseId)),
  );
  blocks.push(...exams);

  for (const exam of exams) {
    const collisions = blocks.filter(
      (block) =>
        block.key !== exam.key &&
        !block.cancelled &&
        block.courseId !== exam.courseId &&
        overlap(exam, block),
    );
    if (!collisions.length) continue;
    exam.conflict = true;
    exam.conflictWith = collisions.map((row) => row.title);
    for (const collision of collisions) collision.conflict = true;
  }
  return blocks.sort(
    (left, right) => {
      const time = left.startTime.localeCompare(right.startTime);
      if (time) return time;
      const layer = Number(left.kind === "exam") - Number(right.kind === "exam");
      return layer || left.key.localeCompare(right.key);
    },
  );
}
