import type { FinderCourse } from "./course-query";
import type { Course, TermId } from "./model";
import type { LmsCourseRecord } from "../platform/desktop-bridge";

export type LmsEntry = {
  key: string;
  termId: TermId;
  lms: LmsCourseRecord;
  title: string;
  code: string;
  offering?: FinderCourse;
  slots: string[];
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
  common: {
    termId: TermId;
    local: Course;
    lms: LmsEntry;
    timeDiffers: boolean;
  }[];
  localOnly: LocalEntry[];
  lmsOnly: LmsEntry[];
  conflicts: Map<string, { group: number; reason: "slot" | "quarter" }>;
};

/** Serializable Core boundary; the dialog adapter converts `conflicts` to a Map. */
export type CoreLmsPreview = Omit<MergeDiff, "conflicts"> & {
  detailIds: string[];
  conflicts: Record<string, { group: number; reason: "slot" | "quarter" }>;
  hasDifferences: boolean;
};
