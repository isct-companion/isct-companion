import { describe, expect, it } from "vitest";
import { SCHOOL_TTL_MS, staleSources } from "./school-freshness";

const now = Date.parse("2026-10-03T12:00:00Z");
const ago = (ms: number) => new Date(now - ms).toISOString();

describe("school cache TTL", () => {
  it("treats a missing cache as stale", () => {
    expect(staleSources({}, undefined, now)).toEqual({ assignments: true, timetable: true, kyomu: true });
  });

  it("pulls assignments after half a day and reviews the timetable monthly", () => {
    const kyomu = { "kyomu:gpa": ago(60_000), "kyomu:registrations": ago(60_000), "kyomu:grades": ago(60_000), "kyomu:requirements": ago(60_000) };
    const runs = { "lms:full": ago(SCHOOL_TTL_MS.assignments - 1), ...kyomu };
    expect(staleSources(runs, ago(SCHOOL_TTL_MS.timetable - 1), now)).toEqual({
      assignments: false,
      timetable: false,
      kyomu: false,
    });
    const later = { "lms:full": ago(SCHOOL_TTL_MS.assignments + 1), "kyomu:gpa": ago(60_000) };
    expect(staleSources(later, ago(SCHOOL_TTL_MS.timetable + 1), now)).toMatchObject({
      assignments: true,
      timetable: true,
    });
  });

  it("lets the oldest 教務Web page decide, and a never-read page counts as stale", () => {
    const fresh = { "kyomu:gpa": ago(0), "kyomu:registrations": ago(0), "kyomu:grades": ago(0), "kyomu:requirements": ago(0) };
    expect(staleSources(fresh, ago(0), now).kyomu).toBe(false);
    expect(staleSources({ ...fresh, "kyomu:grades": ago(SCHOOL_TTL_MS.kyomu + 1) }, ago(0), now).kyomu).toBe(true);
    const { "kyomu:requirements": _missing, ...partial } = fresh;
    expect(staleSources(partial, ago(0), now).kyomu).toBe(true);
  });
});
