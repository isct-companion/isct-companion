import { describe, expect, it } from "vitest";

import {
  clashingCourses,
  condition,
  localFactors,
  remotePlan,
  factorsOf,
  liveText,
  normalizedKeys,
  run,
  suggest,
  type FinderContext,
  type FinderCourse,
} from "./course-query";

const course = (
  code: string,
  ja: string,
  en: string,
  schedule: FinderCourse["schedule"],
  extra: Partial<FinderCourse> = {},
): FinderCourse => ({
  id: `${code}-id`,
  code,
  year: 2025,
  quarters: ["3Q"],
  title: { ja, en },
  instructors: [{ ja: "山田 太郎", en: "Taro Yamada" }],
  credits: { lecture: 2, exercise: 0, lab: 0 },
  instructionLanguage: "ja",
  format: { kinds: ["lecture"] },
  schedule,
  unitCode: "342300",
  syllabusUrl: `https://syllabus.s.isct.ac.jp/courses/2025/4/x/${code}`,
  ...extra,
});
const weekly = (day: number, a: number, b: number) => ({ kind: "weekly", day, periods: [a, b] as [number, number] });

const COURSES = [
  course("CSC.T262", "アセンブリ言語", "Assembly Language", [weekly(2, 3, 4), weekly(5, 3, 4)]),
  course("CSC.T341", "コンピュータ論理設計", "Computer Logic Design", [weekly(1, 3, 4), weekly(4, 3, 4)], { format: { kinds: ["lecture", "exercise"] } }),
  course("MCS.T223", "数理統計学", "Mathematical Statistics", [weekly(2, 3, 4)], { unitCode: "342200", format: { kinds: ["exercise"] } }),
  course("CSC.Z381", "研究プロジェクト", "Research Project", [{ kind: "intensive" }], { quarters: ["3Q", "4Q"] }),
  course("CSC.T377", "情報工学英語プレゼンテーション", "English Presentation", [weekly(3, 1, 2)], { instructionLanguage: "en", quarters: ["4Q"] }),
];
const ctx: FinderContext = {
  currentQuarter: 3,
  recommended: new Map([
    ["CSC.T262", { group: "A", mark: "recommended" }],
    ["CSC.T341", { group: "A", mark: "recommended" }],
    ["CSC.Z381", { group: "R", mark: "required" }],
  ]),
  departments: [
    { id: "cs", name: { ja: "情報工学系", en: "Computer Science" }, unitCode: "342300" },
    { id: "mcs", name: { ja: "数理・計算科学系", en: "Mathematical and Computing Science" }, unitCode: "342200" },
  ],
  busy: [{ day: 1, from: 3, to: 4 }],
};
const codes = (keys: string[]) => run(COURSES, factorsOf(keys, ctx).factors, ctx).map((r) => r.course.code);

