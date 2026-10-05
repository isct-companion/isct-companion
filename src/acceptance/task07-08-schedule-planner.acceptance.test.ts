import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { detectConflicts } from "../core/academic";
import { filterCourseCatalog } from "../core/course-workflows";
import type { Course, PersonalEvent } from "../core/model";
import { seedWorkspace } from "../core/model";
import { intensiveDays } from "../core/user-schedule";
import { weekColumnsAt } from "../components/timetable/workspace";

type PublicCourse = {
  externalId: string;
  year: number;
  quarter: "1Q" | "2Q" | "3Q" | "4Q";
  code: string;
  title: string;
  instructor: string;
  credits: number;
  language: string;
  meetings: Course["meetings"];
  url: string;
  content: { sourceUrl: string; fetchedAt: string };
};

const parsedPath = fileURLToPath(
  new URL("../../artifacts/syllabus-research/course-examples-parsed.json", import.meta.url),
);
const provenancePath = fileURLToPath(
  new URL("../../artifacts/syllabus-research/content-provenance.json", import.meta.url),
);
const publicCourses = JSON.parse(readFileSync(parsedPath, "utf8")) as PublicCourse[];
const provenance = JSON.parse(readFileSync(provenancePath, "utf8")) as {
  observedDate: string;
  fixtures: { file: string; url: string }[];
};

function asCourse(source: PublicCourse): Course {
  return {
    id: `syllabus:${source.externalId}`,
    rev: 1,
    updatedAt: source.content.fetchedAt,
    updatedBy: "public-syllabus-fixture",
    kind: "course",
    termId: `${source.year}-${source.quarter}`,
    code: source.code,
    title: source.title,
    instructor: source.instructor,
    credits: source.credits,
    creditSplit: { lecture: source.credits, exercise: 0, lab: 0 },
    instructionLanguage:
      source.language === "Japanese"
        ? "ja"
        : source.language === "English"
          ? "en"
          : "other",
    color: "#52657a",
    meetings: source.meetings,
    url: source.url,
    source: {
      provider: "syllabus",
      externalId: source.externalId,
      fetchedAt: source.content.fetchedAt,
    },
  };
}

describe("Task 7 intensive dates", () => {
  it("quick-sets the inclusive range and removal keeps that day out of Schedule", () => {
    const workspace = seedWorkspace();
    const course: Course = {
      id: "local-intensive-course",
      rev: 1,
      updatedAt: "2026-09-29T00:00:00Z",
      updatedBy: workspace.deviceId,
      kind: "course",
      termId: "2026-3Q",
      code: "LOCAL.INTENSIVE",
      title: "Local intensive course",
      instructor: "",
      credits: 1,
      creditSplit: { lecture: 1, exercise: 0, lab: 0 },
      instructionLanguage: "other",
      color: "#52657a",
      meetings: [],
      source: { provider: "manual" },
    };
    workspace.courses = [course];
    const quickSet = intensiveDays("2026-10-20", "2026-10-22", [1, 2]);
    expect(quickSet).toEqual([
      { date: "2026-10-20", periods: [1, 2] },
      { date: "2026-10-21", periods: [1, 2] },
      { date: "2026-10-22", periods: [1, 2] },
    ]);
    workspace.intensiveSchedules = [
      {
        id: "local-intensive-schedule",
        rev: 1,
        updatedAt: "2026-09-29T00:00:00Z",
        updatedBy: workspace.deviceId,
        courseId: course.id,
        from: "2026-10-20",
        to: "2026-10-22",
        periods: [1, 2],
        days: quickSet.filter((day) => day.date !== "2026-10-21"),
      },
    ];

    const columns = weekColumnsAt(
      workspace,
      [course],
      [],
      new Date("2026-10-20T12:00:00+09:00"),
    );
    const datesWithCourse = columns
      .filter((column) => column.blocks.some((block) => block.id === course.id))
      .map((column) => column.date);
    expect(datesWithCourse).toEqual(["2026-10-20", "2026-10-22"]);
    expect(
      columns.flatMap((column) => column.blocks).find((block) => block.id === course.id),
    ).toMatchObject({ startTime: "08:50", endTime: "10:30" });
  });
});

describe("Task 8 planner slot and conflict rules", () => {
  it("selects the real public Tue 3–4 offering in both covered periods", () => {
    const parallelFixture = provenance.fixtures.find(
      (item) => item.file === "parallel-en-content.html",
    );
    expect(provenance.observedDate).toBe("2026-09-10");
    expect(parallelFixture?.url).toBe(
      "https://syllabus.s.isct.ac.jp/courses/2026/4/0-904-342300-0-0/202602440?hl=en",
    );

    const courses = publicCourses.map(asCourse);
    for (const period of [3, 4]) {
      const result = filterCourseCatalog(
        courses,
        { query: "", language: "all", weekday: 2, period, status: "all" },
        new Set(),
        new Set(),
      );
      expect(result.map((course) => [course.code, course.title])).toEqual([
        ["CSC.T354", "Parallel Programming"],
      ]);
    }
    const parallel = courses.find((course) => course.code === "CSC.T354")!;
    expect(parallel.meetings).toEqual([
      expect.objectContaining({ days: [2], periods: [3, 4] }),
      expect.objectContaining({ days: [5], periods: [3, 4] }),
    ]);
    expect(parallel.url).toBe(parallelFixture?.url);
  });

  it("flags a collision between the real two-slot course and a local blocking item", () => {
    const parallel = asCourse(
      publicCourses.find((course) => course.code === "CSC.T354")!,
    );
    const localBlock: PersonalEvent = {
      id: "local-block",
      rev: 1,
      updatedAt: "2026-09-29T00:00:00Z",
      updatedBy: "local-device",
      kind: "event",
      termId: parallel.termId,
      title: "Local calendar block",
      room: "",
      days: [2],
      startTime: "11:00",
      endTime: "11:30",
      blocking: true,
    };

    expect(detectConflicts([parallel], [localBlock])).toEqual([
      {
        leftId: parallel.id,
        rightId: localBlock.id,
        day: 2,
        startTime: "11:00",
        endTime: "11:30",
      },
    ]);
  });
});
