import { seedWorkspace } from "@app/lib/model";
import type { Course, PlanItem, Workspace } from "@app/lib/model";
import type { FinderCourse } from "@app/generated/persistence";
import type { SchoolRecords } from "@app/platform/desktop-bridge";

/**
 * Data builders shared by the suites.
 *
 * The academic year is pinned to 2026 because the school's published quarter
 * dates, the drift-checked syllabus fixtures and the live site all agree on it.
 */
export const YEAR = 2026;
export const TERM = "2026-3Q" as const;
export const TERM_RANGE = { from: "2026-09-28", to: "2026-12-02" };
export const SYLLABUS_SCOPE = "4/0-904-342300-0-0";
export const DEVICE = "device:test";
export const UPDATED_AT = "2026-10-01T00:00:00.000Z";

export function emptySchoolRecords(): SchoolRecords {
  return {
    lastRuns: {},
    pageAvailability: {},
    lmsCourses: [],
    registrations: [],
    registrationStates: [],
    grades: [],
    gpaByTerm: [],
    requirements: [],
  };
}

/** A syllabus offering as the finder and catalog return it. */
export function offering(
  id: string,
  code: string,
  title: string,
  overrides: Partial<FinderCourse> = {},
): FinderCourse {
  return {
    id,
    year: YEAR,
    quarters: ["3Q"],
    code,
    title: { ja: title, en: title },
    instructors: [{ ja: "教員", en: "Teacher" }],
    instructionLanguage: "ja",
    format: { kinds: ["lecture"] },
    credits: { lecture: 2, exercise: 0, lab: 0 },
    schedule: [{ kind: "weekly", day: 2, periods: [7, 8] }],
    syllabusUrl: `https://syllabus.s.isct.ac.jp/courses/${YEAR}/${SYLLABUS_SCOPE}/${id}`,
    ...overrides,
  };
}

/** The workspace row the shell renders for a placed offering. */
export function courseFor(
  source: FinderCourse,
  overrides: Partial<Course> = {},
): Course {
  const title = source.title?.en ?? source.title?.ja ?? source.code;
  return {
    id: `${YEAR}-${source.id}`,
    kind: "course",
    rev: 1,
    updatedAt: UPDATED_AT,
    updatedBy: DEVICE,
    termId: TERM,
    offeringId: source.id,
    names: { title: source.title, instructor: source.instructors?.[0] },
    code: source.code,
    title,
    instructor: "Teacher",
    credits: 2,
    creditSplit: source.credits,
    instructionLanguage: "ja",
    color: "#556b8d",
    meetings: [
      {
        days: [2],
        periods: [7, 8],
        startTime: "15:25",
        endTime: "17:05",
        room: "",
      },
    ],
    url: source.syllabusUrl,
    source: { provider: "syllabus", externalId: source.id },
    ...overrides,
  } as Course;
}

export function planItemFor(
  course: Course,
  overrides: Partial<PlanItem> = {},
): PlanItem {
  return {
    id: "018f0000-0000-7000-8000-000000000002",
    kind: "plan-item",
    rev: 1,
    updatedAt: UPDATED_AT,
    updatedBy: DEVICE,
    termId: TERM,
    courseId: course.id,
    offeringId: course.offeringId,
    state: "planned",
    ...overrides,
  } as PlanItem;
}

/** A workspace with the term, device and any extra rows pinned for a suite. */
export function workspaceFixture(
  rows: { courses?: Course[]; planItems?: PlanItem[] } = {},
): Workspace {
  const workspace = seedWorkspace();
  workspace.deviceId = DEVICE;
  workspace.selectedTerm = TERM;
  workspace.currentTerm = TERM;
  workspace.terms = [TERM];
  for (const task of workspace.scheduledTasks) {
    task.updatedAt = UPDATED_AT;
    task.updatedBy = DEVICE;
  }
  workspace.courses.push(...(rows.courses ?? []));
  workspace.planItems.push(...(rows.planItems ?? []));
  return workspace;
}

export function overviewFor(
  workspace: Workspace,
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  const active = workspace.courses.filter((row) => !row.deletedAt);
  const colors: Record<string, string> = {};
  for (const row of active) colors[row.id] = row.color;
  return {
    term: TERM,
    currentTerm: TERM,
    selectedTerm: TERM,
    todayTerm: TERM,
    coursesTerm: TERM,
    range: TERM_RANGE,
    courses: active,
    termCourseIds: active.map((row) => row.id),
    declaredCredits: null,
    termCredits: active.reduce((total, row) => total + row.credits, 0),
    colors,
    enrolledCourseIds: active.map((row) => row.id),
    onTimetableCourseIds: active.map((row) => row.id),
    allTerms: [TERM],
    defaultFinderKeys: [],
    ...overrides,
  };
}

export function planningWeekFor(
  workspace: Workspace,
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  const active = workspace.courses.filter((row) => !row.deletedAt);
  // Blocks land on the day each meeting names, so a fixture course with a
  // Tuesday and a Friday meeting produces blocks on both.
  const blocks = active.flatMap((row) =>
    row.meetings.flatMap((meeting, slot) =>
      meeting.days.map((day) => ({
        key: `${row.id}-${slot}-${day}`,
        id: row.id,
        courseId: row.id,
        kind: "course",
        date: "",
        title: row.title,
        subtitle: row.code,
        room: meeting.room ?? "",
        startTime: meeting.startTime ?? "15:25",
        endTime: meeting.endTime ?? "17:05",
        color: row.color,
        day,
      })),
    ),
  );
  return {
    courses: active,
    days: Array.from({ length: 7 }, (_, index) => ({
      day: index + 1,
      blocks: blocks
        .filter((block) => block.day === index + 1)
        .map(({ day: _day, ...block }) => block),
    })),
    conflictCount: 0,
    conflicts: [],
    ...overrides,
  };
}

export function scheduleViewFor(
  workspace: Workspace,
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return { ...overviewFor(workspace), agendas: [], problems: [], ...overrides };
}
