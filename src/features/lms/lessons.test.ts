// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { courseLessons, frontierLesson, lessonNumber, lessonSummary, gravityTarget, markedLesson, markSchedule, railLabels } from "./lessons";

describe("lessons", () => {
  it("reads the lesson number from common section titles", () => {
    expect(lessonNumber("第2回")).toBe(2);
    expect(lessonNumber("第 12 回 クラス")).toBe(12);
    expect(lessonNumber("第３回")).toBe(3);
    expect(lessonNumber("Lecture 4")).toBe(4);
    expect(lessonNumber("Week 10: Graphs")).toBe(10);
    expect(lessonNumber("5. Recursion")).toBe(5);
    expect(lessonNumber("一般")).toBeUndefined();
    expect(lessonNumber("連絡先・オフィスアワー")).toBeUndefined();
    expect(lessonNumber("2026年度 講義資料")).toBeUndefined();
  });

  it("summarises a lesson by the first line of its plan", () => {
    expect(lessonSummary("<p>授業計画：正規文法<br>課題：確認する</p>")).toBe("正規文法");
    expect(lessonSummary("<p>Course schedule: Basic syntax</p><p>Objectives: …</p>")).toBe("Basic syntax");
    expect(lessonSummary(undefined)).toBe("");
  });

  it("needs two numbered sections, and stops at the last lesson with material", () => {
    const sections = [
      { id: 1, title: "一般" },
      { id: 2, title: "第1回" },
      { id: 3, title: "第2回" },
      { id: 4, title: "第3回" },
    ];
    const lessons = courseLessons(sections, (id) => id <= 3);
    expect(lessons.map((l) => l.number)).toEqual([1, 2, 3]);
    expect(frontierLesson(lessons)?.number).toBe(2);
    expect(frontierLesson(courseLessons(sections, () => false))?.number).toBe(1);
    expect(courseLessons(sections.slice(0, 2), () => true)).toEqual([]);
  });

  it("marks a schedule table's past, current and cancelled rows", () => {
    const html = "<table><tr><th>日付</th><th>授業回目</th></tr><tr><td>10/2</td><td>1回目</td></tr><tr><td>10/16</td><td>休講</td></tr><tr><td>10/20</td><td>2回目</td></tr><tr><td>10/23</td><td>3回目</td></tr></table>";
    const page = new DOMParser().parseFromString(markSchedule(html, 2), "text/html");
    const rows = [...page.querySelectorAll("tr")].map((row) => row.className);
    expect(rows).toEqual(["", "lmsRowPast", "lmsRowOff lmsRowPast", "lmsRowNow", ""]);
    expect(markSchedule("<p>no table</p>", 2)).toBe("<p>no table</p>");
  });

  it("labels the rail by order when the titles' numbers do not count up", () => {
    const at = (numbers: number[]) => numbers.map((number, id) => ({ id, number, posted: true }));
    expect(railLabels(at([0, 1, 2]))).toEqual(["0", "1", "2"]);
    expect(railLabels(at([7, 9, 8, 11, 11]))).toEqual(["1", "2", "3", "4", "5"]);
  });

  it("keeps a marked lesson until a later class starts, then moves on", () => {
    const lessons = [1, 2, 3].map((number) => ({ id: 10 + number, number, posted: true }));
    const log = [
      { occurrence: "2026-10-06T15:25:00+09:00", sectionId: 11 },
      { occurrence: "2026-10-08T10:45:00+09:00", sectionId: 12 },
    ];
    // The marked class itself has started (or ended): still its lesson.
    expect(markedLesson(lessons, log, "2026-10-08T10:45:00+09:00")?.number).toBe(2);
    // The next week's class has started: the course moved on.
    expect(markedLesson(lessons, log, "2026-10-15T10:45:00+09:00")?.number).toBe(3);
    expect(markedLesson(lessons, null, "2026-10-15T10:45:00+09:00")).toBeUndefined();
  });

  it("settles near a section boundary and leaves the middle of a section alone", () => {
    const height = 500;
    const boundaries = [1000, 1600];
    // A boundary in the top band: the new section's start comes to the top.
    expect(gravityTarget(940, height, boundaries)).toBe(1000);
    // Just past it, above the top: drawn back to the start.
    expect(gravityTarget(1050, height, boundaries)).toBe(1000);
    // A boundary in the bottom band: the old section's end comes back to the bottom.
    expect(gravityTarget(560, height, boundaries)).toBe(500);
    // Mid-section, or a boundary in the middle of the outline: no pull.
    expect(gravityTarget(1250, height, boundaries)).toBeUndefined();
    expect(gravityTarget(750, height, boundaries)).toBeUndefined();
    // Already resting on a boundary.
    expect(gravityTarget(1000, height, boundaries)).toBeUndefined();
    // A short section: its start near the top beats the next start near the bottom.
    expect(gravityTarget(940, height, [1000, 1400])).toBe(1000);
  });
});
