import type { Course, Meeting, TermId, Workspace } from "./model";

import {
  cacheSyllabusContent,
  syllabusIdentity,
  type SyllabusContent,
} from "./syllabus-content";

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

export function addSyllabusDraft(workspace: Workspace, item: SyllabusCourse) {
  if (item.content) cacheSyllabusContent(item.content);
  const termId = `${item.year}-${item.quarter}` as TermId;
  const now = new Date().toISOString();
  let course = workspace.courses.find(
    (candidate) =>
      !candidate.deletedAt &&
      candidate.termId === termId &&
      isSyllabusCourse(candidate) &&
      candidate.source.externalId === item.externalId,
  );
  if (!course) {
    course = {
      id: crypto.randomUUID(),
      kind: "course",
      rev: 1,
      updatedAt: now,
      updatedBy: workspace.deviceId,
      termId,
      code: item.code,
      title: item.title,
      syllabusSource: {
        title: item.title,
        instructor: item.instructor,
        language: item.language ?? "",
      },
      instructor: item.instructor,
      credits: item.credits ?? 0,
      language: item.language ?? "",
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
  let draft = workspace.drafts.find(
    (candidate) =>
      candidate.termId === termId && candidate.courseId === course!.id,
  );
  if (draft) {
    draft.deletedAt = undefined;
    draft.rev += 1;
    draft.updatedAt = now;
    draft.updatedBy = workspace.deviceId;
  } else {
    draft = {
      id: crypto.randomUUID(),
      kind: "draft",
      rev: 1,
      updatedAt: now,
      updatedBy: workspace.deviceId,
      termId,
      courseId: course.id,
    };
    workspace.drafts.push(draft);
  }
  return { course, draft, sourceKey: key(item) };
}

export function removeSyllabusDraft(
  workspace: Workspace,
  courseId: string,
  termId: TermId,
) {
  const draft = workspace.drafts.find(
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
