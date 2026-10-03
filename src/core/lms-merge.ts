import { timetableCourses } from "../components/timetable/workspace";
import type { FinderCourse } from "./course-query";
import {
  pickLocalized,
  type Course,
  type Quarter,
  type TermId,
  type Workspace,
} from "./model";
import type { LmsCourseRecord } from "../platform/desktop-bridge";
import { catalogCourse } from "../platform/desktop-bridge";
import { stamp } from "../app/shared";
import { clockForPeriods } from "./user-schedule";
import { decodeEntities } from "./text";
import type { Meeting } from "./model";

/** `LAE.E213-53`, `LAH.T209[2026]` → `LAE.E213`, `LAH.T209`. */
export const baseCode = (code: string) =>
  code
    .replace(/\[[^\]]*\]\s*$/, "")
    .replace(/-\d+$/, "")
    .trim()
    .toUpperCase();

export type LmsEntry = {
  key: string;
  termId: TermId;
  lms: LmsCourseRecord;
  title: string;
  code: string;
  offering?: FinderCourse;
  /** Weekly slots `day:period`: the LMS timetable grid, else the syllabus. */
  slots: string[];
  /** Meetings the LMS timetable grid shows for this term, when it was read. */
  gridMeetings?: { day: number; periods: [number, number] }[];
};
export type LocalEntry = {
  key: string;
  termId: TermId;
  course: Course;
  slots: string[];
};
export type MergeDiff = {
  terms: TermId[];
  /** `timeDiffers`: the LMS grid shows other weekly slots than the local course. */
  common: { termId: TermId; local: Course; lms: LmsEntry; timeDiffers: boolean }[];
  localOnly: LocalEntry[];
  lmsOnly: LmsEntry[];
  /** Conflict group per entry key; entries of one group share a colour. */
  conflicts: Map<string, { group: number; reason: "slot" | "quarter" }>;
};

const QUARTERS: Quarter[] = ["1Q", "2Q", "3Q", "4Q"];

/**
 * Quarters an LMS course runs in: those its timetable grid was seen in, plus
 * the syllabus offering's (else the LMS category's) quarters.
 */
export function lmsTerms(row: LmsCourseRecord, offering?: FinderCourse): TermId[] {
  const grid = Object.keys(row.timetables ?? {}).filter((term) => /^\d{4}-[1-4]Q$/.test(term));
  const year = row.year ?? offering?.year;
  if (!year) return grid as TermId[];
  const quarters = (offering?.quarters.length ? offering.quarters : row.quarters).filter(
    (quarter): quarter is Quarter => QUARTERS.includes(quarter as Quarter),
  );
  return [...new Set([...grid, ...quarters.map((quarter) => `${year}-${quarter}`)])] as TermId[];
}

const gridSlots = (meetings: { day: number; periods: [number, number] }[]) =>
  meetings.flatMap((meeting) =>
    Array.from(
      { length: meeting.periods[1] - meeting.periods[0] + 1 },
      (_, index) => `${meeting.day}:${meeting.periods[0] + index}`,
    ),
  );
const sameSlots = (left: string[], right: string[]) =>
  left.length === right.length && [...left].sort().join() === [...right].sort().join();

/** Meetings from the LMS grid, with the syllabus room for the same day when known. */
export function meetingsFromGrid(
  grid: { day: number; periods: [number, number] }[],
  syllabus: Meeting[] = [],
): Meeting[] {
  return grid.map((meeting) => {
    const clock = clockForPeriods(meeting.periods);
    const twin = syllabus.find((item) => item.days.includes(meeting.day));
    return {
      days: [meeting.day],
      periods: Array.from(
        { length: meeting.periods[1] - meeting.periods[0] + 1 },
        (_, index) => meeting.periods[0] + index,
      ),
      startTime: clock.start,
      endTime: clock.end,
      room: twin?.room ?? "",
      roomNames: twin?.roomNames,
    };
  });
}

const courseSlots = (course: Course) =>
  course.meetings.flatMap((meeting) =>
    meeting.days.flatMap((day) => meeting.periods.map((period) => `${day}:${period}`)),
  );
const offeringSlots = (offering?: FinderCourse) =>
  (offering?.schedule ?? []).flatMap((entry) =>
    entry.kind === "weekly" && entry.day && entry.periods
      ? Array.from(
          { length: entry.periods[1] - entry.periods[0] + 1 },
          (_, index) => `${entry.day}:${entry.periods![0] + index}`,
        )
      : [],
  );