describe("course query", () => {
  it("reads many spellings into one canonical condition", () => {
    expect(condition("tue3-4", ctx).key).toBe("火3-4");
    expect(condition("火曜", ctx).key).toBe("火曜");
    expect(condition("rec", ctx).key).toBe("推荐");
    expect(condition("csc.t3", ctx)).toMatchObject({ type: "code", key: "CSC.T3", prefix: true });
    expect(condition("rec", ctx).type).toBe("mark");
    expect(condition("cs", ctx)).toMatchObject({ type: "dept", key: "情報工学系" });
    expect(condition("!集中", ctx)).toMatchObject({ exclude: true, key: "!集中" });
    expect(condition("オートマ", ctx).type).toBe("text");
  });

  it("treats the same kind as or, different kinds as and, and drops duplicates", () => {
    expect(codes(["curr", "火3-4", "金3-4"])).toEqual(["CSC.T262", "MCS.T223"]);
    expect(codes(["curr", "火3-4", "推荐"])).toEqual(["CSC.T262"]);
    const parsed = factorsOf(["推荐", "rec", "推荐"], ctx);
    expect(parsed.duplicates).toBe(2);
    expect(normalizedKeys(["rec", "curr", "火3-4", "rec"], ctx)).toEqual(["curr", "火3-4", "推荐"]);
  });

  it("excludes with ! and filters local-only conditions", () => {
    expect(codes(["curr", "!集中"])).not.toContain("CSC.Z381");
    expect(codes(["curr", "必修"])).toEqual(["CSC.Z381"]);
    expect(codes(["curr", "空き"])).not.toContain("CSC.T341"); // Mon 3-4 is taken
    expect(codes(["数理・計算科学系"])).toEqual(["MCS.T223"]);
    expect(codes(["英語"])).toEqual(["CSC.T377"]);
  });

  it("reports which alternative matched when a condition has several", () => {
    const rows = run(COURSES, factorsOf(["火3-4", "月3-4"], ctx).factors, ctx);
    expect(rows.find((r) => r.course.code === "CSC.T341")?.matched).toEqual(["月3-4"]);
  });

  it("suggests conditions, courses and a text fallback with counts", () => {
    const list = suggest("火", ["curr"], COURSES, ctx);
    expect(list[0]).toMatchObject({ kind: "condition", key: "火曜" });
    expect(list.some((s) => s.kind === "condition" && s.key === "火3-4" && s.count === 2)).toBe(true);
    expect(suggest("金3", [], COURSES, ctx)[0]).toMatchObject({ kind: "condition", key: "金3-4" });
    expect(suggest("アセン", [], COURSES, ctx).some((s) => s.kind === "course" && s.course.code === "CSC.T262")).toBe(true);
    expect(suggest("!集", [], COURSES, ctx)[0]).toMatchObject({ kind: "condition", key: "!集中" });
    expect(suggest("", ["curr"], COURSES, ctx).map((s) => s.kind === "condition" && s.key)).not.toContain("curr");
  });

  it("offers recommendation conditions only when that optional data exists", () => {
    const bare = { ...ctx, recommended: new Map() };
    const keysOf = (list: ReturnType<typeof suggest>) => list.map((s) => s.kind === "condition" && s.key);
    expect(keysOf(suggest("", [], COURSES, bare))).not.toContain("推荐");
    expect(keysOf(suggest("rec", [], COURSES, bare))).not.toContain("推荐");
    expect(keysOf(suggest("", [], COURSES, ctx))).toContain("推荐");
  });

  it("recognises Saturday but not Sunday", () => {
    expect(condition("sat3-4", ctx).key).toBe("土3-4");
    expect(condition("土曜", ctx).key).toBe("土曜");
    expect(condition("sun", ctx).type).toBe("text");
  });

  it("sends only what the syllabus understands and filters the rest locally", () => {
    const scopeOf = (unit: string) => (unit === "342300" ? "4/0-904-342300-0-0" : unit === "342200" ? "4/0-904-342200-0-0" : undefined);
    const plan = (keys: string[]) => remotePlan(factorsOf(keys, ctx).factors, scopeOf);
    expect(plan(["curr", "情報工学系", "火3-4"])).toEqual({ queries: [{ q: ".", scope: "4/0-904-342300-0-0" }] });
    expect(plan(["アセンブリ", "300番台"])).toEqual({ queries: [{ q: "アセンブリ", groups: ["300"] }] });
    expect(plan(["cs", "mcs", "CSC"])).toMatchObject({ queries: [{ code: "CSC" }, { code: "CSC" }] });
    expect(plan(["curr", "火3-4"])).toEqual({ unsupported: "empty" });
    expect(plan(["cs", "mcs", "CSC", "MCS", "LAH"])).toEqual({ unsupported: "too-many" });
    const local = localFactors(factorsOf(["curr", "cs", "火3-4", "!集中", "アセンブリ"], ctx).factors);
    expect(local.map((f) => f.type)).toEqual(["quarter", "time", "exclude"]);
  });

  it("finds the placed courses a result would replace", () => {
    const placed = [
      { code: "X.1", meetings: [{ days: [2], periods: [3, 4] }] },
      { code: "X.2", meetings: [{ days: [2], periods: [5, 6] }] },
      { code: "X.3", meetings: [{ days: [], periods: [] }] },
      { code: "CSC.T262", meetings: [{ days: [2], periods: [3, 4] }] },
    ];
    expect(clashingCourses(COURSES[0], placed).map((p) => p.code)).toEqual(["X.1"]);
    expect(clashingCourses(COURSES[3], placed)).toEqual([]);
  });

  it("uses typed text as a live filter only when it is not a condition", () => {
    expect(liveText("火", [], COURSES, ctx)).toBe("");
    expect(liveText("アセンブリ", [], COURSES, ctx)).toBe("アセンブリ");
  });
});
