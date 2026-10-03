import type { Course, Workspace } from "./model";

export type CourseFilters = {
  query: string;
  language: string;
  weekday: number;
  period: number;
  status: "all" | "planned" | "available" | "conflict";
};

export function filterCourseCatalog(
  courses: Course[],
  filters: CourseFilters,
  plannedIds: ReadonlySet<string>,
  conflictIds: ReadonlySet<string>,
) {
  const tokens = filters.query
    .trim()
    .toLocaleLowerCase()
    .split(/\s+/)
    .filter(Boolean);
  return courses.filter((course) => {
    const haystack =
      `${course.title} ${course.code} ${course.instructor} ${course.instructionLanguage} ${course.meetings.map((meeting) => meeting.room).join(" ")}`.toLocaleLowerCase();
    const meeting =
      (!filters.weekday && !filters.period) ||
      course.meetings.some(
        (item) =>
          (!filters.weekday || item.days.includes(filters.weekday)) &&
          (!filters.period || item.periods.includes(filters.period)),
      );
    const planned = plannedIds.has(course.id);
    return (
      tokens.every((token) => haystack.includes(token)) &&
      (filters.language === "all" ||
        course.instructionLanguage === filters.language) &&
      meeting &&
      (filters.status === "all" ||
        (filters.status === "planned" && planned) ||
        (filters.status === "available" && !planned) ||
        (filters.status === "conflict" && conflictIds.has(course.id)))
    );
  });
}

export function deleteLocalCourseBatch(
  workspace: Workspace,
  ids: ReadonlySet<string>,
) {
  const now = new Date().toISOString();
  const records = [
    ...workspace.courses.filter(
      (course) => ids.has(course.id) && !course.deletedAt,
    ),
    ...workspace.memberships.filter(
      (item) => ids.has(item.courseId) && !item.deletedAt,
    ),
    ...workspace.planItems.filter(
      (item) => Boolean(item.courseId && ids.has(item.courseId)) && !item.deletedAt,
    ),
  ];
  for (const item of records) {
    item.deletedAt = now;
    item.updatedAt = now;
    item.updatedBy = workspace.deviceId;
    item.rev += 1;
  }
  return records.length;
}

export function editCourseBatch(
  workspace: Workspace,
  ids: ReadonlySet<string>,
  changes: { language?: string; instructor?: string; credits?: number },
) {
  if (
    changes.credits !== undefined &&
    (!Number.isFinite(changes.credits) ||
      changes.credits < 0 ||
      changes.credits > 20)
  )
    throw new Error("Credits must be between 0 and 20.");
  const language = changes.language?.trim() || undefined;
  const instructor = changes.instructor?.trim() || undefined;
  if (!language && !instructor && changes.credits === undefined) return 0;
  const now = new Date().toISOString();
  const courses = workspace.courses.filter(
    (course) => ids.has(course.id) && !course.deletedAt,
  );
  for (const course of courses) {
    if (language && ["ja", "en", "ja-en", "other"].includes(language))
      course.instructionLanguage =
        language as Course["instructionLanguage"];
    if (instructor) course.instructor = instructor;
    if (changes.credits !== undefined) course.credits = changes.credits;
    course.rev += 1;
    course.updatedAt = now;
    course.updatedBy = workspace.deviceId;
  }
  return courses.length;
}

export function visibleSelection<T extends { id: string }>(
  items: T[],
  selected: ReadonlySet<string>,
) {
  return items.filter((item) => selected.has(item.id));
}