/**
 * Compares the timetable of each term with the LMS course list. Courses match
 * by syllabus offering, else by code without class suffix. Among the
 * unmatched, a local and an LMS course conflict when they share a weekly slot
 * (same term) or the same code in different quarters.
 */
export function diffLms(
  data: Workspace,
  lmsCourses: LmsCourseRecord[],
  terms: TermId[],
  offerings: Map<string, FinderCourse>,
): MergeDiff {
  const locale = data.settings.locale;
  const scope = new Set(terms);
  const lmsEntries: LmsEntry[] = [];
  for (const row of lmsCourses) {
    if (row.hidden) continue;
    const offering = row.offeringId ? offerings.get(row.offeringId) : undefined;
    for (const termId of lmsTerms(row, offering)) {
      if (!scope.has(termId)) continue;
      const gridMeetings = row.timetables?.[termId];
      lmsEntries.push({
        key: `lms:${row.lmsId}:${termId}`,
        termId,
        lms: row,
        offering,
        title: decodeEntities(pickLocalized(offering?.title ?? row.title, locale, row.shortname)),
        code: offering?.code ?? row.shortname.replace(/\[[^\]]*\]\s*$/, ""),
        slots: gridMeetings ? gridSlots(gridMeetings) : offeringSlots(offering),
        gridMeetings,
      });
    }
  }
  const common: MergeDiff["common"] = [];
  const localOnly: LocalEntry[] = [];
  const matched = new Set<string>();
  for (const termId of terms) {
    for (const course of timetableCourses(data, termId)) {
      const match = lmsEntries.find(
        (entry) =>
          entry.termId === termId &&
          !matched.has(entry.key) &&
          ((course.offeringId && course.offeringId === entry.lms.offeringId) ||
            (course.code && baseCode(course.code) === baseCode(entry.code))),
      );
      if (match) {
        matched.add(match.key);
        common.push({
          termId,
          local: course,
          lms: match,
          timeDiffers: Boolean(match.gridMeetings) && !sameSlots(courseSlots(course), match.slots),
        });
      } else
        localOnly.push({
          key: `local:${course.id}`,
          termId,
          course,
          slots: courseSlots(course),
        });
    }
  }
  const lmsOnly = lmsEntries.filter((entry) => !matched.has(entry.key));

  const conflicts: MergeDiff["conflicts"] = new Map();
  let group = 0;
  for (const remote of lmsOnly)
    for (const local of localOnly) {
      const sameCode = baseCode(local.course.code) === baseCode(remote.code) && Boolean(local.course.code);
      const reason =
        sameCode && local.termId !== remote.termId
          ? "quarter"
          : local.termId === remote.termId && local.slots.some((slot) => remote.slots.includes(slot))
            ? "slot"
            : undefined;
      if (!reason) continue;
      const existing = conflicts.get(remote.key) ?? conflicts.get(local.key);
      const id = existing?.group ?? group++;
      conflicts.set(remote.key, { group: id, reason });
      conflicts.set(local.key, { group: id, reason });
    }
  return { terms, common, localOnly, lmsOnly, conflicts };
}

export const hasDifferences = (diff: MergeDiff) =>
  diff.localOnly.length > 0 ||
  diff.lmsOnly.length > 0 ||
  diff.common.some((pair) => pair.timeDiffers);

/**
 * Applies the person's choices: adds the chosen LMS courses, drops the local
 * ones they unchecked, and links LMS materials and assignments to the
 * timetable course they belong to.
 */
