import type {
  Course,
  PersonalEvent,
  TermId,
  Workspace,
} from "../../core/model";
import { effectiveWeekday } from "../../core/academic";
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
    data.drafts.some((x) => x.courseId === course.id && !x.deletedAt)
  );
}
export function setTimetable(d: Workspace, course: Course, on: boolean) {
  const now = stamp();
  const membership = d.memberships.find(
    (m) => m.courseId === course.id && !m.deletedAt,
  );
  const draft = d.drafts.find((x) => x.courseId === course.id && !x.deletedAt);
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
      status: "enrolled",
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
  const ids = new Set([
    ...data.memberships
      .filter((m) => m.termId === term && !m.deletedAt)
      .map((m) => m.courseId),
    ...data.drafts
      .filter((x) => x.termId === term && !x.deletedAt)
      .map((x) => x.courseId),
  ]);
  return data.courses.filter(
    (c) => ids.has(c.id) && c.termId === term && !c.deletedAt,
  );
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
export function dayBlocks(
  courses: Course[],
  events: PersonalEvent[],
  day: number,
  iso?: string,
  conflicts = new Set<string>(),
  match?: (course: Course) => boolean,
): Block[] {
  const blocks: Block[] = [];
  for (const course of courses)
    for (const [index, meeting] of course.meetings.entries()) {
      if (!meeting.days.includes(day)) continue;
      if (iso && meeting.validFrom && iso < meeting.validFrom) continue;
      if (iso && meeting.validTo && iso > meeting.validTo) continue;
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
    if (!(event.days?.includes(day) || (iso && event.date === iso))) continue;
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
export function weekColumns(
  data: Workspace,
  courses: Course[],
  events: PersonalEvent[],
  conflicts?: Set<string>,
  match?: (course: Course) => boolean,
): DayColumn[] {
  return weekColumnsAt(data, courses, events, new Date(), conflicts, match);
}
export function weekColumnsAt(
  data: Workspace,
  courses: Course[],
  events: PersonalEvent[],
  anchor: Date,
  conflicts?: Set<string>,
  match?: (course: Course) => boolean,
): DayColumn[] {
  const todayIso = jstDate(new Date());
  const anchorIso = jstDate(anchor);
  const dow = new Date(`${anchorIso}T12:00:00+09:00`).getUTCDay();
  const mondayShift = dow === 0 ? -6 : 1 - dow;
  return dayNames.map((label, index) => {
    const date = new Date(`${anchorIso}T12:00:00+09:00`);
    date.setUTCDate(date.getUTCDate() + mondayShift + index);
    const iso = jstDate(date);
    const effective = effectiveWeekday(date, data.exceptions);
    return {
      label,
      date: iso,
      isToday: iso === todayIso,
      blocks: effective
        ? dayBlocks(courses, events, effective, iso, conflicts, match)
        : [],
    };
  });
}
