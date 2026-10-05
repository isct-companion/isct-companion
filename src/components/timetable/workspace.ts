import type {
  Course,
  PersonalEvent,
  TermId,
  Workspace,
} from "../../core/model";
import { calendarState, occurrencesOnDate, quarterRange, scheduledCourses } from "../../core/schedule";
import { dayNames, type Block, type DayColumn } from "./timetable";
import { stamp, jstDate } from "../../app/shared";

export function enrolledCourses(data: Workspace, term: TermId) {
  const ids = new Set(
    data.memberships
      .filter((m) => m.termId === term && !m.deletedAt)
      .map((m) => m.courseId),
  );
  return data.courses.filter((c) => ids.has(c.id) && !c.deletedAt);
}
export function onTimetable(data: Workspace, course: Course) {
  return (
    data.memberships.some((m) => m.courseId === course.id && !m.deletedAt) ||
    data.planItems.some((x) => x.courseId === course.id && !x.deletedAt)
  );
}
export function setTimetable(d: Workspace, course: Course, on: boolean) {
  const now = stamp();
  const membership = d.memberships.find(
    (m) => m.courseId === course.id && !m.deletedAt,
  );
  const draft = d.planItems.find(
    (x) => x.courseId === course.id && !x.deletedAt,
  );
  if (on) {
    if (membership || draft) return;
    d.memberships.push({
      id: crypto.randomUUID(),
      kind: "membership",
      rev: 1,
      updatedAt: now,
      updatedBy: d.deviceId,
      termId: course.termId,
      courseId: course.id,
      status: "declared",
      evidence: { provider: "manual" },
    });
    return;
  }
  for (const item of [membership, draft])
    if (item) {
      item.deletedAt = now;
      item.rev += 1;
      item.updatedAt = now;
    }
}
export function timetableCourses(data: Workspace, term: TermId) {
  return scheduledCourses(data, term);
}
export function allTerms(data: Workspace, currentTerm: TermId): TermId[] {
  return [
    ...new Set<TermId>([
      ...data.courses.filter((c) => !c.deletedAt).map((c) => c.termId),
      ...(data.terms ?? []),
      currentTerm,
    ]),
  ].sort();
}
function templateDayBlocks(
  courses: Course[],
  events: PersonalEvent[],
  day: number,
  conflicts = new Set<string>(),
  match?: (course: Course) => boolean,
): Block[] {
  const blocks: Block[] = [];
  for (const course of courses)
    for (const [index, meeting] of course.meetings.entries()) {
      if (!meeting.days.includes(day)) continue;
      blocks.push({
        key: `${course.id}-${index}-${day}`,
        id: course.id,
        kind: "course",
        title: course.overrides?.title || course.title,
        subtitle: course.code,
        room: meeting.room,
        startTime: meeting.startTime,
        endTime: meeting.endTime,
        color: course.overrides?.color || course.color,
        conflict: conflicts.has(course.id),
        dim: match ? !match(course) : false,
      });
    }
  for (const event of events) {
    if (event.deletedAt) continue;
    if (event.date || !event.days?.includes(day)) continue;
    blocks.push({
      key: `${event.id}-${day}`,
      id: event.id,
      kind: "event",
      title: event.title,
      room: event.room,
      startTime: event.startTime,
      endTime: event.endTime,
      color: "#8a887f",
      conflict: conflicts.has(event.id),
    });
  }
  return blocks.sort((a, b) => a.startTime.localeCompare(b.startTime));
}
export function weekColumnsAt(
  data: Workspace,
  courses: Course[],
  events: PersonalEvent[],
  anchor: Date,
  conflicts?: Set<string>,
  match?: (course: Course) => boolean,
  term: TermId = courses[0]?.termId ?? events[0]?.termId ?? data.selectedTerm,
): DayColumn[] {
  const todayIso = jstDate(new Date());
  const anchorIso = jstDate(anchor);
  const dow = new Date(`${anchorIso}T12:00:00+09:00`).getUTCDay();
  const mondayShift = dow === 0 ? -6 : 1 - dow;
  return dayNames.map((label, index) => {
    const date = new Date(`${anchorIso}T12:00:00+09:00`);
    date.setUTCDate(date.getUTCDate() + mondayShift + index);
    const iso = jstDate(date);
    const calendar = calendarState(data, iso);
    const rawLabel = calendar?.label;
    const range = quarterRange(data, term);
    const outsideTeaching = iso >= range.from && iso <= range.to && Boolean(range.teachingRanges && calendar?.type !== "replacement" && !range.teachingRanges.some(r => iso >= r.start && iso <= r.end));
    const noClasses = outsideTeaching || ["holiday", "no-class", "break"].includes(calendar?.type ?? "");
    const fullNotice = typeof rawLabel === "string" ? rawLabel : rawLabel ? rawLabel[data.settings.locale === "ja" ? "ja" : "en"] || rawLabel.en || rawLabel.ja : noClasses ? (data.settings.locale === "ja" ? "授業休み" : "No classes") : undefined;
    const blocks = occurrencesOnDate(data, term, iso, {
      courses,
      includeCancelledCourses: true,
      events,
      conflicts,
      match,
    });
    return {
      label,
      date: iso,
      isToday: iso === todayIso,
      calendarNotice: fullNotice?.replace(/^No classes due to\s+/i, ""),
      noClasses,
      blocks: blocks.sort((a, b) => a.startTime.localeCompare(b.startTime)),
    };
  });
}

/** Courses is a planning template, including future quarters, not an actual dated week. */
export function templateWeekColumns(
  courses: Course[],
  events: PersonalEvent[],
  conflicts?: Set<string>,
): DayColumn[] {
  return dayNames.map((label, index) => ({
    label,
    blocks: templateDayBlocks(courses, events, index + 1, conflicts),
  }));
}
