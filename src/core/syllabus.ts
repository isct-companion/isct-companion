import type {
  Course,
  InstructionLanguage,
  Meeting,
  PlanItem,
  Quarter,
  TermId,
  Workspace,
} from "./model";

import { syllabusIdentity, type SyllabusContent } from "./syllabus-content";

export type SyllabusCourse = {
  externalId: string;
  year: number;
  quarter: "1Q" | "2Q" | "3Q" | "4Q";
  quarters?: ("1Q" | "2Q" | "3Q" | "4Q")[];
  code: string;
  title: string;
  instructor: string;
  credits?: number;
  language?: string;
  meetings?: Meeting[];
  url: string;
  updated?: string;
  content?: SyllabusContent | null;
};

const key = (item: Pick<SyllabusCourse, "externalId" | "year" | "quarter">) =>
  `${item.externalId}:${item.year}-${item.quarter}`;

export const isSyllabusCourse = (course: Course) =>
  course.source.provider === "syllabus" ||
  (course.source.provider === "school" &&
    Boolean(course.url && syllabusIdentity(course.url)));

export const canonicalCourseId = (academicYear: number, offeringId: string) =>
  `${academicYear}-${offeringId}`;

const instructionLanguage = (value?: string): InstructionLanguage => {
  const normalized = value?.toLocaleLowerCase() ?? "";
  if (normalized.includes("japanese") && normalized.includes("english"))
    return "ja-en";
  if (normalized === "japanese" || normalized === "日本語") return "ja";
  if (normalized === "english" || normalized === "英語") return "en";
  return "other";
};

/**
 * Quarters an offering is planned into when added in `quarter`: a 3Q–4Q
 * offering added in one of its quarters goes into all of them; one added
 * outside its quarters stays in the quarter on screen.
 */
const plannedTerms = (
  year: number,
  quarter: Quarter,
  quarters: readonly Quarter[] = [],
): TermId[] =>
  (quarters.includes(quarter) ? quarters : [quarter]).map(
    (item) => `${year}-${item}` as TermId,
  );

export function addSyllabusDraft(
  workspace: Workspace,
  item: SyllabusCourse,
  ids: { course: string; planItem: string },
) {
  const termId = `${item.year}-${item.quarter}` as TermId;
  const termIds = plannedTerms(item.year, item.quarter, item.quarters);
  const courseId = canonicalCourseId(item.year, item.externalId);
  const now = new Date().toISOString();
  let course = workspace.courses.find(
    (candidate) =>
      !candidate.deletedAt &&
      (candidate.id === courseId ||
        (isSyllabusCourse(candidate) &&
          candidate.source.externalId === item.externalId)),
  );
  if (!course) {
    course = {
      id: courseId,
      kind: "course",
      rev: 1,
      updatedAt: now,
      updatedBy: workspace.deviceId,
      termId,
      offeringId: item.externalId,
      names: { title: { en: item.title }, instructor: { en: item.instructor } },
      code: item.code,
      title: item.title,
      instructor: item.instructor,
      credits: item.credits ?? 0,
      creditSplit: { lecture: item.credits ?? 0, exercise: 0, lab: 0 },
      instructionLanguage: instructionLanguage(item.language),
      color: "#556b8d",
      meetings: item.meetings ?? [],
      url: item.url,
      source: {
        provider: "syllabus",
        externalId: item.externalId,
        fetchedAt: now,
      },
    };
    workspace.courses.push(course);
  } else {
    Object.assign(course, {
      url: item.url,
      rev: course.rev + 1,
      updatedAt: now,
      updatedBy: workspace.deviceId,
      source: {
        ...course.source,
        provider: "syllabus" as const,
        externalId: item.externalId,
        fetchedAt: now,
      },
    });
  }
  /** Plan items this add put on the timetable (new or brought back). */
  const placed: PlanItem[] = [];
  const drafts = termIds.map((planTerm) => {
    let draft = workspace.planItems.find(
      (candidate) =>
        candidate.termId === planTerm && candidate.courseId === course!.id,
    );
    const active = Boolean(draft && !draft.deletedAt);
    if (draft) {
      draft.deletedAt = undefined;
      draft.rev += 1;
      draft.updatedAt = now;
      draft.updatedBy = workspace.deviceId;
    } else {
      draft = {
        // The quarter on screen takes the given ID; the others derive from it.
        id: planTerm === termId ? ids.planItem : `${ids.planItem}:${planTerm}`,
        kind: "plan-item",
        rev: 1,
        updatedAt: now,
        updatedBy: workspace.deviceId,
        termId: planTerm,
        courseId: course!.id,
        offeringId: item.externalId,
        state: "planned",
      };
      workspace.planItems.push(draft);
    }
    if (!active) placed.push(draft);
    return draft;
  });
  return { course, drafts, placed, sourceKey: key(item) };
}

export function removeSyllabusDraft(
  workspace: Workspace,
  courseId: string,
  termId: TermId,
) {
  const draft = workspace.planItems.find(
    (candidate) =>
      candidate.courseId === courseId &&
      candidate.termId === termId &&
      !candidate.deletedAt,
  );
  if (!draft) return false;
  const now = new Date().toISOString();
  draft.deletedAt = now;
  draft.rev += 1;
  draft.updatedAt = now;
  draft.updatedBy = workspace.deviceId;
  return true;
}
