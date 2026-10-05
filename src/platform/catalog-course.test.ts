import { describe, expect, it } from "vitest";
import { catalogCourse } from "./desktop-bridge";

const row = (periods: [number, number]) => ({
  id: "202600001",
  year: 2026,
  quarters: ["3Q"],
  code: "CSC.T363",
  title: { ja: "演習", en: "Exercise" },
  instructors: [],
  syllabusUrl: "https://syllabus.s.isct.ac.jp/courses/2026/4/0-904-342300-0-0/202600001",
  credits: { lecture: 0, exercise: 2, lab: 0 },
  instructionLanguage: "ja",
  schedule: [{ kind: "weekly", day: 2, periods }],
});

describe("catalog course meetings", () => {
  it("gives clock times to any period range, not only two-period slots", () => {
    expect(catalogCourse(row([5, 8]), "ja").meetings![0]).toMatchObject({ startTime: "13:30", endTime: "17:05" });
    expect(catalogCourse(row([3, 4]), "ja").meetings![0]).toMatchObject({ startTime: "10:45", endTime: "12:25" });
    expect(catalogCourse(row([1, 10]), "ja").meetings![0]).toMatchObject({ startTime: "08:50", endTime: "18:55" });
  });
});