export function applyLmsMerge(
  draft: Workspace,
  diff: MergeDiff,
  add: Set<string>,
  keep: Set<string>,
  /** Common courses (`local:{id}` keys) that take the LMS grid's time. */
  retime: Set<string> = new Set(),
): Workspace {
  const now = stamp();
  const locale = draft.settings.locale === "ja" ? "ja" : "en";
  const linked = new Map<number, string>();
  for (const pair of diff.common) linked.set(pair.lms.lms.lmsId, pair.local.id);

  for (const entry of diff.lmsOnly) {
    if (!add.has(entry.key)) continue;
    const year = Number(entry.termId.slice(0, 4));
    const id = entry.lms.offeringId
      ? `course:${entry.termId}:${entry.lms.offeringId}`
      : `course:${entry.termId}:lms-${entry.lms.lmsId}`;
    let course = draft.courses.find((item) => item.id === id);
    const fromSyllabus = entry.offering ? catalogCourse(entry.offering, locale) : undefined;
    const credits = entry.offering?.credits ?? { lecture: 0, exercise: 0, lab: 0 };
    if (!course) {
      course = {
        id,
        kind: "course",
        rev: 1,
        updatedAt: now,
        updatedBy: draft.deviceId,
        termId: entry.termId,
        offeringId: entry.lms.offeringId ?? undefined,
        names: { title: entry.offering?.title ?? entry.lms.title, instructor: entry.offering?.instructors[0] ?? {} },
        code: entry.code,
        title: entry.title,
        instructor: fromSyllabus?.instructor ?? "",
        credits: credits.lecture + credits.exercise + credits.lab,
        creditSplit: credits,
        instructionLanguage:
          (["ja", "en", "ja-en"] as const).find((value) => value === entry.offering?.instructionLanguage) ?? "other",
        color: "#526e91",
        meetings: entry.gridMeetings
          ? meetingsFromGrid(entry.gridMeetings, fromSyllabus?.meetings)
          : (fromSyllabus?.meetings ?? []),
        url: entry.offering?.syllabusUrl ?? entry.lms.url,
        source: { provider: "lms", externalId: String(entry.lms.lmsId), fetchedAt: now },
      };
      draft.courses.push(course);
    } else if (course.deletedAt) {
      Object.assign(course, { deletedAt: undefined, rev: course.rev + 1, updatedAt: now, updatedBy: draft.deviceId });
    }
    if (!draft.memberships.some((item) => item.courseId === id && item.termId === entry.termId && !item.deletedAt))
      draft.memberships.push({
        id: `lms-membership:${entry.lms.lmsId}:${entry.termId}`,
        kind: "membership",
        rev: 1,
        updatedAt: now,
        updatedBy: draft.deviceId,
        termId: entry.termId,
        courseId: id,
        status: "lms-member",
        evidence: { provider: "lms", externalId: String(entry.lms.lmsId), fetchedAt: now },
      });
    if (!linked.has(entry.lms.lmsId)) linked.set(entry.lms.lmsId, id);
  }

  // Courses added from LMS follow its current name (e.g. once `&amp;` is decoded).
  for (const pair of diff.common) {
    const course = draft.courses.find((item) => item.id === pair.local.id);
    if (!course || course.source.provider !== "lms" || course.overrides?.title) continue;
    const names = pair.lms.offering?.title ?? pair.lms.lms.title;
    if (JSON.stringify(course.names?.title ?? {}) === JSON.stringify(names) && course.title === pair.lms.title)
      continue;
    course.names = { title: names, instructor: course.names?.instructor ?? {} };
    course.title = pair.lms.title;
    course.rev += 1;
    course.updatedAt = now;
    course.updatedBy = draft.deviceId;
  }

  for (const pair of diff.common) {
    if (!pair.timeDiffers || !pair.lms.gridMeetings || !retime.has(`local:${pair.local.id}`)) continue;
    const course = draft.courses.find((item) => item.id === pair.local.id);
    if (!course) continue;
    course.meetings = meetingsFromGrid(pair.lms.gridMeetings, course.meetings);
    course.rev += 1;
    course.updatedAt = now;
    course.updatedBy = draft.deviceId;
  }

  for (const entry of diff.localOnly) {
    if (keep.has(entry.key)) continue;
    for (const item of [...draft.planItems, ...draft.memberships])
      if (item.courseId === entry.course.id && item.termId === entry.termId && !item.deletedAt) {
        item.deletedAt = now;
        item.rev += 1;
        item.updatedAt = now;
        item.updatedBy = draft.deviceId;
      }
  }

  for (const item of [...draft.assignments, ...draft.materials]) {
    const lmsCourseId = (item as { lmsCourseId?: number }).lmsCourseId;
    const target = lmsCourseId === undefined ? undefined : linked.get(lmsCourseId);
    if (target && item.courseId !== target) {
      item.courseId = target;
      item.rev += 1;
      item.updatedAt = now;
      item.updatedBy = draft.deviceId;
    }
  }
  return draft;
}
