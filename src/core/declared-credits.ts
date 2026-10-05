import type { TermId } from "./model";

/** A course as 教務Web's 科目から lists it (`registrations`, kind `declared`). */
export type DeclaredCourse = {
  kind?: string;
  offeringId?: string;
  quarters?: number[];
  effectiveCredits?: number;
  deletedAt?: string;
};

/**
 * Credits declared on 教務Web for one quarter, as 教務Web counts them. A
 * course running over several quarters counts its share in each. Undefined
 * when 教務Web has nothing for that year (the caller falls back).
 */
export function declaredTermCredits(rows: DeclaredCourse[], term: TermId): number | undefined {
  const year = term.slice(0, 4);
  const quarter = Number(term.slice(5, 6));
  const declared = rows.filter(
    (row) => row.kind === "declared" && !row.deletedAt && row.offeringId?.startsWith(year),
  );
  if (!declared.length) return undefined;
  const total = declared.reduce((sum, row) => {
    const quarters = row.quarters ?? [];
    return quarters.includes(quarter) ? sum + (row.effectiveCredits ?? 0) / quarters.length : sum;
  }, 0);
  return Math.round(total * 100) / 100;
}
