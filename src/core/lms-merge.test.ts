import { describe, expect, it } from "vitest";
import { applyLmsMerge, baseCode, diffLms, hasDifferences } from "./lms-merge";
import { seedWorkspace, type Course, type TermId, type Workspace } from "./model";
import type { FinderCourse } from "./course-query";
import type { LmsCourseRecord } from "../platform/desktop-bridge";

const term: TermId = "2026-3Q";
const course = (id: string, code: string, offeringId?: string, day = 1, periods = [3, 4]): Course => ({
  id,
  kind: "course",
  rev: 1,
  updatedAt: "2026-09-01T00:00:00Z",
  updatedBy: "test",
  termId: term,
  offeringId,
  code,
  title: id,
  instructor: "",
  credits: 2,
  creditSplit: { lecture: 2, exercise: 0, lab: 0 },
  instructionLanguage: "ja",
  color: "#000",
  meetings: [{ days: [day], periods, startTime: "10:45", endTime: "12:25", room: "" }],
  source: { provider: "syllabus", externalId: offeringId },
});
const planned = (data: Workspace, item: Course) => {
  data.courses.push(item);
  data.planItems.push({
    id: `plan:${item.id}`,
    kind: "plan-item",
    rev: 1,
    updatedAt: item.updatedAt,
    updatedBy: "test",
    termId: item.termId,
    courseId: item.id,
    state: "planned",
  });
};
const lms = (lmsId: number, shortname: string, offeringId?: string, quarters = ["3Q"]): LmsCourseRecord => ({
  id: `lms-course:${lmsId}`,
  lmsId,
  offeringId,
  shortname,
  title: { ja: shortname },
  year: 2026,
  quarters,
  url: `https://lms.s.isct.ac.jp/2025/course/view.php?id=${lmsId}`,
});
const offering = (id: string, code: string, day: number, periods: [number, number], quarters: FinderCourse["quarters"] = ["3Q"]): FinderCourse => ({
  id,
  code,
  year: 2026,
  quarters,
  title: { ja: code, en: code },
  instructors: [{ ja: "教員" }],
  credits: { lecture: 2, exercise: 0, lab: 0 },
  instructionLanguage: "ja",
  format: { kinds: ["lecture"] },
  schedule: [{ kind: "weekly", day, periods }],
  syllabusUrl: `https://syllabus.s.isct.ac.jp/courses?jwc=${id}`,
  detailed: true,
});

describe("LMS pull merge", () => {
  it("normalizes class suffixes and LMS year tags", () => {
    expect(baseCode("LAE.E213-53")).toBe("LAE.E213");
    expect(baseCode("LAH.T209[2026]")).toBe("LAH.T209");
  });

  it("splits common, local-only and LMS-only courses and pairs slot conflicts", () => {
    const data = seedWorkspace();
    planned(data, course("same", "CSC.T273", "202602431"));
    planned(data, course("mine", "CSC.T999", undefined, 2, [3, 4]));
    planned(data, course("by-code", "CSC.T262-01"));
    const offerings = new Map([["202602440", offering("202602440", "CSC.T271", 2, [3, 4])]]);
    const diff = diffLms(
      data,
      [lms(1, "CSC.T273[2026]", "202602431"), lms(2, "CSC.T271[2026]", "202602440"), lms(3, "CSC.T262[2026]")],
      [term],
      offerings,
    );
    expect(diff.common.map((pair) => pair.local.id).sort()).toEqual(["by-code", "same"]);
    expect(diff.localOnly.map((entry) => entry.course.id)).toEqual(["mine"]);
    expect(diff.lmsOnly.map((entry) => entry.lms.lmsId)).toEqual([2]);
    expect(hasDifferences(diff)).toBe(true);
    const local = diff.conflicts.get("local:mine");
    expect(local?.reason).toBe("slot");
    expect(diff.conflicts.get(diff.lmsOnly[0].key)?.group).toBe(local?.group);
  });

  it("limits a current-quarter pull to that quarter", () => {
    const data = seedWorkspace();
    const diff = diffLms(data, [lms(1, "A.B100[2026]", undefined, ["4Q"])], [term], new Map());
    expect(hasDifferences(diff)).toBe(false);
  });

  it("adds chosen LMS courses, drops unchecked local ones and relinks assignments", () => {
    const data = seedWorkspace();
    planned(data, course("mine", "CSC.T999"));
    data.assignments.push({
      id: "lms-assignment:9",
      kind: "assignment",
      rev: 1,
      updatedAt: "2026-09-01T00:00:00Z",
      updatedBy: "lms",
      courseId: "course:2026:202602440",
      title: "Report",
      details: "",
      dueAt: "2026-10-10T00:00:00Z",
      lmsCmid: 9,
      lmsCourseId: 2,
      source: { provider: "lms" },
    });
    const offerings = new Map([["202602440", offering("202602440", "CSC.T271", 2, [5, 6])]]);
    const diff = diffLms(data, [lms(2, "CSC.T271[2026]", "202602440")], [term], offerings);
    const draft = applyLmsMerge(structuredClone(data), diff, new Set([diff.lmsOnly[0].key]), new Set());
    const added = draft.courses.find((item) => item.offeringId === "202602440");
    expect(added?.meetings[0].days).toEqual([2]);
    expect(added?.creditSplit.lecture).toBe(2);
    expect(draft.memberships.some((item) => item.courseId === added?.id && item.status === "lms-member")).toBe(true);
    expect(draft.planItems.find((item) => item.courseId === "mine")?.deletedAt).toBeTruthy();
    expect(draft.assignments[0].courseId).toBe(added?.id);
  });

  it("takes weekly slots from the LMS timetable grid before the syllabus", () => {
    const data = seedWorkspace();
    planned(data, course("same", "CSC.T262", "202602427", 1, [1, 2]));
    const grid = { [term]: [{ day: 2, periods: [3, 4] as [number, number] }, { day: 5, periods: [3, 4] as [number, number] }] };
    const offerings = new Map([["202602440", offering("202602440", "CSC.T271", 1, [1, 2])]]);
    const diff = diffLms(
      data,
      [
        { ...lms(1, "CSC.T262[2026]", "202602427"), timetables: grid },
        { ...lms(2, "CSC.T271[2026]", "202602440", []), timetables: { [term]: [{ day: 3, periods: [5, 6] }] } },
      ],
      [term],
      offerings,
    );
    // The grid places a course even when the LMS category does not.
    expect(diff.lmsOnly.map((entry) => entry.termId)).toEqual([term]);
    expect(diff.lmsOnly[0].slots).toEqual(["3:5", "3:6"]);
    expect(diff.common[0].timeDiffers).toBe(true);
    expect(hasDifferences(diff)).toBe(true);

    const draft = applyLmsMerge(
      structuredClone(data),
      diff,
      new Set([diff.lmsOnly[0].key]),
      new Set(),
      new Set(["local:same"]),
    );
    expect(draft.courses.find((item) => item.id === "same")?.meetings.map((m) => [m.days[0], m.periods])).toEqual([
      [2, [3, 4]],
      [5, [3, 4]],
    ]);
    const added = draft.courses.find((item) => item.offeringId === "202602440");
    expect(added?.meetings.map((m) => [m.days[0], m.periods, m.startTime])).toEqual([[3, [5, 6], "13:30"]]);
  });
});
