import { DATA_POLICY } from "../generated/persistence";

/**
 * School data is cached in SQLite and pulled again only when it is older
 * than its TTL. Assignments change daily; the timetable rarely does.
 */
export const SCHOOL_TTL_MS = {
  /** LMS assignments and materials. */
  assignments: DATA_POLICY.lms.staleAfterSeconds * 1000,
  /** Comparing the timetable with LMS (and reviewing differences). */
  timetable: DATA_POLICY.timetable.staleAfterSeconds * 1000,
  /** 教務Web: GPA, registration state, grades, requirements. */
  kyomu: DATA_POLICY.kyomu.staleAfterSeconds * 1000,
} as const;

/** The 教務Web page kinds one pull reads (`import_runs.scope`). */
export const KYOMU_PAGES = ["gpa", "registrations", "declared", "grades", "requirements"] as const;

export type StaleSources = {
  assignments: boolean;
  timetable: boolean;
  kyomu: boolean;
};

const age = (iso: string | undefined, now: number) => {
  const at = iso ? Date.parse(iso) : NaN;
  return Number.isFinite(at) ? now - at : Infinity;
};

/**
 * Which caches are due. `lastRuns` holds the last complete import per
 * `provider:scope` (from `import_runs`); the timetable counts from its last
 * review, not from the last pull.
 */
export function staleSources(
  lastRuns: Record<string, string>,
  timetableReviewedAt: string | undefined,
  now = Date.now(),
): StaleSources {
  // Every 教務Web page must have succeeded once; the oldest one decides.
  const kyomuPages = KYOMU_PAGES.map((page) => lastRuns[`kyomu:${page}`]);
  const kyomu = kyomuPages.includes(undefined as never)
    ? undefined
    : (kyomuPages as string[]).sort()[0];
  return {
    assignments: age(lastRuns["lms:full"], now) > SCHOOL_TTL_MS.assignments,
    timetable: age(timetableReviewedAt, now) > SCHOOL_TTL_MS.timetable,
    kyomu: !kyomu || age(kyomu, now) > SCHOOL_TTL_MS.kyomu,
  };
}
